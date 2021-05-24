//! Primitive spawner that creates a new thread for each actor

use crate::{FutureBox, FutureResultBox, Mailbox, Receiver, Spawner};
use futures::{
    channel::{
        mpsc::{unbounded, UnboundedReceiver},
        oneshot,
    },
    executor::block_on,
    StreamExt,
};
use std::{future::Future, pin::Pin, task::Poll};

/// Spawner that creates a new thread for each actor
///
/// The threads are named according to the given string.
pub struct ThreadSpawner(String);

impl ThreadSpawner {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl Spawner for ThreadSpawner {
    fn spawn(&self, fut: FutureBox) -> FutureResultBox {
        let (tx, rx) = oneshot::channel();
        std::thread::Builder::new()
            .name(self.0.clone())
            .spawn(move || {
                let result = block_on(fut);
                let _ = tx.send(result);
            })
            .unwrap();
        Box::pin(async move {
            let result = rx.await?;
            Ok(result)
        })
    }
}

pub struct FuturesMailbox;

impl Mailbox for FuturesMailbox {
    fn make_mailbox<M: Send + 'static>(&self) -> (super::ActorRef<M>, Box<dyn Receiver<M>>) {
        let (tx, rx) = unbounded::<M>();
        let aref = super::ActorRef::new(Box::new(move |msg| {
            let _ = tx.unbounded_send(msg);
        }));
        (aref, Box::new(FuturesReceiver(rx)))
    }
}

pub struct FuturesReceiver<M>(UnboundedReceiver<M>);

impl<M: Send + 'static> super::Receiver<M> for FuturesReceiver<M> {
    fn receive(&mut self) -> &mut (dyn Future<Output = anyhow::Result<M>> + Send + Unpin + '_) {
        self
    }
}

impl<M: Send + 'static> Future for FuturesReceiver<M> {
    type Output = anyhow::Result<M>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.as_mut().0.poll_next_unpin(cx) {
            Poll::Ready(Some(msg)) => Poll::Ready(Ok(msg)),
            Poll::Ready(None) => Poll::Ready(Err(anyhow::anyhow!("channel closed"))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActorRef, Context, NoActorRef};
    use anyhow::Result;
    use futures::{channel::oneshot, poll};
    use std::{task::Poll, thread::sleep, time::Duration};

    async fn actor(mut ctx: Context<(String, ActorRef<String>)>) -> Result<()> {
        loop {
            let (name, sender) = ctx.receive().await?;
            let (responder, handle) = actor!(FuturesMailbox, |ctx| {
                let m = ctx.receive().await?;
                sender.tell(format!("Hello {}!", m));
                Ok(())
            });
            responder.tell(name);
            let _ = handle.await;
        }
    }

    #[test]
    fn smoke() {
        block_on(async {
            let (aref, join_handle) =
                actor!(FuturesMailbox, ThreadSpawner::new("a"), fn actor(ctx));

            let (tx, rx) = oneshot::channel();
            let (receiver, jr) = actor!(FuturesMailbox, ThreadSpawner::new("a"), |ctx| {
                let msg = ctx.receive().await?;
                let _ = tx.send(msg);
                Ok("buh")
            });
            aref.tell(("Fred".to_owned(), receiver));
            assert_eq!(rx.await.unwrap(), "Hello Fred!");
            assert_eq!(jr.await.unwrap().unwrap(), "buh");

            let (tx, rx) = oneshot::channel();
            let (receiver, jr) = actor!(FuturesMailbox, ThreadSpawner::new("a"), |ctx| {
                let msg = ctx.receive().await?;
                let _ = tx.send(msg);
                Ok(42)
            });
            aref.tell(("Barney".to_owned(), receiver));
            assert_eq!(rx.await.unwrap(), "Hello Barney!");
            assert_eq!(jr.await.unwrap().unwrap(), 42);

            drop(aref);
            join_handle
                .await
                .unwrap()
                .unwrap_err()
                .downcast::<NoActorRef>()
                .unwrap();
        })
    }

    #[test]
    fn dropped() {
        block_on(async {
            let (tx, mut rx) = oneshot::channel();
            let (aref, handle) = actor!(FuturesMailbox, ThreadSpawner::new("a"), |ctx| {
                let result: Result<()> = ctx.receive().await;
                let _ = tx.send(result);
                Ok(())
            });

            sleep(Duration::from_millis(200));
            match poll!(&mut rx) {
                Poll::Pending => {}
                x => panic!("unexpected result: {:?}", x),
            }

            drop(aref);
            handle.await.unwrap().unwrap();
            let err = match poll!(rx) {
                Poll::Ready(Ok(e)) => e.unwrap_err(),
                x => panic!("unexpected poll result: {:?}", x),
            };
            err.downcast::<NoActorRef>()
                .unwrap_or_else(|e| panic!("unexpected error type: {}", e));
        })
    }
}
