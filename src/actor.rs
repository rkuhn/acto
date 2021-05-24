use ::tokio::sync::oneshot;
use anyhow::Result;
use derive_more::{Display, Error};
use parking_lot::Mutex;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Poll, Waker},
};

/// An ActorRef is the sending side of the actor’s mailbox, it can be freely cloned.
///
/// When the last `ActorRef` for an actor has been dropped, only the actor itself can
/// fabricate a new one. And if the actor is waiting for new messages from its mailbox
/// before a new `ActorRef` is created, the actor would be stuck in this state forever.
/// Therefore, the actor’s `receive()` function returns an error in this case, which
/// will usually lead to the actor shutting down.
pub struct ActorRef<M>(Arc<ActorRefInner<M>>);

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        self.0.count.fetch_add(1, Ordering::SeqCst);
        Self(self.0.clone())
    }
}

impl<M> Drop for ActorRef<M> {
    fn drop(&mut self) {
        let prev = self.0.count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // we’re the last external reference, so wake up the Actor if it is waiting for us
            if let Some(waker) = self.0.waker.lock().take() {
                waker.wake();
            }
        }
    }
}

impl<M> ActorRef<M> {
    pub fn new(tell: Box<dyn Fn(M) + Send + Sync + 'static>) -> Self {
        Self(Arc::new(ActorRefInner {
            count: AtomicUsize::new(0),
            waker: Mutex::new(None),
            tell,
        }))
    }

    pub fn tell(&self, msg: M) {
        (self.0.tell)(msg);
    }
}

struct ActorRefInner<M> {
    count: AtomicUsize,
    waker: Mutex<Option<Waker>>,
    tell: Box<dyn Fn(M) + Send + Sync + 'static>, // TODO get rid of the box (requires unsafe)
}

/// The context in which an actor is running
///
/// This context allows the actor to
///
/// - receive messages
/// - get its own address (i.e. [`ActorRef`](struct.ActorRef.html))
/// - spawn new actors
///
/// The definition of child actors is best donw using the [`actor`](macro.actor.html) macro.
pub struct Context<M> {
    recv: Box<dyn Receiver<M>>,
    aref: ActorRef<M>,
    spawner: Arc<dyn Spawner>,
}

impl<M: Send + 'static> Context<M> {
    pub fn new(mailbox: impl Mailbox, spawner: Arc<dyn Spawner>) -> Self {
        let (aref, recv) = mailbox.make_mailbox();
        Self {
            recv,
            aref,
            spawner,
        }
    }

    pub fn inherit<N: Send + 'static, MB: Mailbox>(&self, mailbox: MB) -> Context<N> {
        let (aref, recv) = mailbox.make_mailbox();
        Context {
            recv,
            aref,
            spawner: self.spawner.clone(),
        }
    }

    /// Receive the next message from the mailbox, possibly waiting for it
    ///
    /// This method’s return value should always be immediately `.await`ed, it has no other use.
    pub fn receive(&mut self) -> ReceiveFuture<'_, M> {
        ReceiveFuture {
            aref: &self.aref.0,
            fut: self.recv.receive(),
        }
    }

    /// The actors own address a.k.a. ActorRef
    ///
    /// The actor can put this `ActorRef` into messages to send it to other actors so that they
    /// can contact this actor back later.
    pub fn me(&self) -> ActorRef<M> {
        self.aref.clone()
    }

    /// Spawn a future for the purpose of running a child actor
    ///
    /// This method is best used via the [`actor`](macro.actor.html) macro.
    pub fn spawn(
        &self,
        fut: Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>,
    ) -> oneshot::Receiver<()> {
        self.spawner.spawn(fut)
    }
}

pub struct ReceiveFuture<'a, M: Send + 'static> {
    aref: &'a ActorRefInner<M>,
    fut: &'a mut (dyn Future<Output = Result<M>> + Send + Unpin + 'a),
}

impl<'a, M: Send + 'static> Future for ReceiveFuture<'a, M> {
    type Output = Result<M>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut *self.fut).poll(cx) {
            Poll::Ready(x) => Poll::Ready(x),
            Poll::Pending => {
                if self.aref.count.load(Ordering::SeqCst) == 0 {
                    // observing no external address means that there cannot be one created, either:
                    // we hold an exclusive reference on the Context (via self.fut)
                    Poll::Ready(Err(NoActorRef.into()))
                } else {
                    *self.aref.waker.lock() = Some(cx.waker().clone());
                    // in case the last ActorRef was dropped between the check and installing the waker,
                    // we must now re-check
                    if self.aref.count.load(Ordering::SeqCst) == 0 {
                        Poll::Ready(Err(NoActorRef.into()))
                    } else {
                        Poll::Pending
                    }
                }
            }
        }
    }
}

