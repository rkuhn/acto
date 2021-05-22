use std::{future::Future, pin::Pin, task::Poll};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver},
    oneshot,
};

pub struct ActorRef<M>(Box<dyn Fn(M) + Send + 'static>);

impl<M> ActorRef<M> {
    pub fn new(f: Box<dyn Fn(M) + Send + 'static>) -> Self {
        Self(f)
    }

    pub fn tell(&self, msg: M) {
        (self.0)(msg);
    }
}

pub struct Context<'a, M, S: Spawner> {
    mk_ref: &'a (dyn Fn() -> ActorRef<M> + Send + Sync),
    mk_recv: &'a mut dyn MkReceive<M>,
    pub spawner: S,
}

impl<'a, M, S: Spawner> Context<'a, M, S> {
    pub fn new(
        mk_ref: &'a (dyn Fn() -> ActorRef<M> + Send + Sync),
        mk_recv: &'a mut dyn MkReceive<M>,
        spawner: S,
    ) -> Self {
        Self {
            mk_ref,
            mk_recv,
            spawner,
        }
    }

    pub fn me(&self) -> ActorRef<M> {
        (self.mk_ref)()
    }

    pub async fn receive(&mut self) -> M {
        self.mk_recv.make().await
    }

    pub fn spawn<N, A>(&mut self, actor: A) -> (ActorRef<N>, oneshot::Receiver<()>)
    where
        N: 'static + Send,
        A: for<'b> FnOnce(Context<'b, N, S>) -> Pin<Box<dyn Future<Output = ()> + Send + 'b>>
            + Send
            + 'static,
    {
        self.spawner.spawn(actor)
    }
}

pub trait MkReceive<M>: Send {
    fn make(&mut self) -> &mut (dyn Future<Output = M> + Send + Unpin);
}

pub trait Spawner: Sized + Send + 'static {
    fn spawn<N, A>(&mut self, actor: A) -> (ActorRef<N>, oneshot::Receiver<()>)
    where
        N: 'static + Send,
        A: for<'a> FnOnce(Context<'a, N, Self>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
            + Send
            + 'static;
}

struct TokioSpawner;

impl Spawner for TokioSpawner {
    fn spawn<N, A>(&mut self, actor: A) -> (ActorRef<N>, oneshot::Receiver<()>)
    where
        N: 'static + Send,
        // this Pin-Box is only needed because there is no other way to specify that the
        // Context’s lifetime shall match the Future’s lifetime (ugh)
        A: for<'a> FnOnce(Context<'a, N, Self>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
            + Send
            + 'static,
    {
        let (tx_join, rx_join) = oneshot::channel();
        let (tx, rx) = unbounded_channel::<N>();
        let mk_ref = move || {
            // problem: this keeps a sender around, so even when the last ActorRef has been
            // dropped the channel stays alive — this is sometimes okay, the actor may hand
            // out its self ref later again, but it makes it impossible to GC useless loop
            // actors.
            let tx = tx.clone();
            ActorRef::new(Box::new(move |msg| {
                let _ = tx.send(msg);
            }))
        };
        let me = mk_ref();
        tokio::spawn(async move {
            let mut mk_recv = TokioReceive(rx);
            let ctx = Context::new(&mk_ref, &mut mk_recv, TokioSpawner);
            actor(ctx).await;
            let _ = tx_join.send(());
        });
        (me, rx_join)
    }
}

struct TokioReceive<M>(UnboundedReceiver<M>);

impl<M: Send> MkReceive<M> for TokioReceive<M> {
    fn make(&mut self) -> &mut (dyn Future<Output = M> + Send + Unpin) {
        self
    }
}

impl<M: Send> Future for TokioReceive<M> {
    type Output = M;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.as_mut().0.poll_recv(cx) {
            Poll::Ready(None) => panic!("wat"),
            Poll::Ready(Some(msg)) => Poll::Ready(msg),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    async fn actor(mut ctx: Context<'_, (String, ActorRef<String>), impl Spawner>) {
        loop {
            let (name, sender) = ctx.receive().await;
            let (responder, handle) = ctx.spawn(move |mut ctx| {
                async move {
                    let m = ctx.receive().await;
                    sender.tell(format!("Hello {}!", m));
                }
                .boxed()
            });
            responder.tell(name);
            let _ = handle.await;
        }
    }

    #[tokio::test]
    async fn smoke() {
        let (aref, _) = TokioSpawner.spawn(|ctx| actor(ctx).boxed());

        let (tx, rx) = oneshot::channel();
        let (receiver, jr) = TokioSpawner.spawn::<String, _>(move |mut ctx| {
            async move {
                let msg = ctx.receive().await;
                let _ = tx.send(msg);
            }
            .boxed()
        });
        aref.tell(("Fred".to_owned(), receiver));
        assert_eq!(rx.await.unwrap(), "Hello Fred!");
        jr.await.unwrap();

        let (tx, rx) = oneshot::channel();
        let (receiver, jr) = TokioSpawner.spawn::<String, _>(move |mut ctx| {
            async move {
                let msg = ctx.receive().await;
                let _ = tx.send(msg);
            }
            .boxed()
        });
        aref.tell(("Barney".to_owned(), receiver));
        assert_eq!(rx.await.unwrap(), "Hello Barney!");
        jr.await.unwrap();
    }
}
