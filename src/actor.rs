use ::tokio::sync::oneshot;
use anyhow::Result;
use std::{future::Future, pin::Pin, sync::Arc};

pub struct ActorRef<M>(Arc<dyn Fn(M) + Send + Sync + 'static>);

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> ActorRef<M> {
    pub fn new(f: Box<dyn Fn(M) + Send + Sync + 'static>) -> Self {
        Self(f.into())
    }

    pub fn tell(&self, msg: M) {
        (self.0)(msg);
    }
}

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

    pub async fn receive(&mut self) -> Result<M> {
        self.recv.receive().await
    }

    pub fn me(&self) -> ActorRef<M> {
        self.aref.clone()
    }

    pub fn spawn(
        &self,
        fut: Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>,
    ) -> oneshot::Receiver<()> {
        self.spawner.spawn(fut)
    }
}

pub trait Receiver<M: Send + 'static>: Send {
    fn receive(&mut self) -> &mut (dyn Future<Output = Result<M>> + Send + Unpin + '_);
}

pub trait Mailbox {
    fn make_mailbox<M: Send + 'static>(&self) -> (ActorRef<M>, Box<dyn Receiver<M>>);
}

pub trait Spawner: Send + Sync + 'static {
    fn spawn(
        &self,
        fut: Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>,
    ) -> oneshot::Receiver<()>;
}

mod tokio {
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

#[macro_export]
macro_rules! actor {
    ($mailbox:path, $spawner:path, fn $f:ident($ctx:ident$(,$arg:expr)*)) => {{
        let spawner = ::std::sync::Arc::new($spawner);
        let $ctx = $crate::actor2::Context::new($mailbox, spawner.clone());
        let aref = $ctx.me();
        let fut = Box::pin($f($ctx, $($arg),*));
        (aref, spawner.spawn(fut))
    }};
    ($mailbox:path, $spawner:path, |$ctx:ident| $code:block) => {{
        let spawner = ::std::sync::Arc::new($spawner);
        let mut $ctx = $crate::actor2::Context::new($mailbox, spawner.clone());
        let aref = $ctx.me();
        let fut = Box::pin(async move {
            $code
            Ok(())
        });
        (aref, spawner.spawn(fut))
    }};
    ($mailbox:path, |$ctx:ident| $code:block) => {{
        let (fut, aref) = {
            let mut $ctx = $ctx.inherit($mailbox);
            let aref = $ctx.me();
            let fut = Box::pin(async move {
                $code
                Ok(())
            });
            (fut, aref)
        };
        (aref, $ctx.spawn(fut))
    }}
}

#[cfg(test)]
mod tests {
    use super::{
        tokio::{Mailbox, Spawner},
        ActorRef, Context, Spawner as _,
    };
    use anyhow::Result;
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
        let (aref, _h) = actor!(Mailbox, Spawner, fn actor(ctx));

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
    }
}