#[derive(Debug, Display, Error)]
#[display(fmt = "cannot receive: no external ActorRef for this actor")]
pub struct NoActorRef;

pub trait Receiver<M: Send + 'static>: Send {
    // this trait is only necessary because FnMut doesn’t make its argument’s self reference lifetime available
    fn receive(&mut self) -> &mut (dyn Future<Output = Result<M>> + Send + Unpin + '_);
}

/// Factory for mailboxes, which usually are MPSC queues under the hood
pub trait Mailbox {
    fn make_mailbox<M: Send + 'static>(&self) -> (ActorRef<M>, Box<dyn Receiver<M>>);
}

/// Facility for spawning a particular kind of Future that is used to run actors
pub trait Spawner: Send + Sync + 'static {
    fn spawn(
        &self,
        fut: Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>,
    ) -> oneshot::Receiver<()>;
}

pub mod tokio {
    use ::tokio::sync::oneshot;
    use futures::Future;
    use std::{pin::Pin, task::Poll};
    use tokio::sync::mpsc;

    #[derive(Clone, Debug)]
    pub struct Spawner;

    impl super::Spawner for Spawner {
        fn spawn(
            &self,
            fut: Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'static>>,
        ) -> oneshot::Receiver<()> {
            let (tx, rx) = oneshot::channel();
            tokio::spawn(async move {
                let _ = fut.await;
                let _ = tx.send(());
            });
            rx
        }
    }

    pub struct Mailbox;

