//! Totally non-working, wishful thinking sketch of actors
//! 
//! Making this work requires at least
//! - higher-kinded types
//! - generic associated types
//! - existential types

use futures::{stream::Next, Future};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub struct ActorRef<M>(Box<dyn Fn(M) + Send + 'static>);

impl<M> ActorRef<M> {
    pub fn tell(&self, msg: M) {
        (self.0)(msg);
    }
}

/// A Context is something that allows an actor to run, which includes the ability to
/// spawn more actors. So you can `spawn` method as a definition what an actor is.
/// The returned oneshot-receiver allows observing actor termination.
pub trait Context<M: Send + 'static> {
    fn me(&self) -> ActorRef<M>;
    fn receive(&mut self) -> (impl Future<Output = M> + Send + '_);
    fn spawn<N, A>(&mut self, actor: A) -> (ActorRef<N>, oneshot::Receiver<()>)
    where
        N: Send + 'static,
        A: for<'a> FnOnce(&'a mut impl Context<N>) -> (impl Future<Output = ()> + Send + 'a);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// demonstrates actor capabilities:
    /// 
    /// - receives messages
    /// - creates actors
    /// - sends messages
    /// - determines how to handle its next message
    async fn actor(ctx: &mut impl Context<(String, ActorRef<String>)>) {
        loop {
            let (name, sender) = ctx.receive().await;
            let (responder, handle) = ctx.spawn(async move |ctx| {
                let m = ctx.receive().await;
                sender.tell(format!("Hello {}!", m));
            });
            responder.tell(name);
            let _ = handle.await;
        }
    }
}
