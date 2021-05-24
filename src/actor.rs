use crate::{FutureBox, FutureResultBox};
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
    pub fn spawn<F>(&self, fut: F) -> impl Future<Output = Result<F::Output>> + Send + 'static
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        crate::spawn(&*self.spawner, fut)
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
    fn spawn(&self, fut: FutureBox) -> FutureResultBox;
}