    impl super::Mailbox for Mailbox {
        fn make_mailbox<M: Send + 'static>(
            &self,
        ) -> (super::ActorRef<M>, Box<dyn super::Receiver<M>>) {
            let (tx, rx) = mpsc::unbounded_channel::<M>();
            let aref = super::ActorRef::new(Box::new(move |msg| {
                let _ = tx.send(msg);
            }));
            (aref, Box::new(Receiver(rx)))
        }
    }

    pub struct Receiver<M>(mpsc::UnboundedReceiver<M>);

    impl<M: Send + 'static> super::Receiver<M> for Receiver<M> {
        fn receive(&mut self) -> &mut (dyn Future<Output = anyhow::Result<M>> + Send + Unpin + '_) {
            self
        }
    }

    impl<M: Send + 'static> Future for Receiver<M> {
        type Output = anyhow::Result<M>;

        fn poll(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            match self.as_mut().0.poll_recv(cx) {
                Poll::Ready(Some(msg)) => Poll::Ready(Ok(msg)),
                Poll::Ready(None) => Poll::Ready(Err(anyhow::anyhow!("channel closed"))),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

/// Define and spawn a new Actor
///
/// The first form allows spawning a top-level actor defined in an `async fn`:
///
/// ```
/// use acto::{actor, Context, Result, TokioMailbox, TokioSpawner};
///
/// async fn my_actor(mut ctx: Context<String>, me: String) -> Result<()> {
///     loop {
///         let msg = ctx.receive().await?;
///         println!("{} got msg: {}", me, msg);
///     }
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let (aref, join_handle) = actor!(TokioMailbox, TokioSpawner, fn my_actor(ctx, "Fred".to_owned()));
/// }
/// ```
///
/// The second form allows spawning a top-level actor defined inline:
///
/// ```
/// use acto::{actor, Context, TokioMailbox, TokioSpawner};
///
/// #[tokio::main]
/// async fn main() {
///     let me = "Barney".to_owned();
///     let (aref, join_handle) = actor!(TokioMailbox, TokioSpawner, |ctx| {
///         loop {
///             let msg: String = ctx.receive().await?;
///             println!("{} got msg: {}", me, msg);
///         }
///     });
/// }
/// ```
///
/// The third form is for spawning actors within other actors. The `ctx` variable can be named differently,
/// but it must be the name of the enclosing actor’s `Context`. It will intentionally shadow this name within
/// the child actor, to make it obvious that the parent’s `Context` cannot be used there.
///
/// ```
/// use acto::{actor, Context, Result, TokioMailbox, TokioSpawner};
///
/// async fn my_actor(mut ctx: Context<String>, me: String) -> Result<()> {
///     loop {
///         let msg = ctx.receive().await?;
///         let (aref, _join_handle) = actor!(TokioMailbox, |ctx| {
///             let msg = ctx.receive().await?;
///             println!("servant: {}", msg);
///             // just to demonstrate that when the external ActorRef is dropped, this stops
///             let _ = ctx.receive().await?;
///             println!("servant done");
///         });
///         aref.tell(format!("{} got msg: {}", me, msg));
///     }
/// }
/// ```
#[macro_export]
macro_rules! actor {
    ($mailbox:path, $spawner:path, fn $f:ident($ctx:ident$(,$arg:expr)*)) => {{
        use $crate::Spawner;
        let _spawner = ::std::sync::Arc::new($spawner);
        let $ctx = $crate::Context::new($mailbox, _spawner.clone());
        let _aref = $ctx.me();
        let fut = Box::pin($f($ctx, $($arg),*));
        (_aref, _spawner.spawn(fut))
    }};
    ($mailbox:path, $spawner:path, |$ctx:ident| $code:block) => {{
        use $crate::Spawner;
        let _spawner = ::std::sync::Arc::new($spawner);
        let mut $ctx = $crate::Context::new($mailbox, _spawner.clone());
        let _aref = $ctx.me();
        let fut = Box::pin(async move {
            $code
            Ok(())
        });
        (_aref, _spawner.spawn(fut))
    }};
    ($mailbox:path, |$ctx:ident| $code:block) => {{
        let (fut, aref) = {
            let mut $ctx = $ctx.inherit($mailbox);
            let _aref = $ctx.me();
            let fut = Box::pin(async move {
                $code
                Ok(())
            });
            (fut, _aref)
        };
        (aref, $ctx.spawn(fut))
    }}
}

#[cfg(test)]
mod tests {
    use super::{
        tokio::{Mailbox, Spawner},
        ActorRef, Context, NoActorRef,
    };
    use anyhow::Result;
    use futures::poll;
    use std::{task::Poll, thread::sleep, time::Duration};
    use tokio::sync::oneshot;

    async fn actor(mut ctx: Context<(String, ActorRef<String>)>) -> Result<()> {
        loop {
            let (name, sender) = ctx.receive().await?;
            let (responder, handle) = actor!(Mailbox, |ctx| {
                let m = ctx.receive().await?;
                sender.tell(format!("Hello {}!", m));
            });
            responder.tell(name);
            let _ = handle.await;
        }
    }

    #[tokio::test]
    async fn smoke() {
        let (aref, join_handle) = actor!(Mailbox, Spawner, fn actor(ctx));

        let (tx, rx) = oneshot::channel();
        let (receiver, jr) = actor!(Mailbox, Spawner, |ctx| {
            let msg = ctx.receive().await?;
            let _ = tx.send(msg);
        });
        aref.tell(("Fred".to_owned(), receiver));
        assert_eq!(rx.await.unwrap(), "Hello Fred!");
        jr.await.unwrap();

        let (tx, rx) = oneshot::channel();
        let (receiver, jr) = actor!(Mailbox, Spawner, |ctx| {
            let msg = ctx.receive().await?;
            let _ = tx.send(msg);
        });
        aref.tell(("Barney".to_owned(), receiver));
        assert_eq!(rx.await.unwrap(), "Hello Barney!");
        jr.await.unwrap();

        drop(aref);
        join_handle.await.unwrap();
    }

    #[tokio::test]
    async fn dropped() {
        let (tx, mut rx) = oneshot::channel();
        let (aref, handle) = actor!(Mailbox, Spawner, |ctx| {
            let result: Result<()> = ctx.receive().await;
            let _ = tx.send(result);
        });

        sleep(Duration::from_millis(200));
        match poll!(&mut rx) {
            Poll::Pending => {}
            x => panic!("unexpected result: {:?}", x),
        }

        drop(aref);
        handle.await.unwrap();
        let err = match poll!(rx) {
            Poll::Ready(Ok(e)) => e.unwrap_err(),
            x => panic!("unexpected poll result: {:?}", x),
        };
        err.downcast::<NoActorRef>()
            .unwrap_or_else(|e| panic!("unexpected error type: {}", e));
    }
}
