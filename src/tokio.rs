use crate::{Mailbox, Receiver, Spawner};
use std::{future::Future, pin::Pin, task::Poll};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug)]
pub struct TokioSpawner;

impl Spawner for TokioSpawner {
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

pub struct TokioMailbox;

impl Mailbox for TokioMailbox {
    fn make_mailbox<M: Send + 'static>(&self) -> (super::ActorRef<M>, Box<dyn Receiver<M>>) {
        let (tx, rx) = mpsc::unbounded_channel::<M>();
        let aref = super::ActorRef::new(Box::new(move |msg| {
            let _ = tx.send(msg);
        }));
        (aref, Box::new(TokioReceiver(rx)))
    }
}

pub struct TokioReceiver<M>(mpsc::UnboundedReceiver<M>);

impl<M: Send + 'static> super::Receiver<M> for TokioReceiver<M> {
    fn receive(&mut self) -> &mut (dyn Future<Output = anyhow::Result<M>> + Send + Unpin + '_) {
        self
    }
}

impl<M: Send + 'static> Future for TokioReceiver<M> {
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

#[cfg(test)]
mod tests {
    use crate::{
        tokio::{TokioMailbox, TokioSpawner},
        ActorRef, Context, NoActorRef,
    };
    use anyhow::Result;
    use futures::poll;
    use std::{task::Poll, thread::sleep, time::Duration};
    use tokio::sync::oneshot;

    async fn actor(mut ctx: Context<(String, ActorRef<String>)>) -> Result<()> {
        loop {
            let (name, sender) = ctx.receive().await?;
            let (responder, handle) = actor!(TokioMailbox, |ctx| {
                let m = ctx.receive().await?;
                sender.tell(format!("Hello {}!", m));
            });
            responder.tell(name);
            let _ = handle.await;
        }
    }

    #[tokio::test]
    async fn smoke() {
        let (aref, join_handle) = actor!(TokioMailbox, TokioSpawner, fn actor(ctx));

        let (tx, rx) = oneshot::channel();
        let (receiver, jr) = actor!(TokioMailbox, TokioSpawner, |ctx| {
            let msg = ctx.receive().await?;
            let _ = tx.send(msg);
        });
        aref.tell(("Fred".to_owned(), receiver));
        assert_eq!(rx.await.unwrap(), "Hello Fred!");
        jr.await.unwrap();

        let (tx, rx) = oneshot::channel();
        let (receiver, jr) = actor!(TokioMailbox, TokioSpawner, |ctx| {
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
        let (aref, handle) = actor!(TokioMailbox, TokioSpawner, |ctx| {
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
