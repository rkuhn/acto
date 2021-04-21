//! Light-weight actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel (even for the SPSC API), should probably use feature flags, and still is quite slow.
//!
//! Please check out [the example](https://github.com/Actyx/actor/blob/master/examples/pingpong.rs)

use std::{future::Future, marker::PhantomData, sync::Arc};

mod actor_id;
mod mailbox;
mod mpsc;
mod pod;
mod sender;
pub mod spawn;
mod spsc;
mod supervisor;

pub mod actor;

pub use actor_id::ActorId;
pub use mailbox::{Mailbox, SenderGone};
pub use mpsc::MPSC;
pub use pod::{MpQueue, Queue};
pub use sender::{DynSender, Sender};
pub use spsc::SPSC;
pub use supervisor::Supervisor;

pub struct Actor<RT, A, Fut, Msg, S>(
    PhantomData<RT>,
    PhantomData<A>,
    PhantomData<Fut>,
    PhantomData<Msg>,
    PhantomData<S>,
);

impl<RT, A, Fut, Msg, S> Actor<RT, A, Fut, Msg, S>
where
    RT: spawn::Spawn,
    Msg: Send,
    Fut: Future + Send + 'static,
    S: Supervisor<Fut::Output>,
{
    pub fn spawn<Q>(rt: &RT, actor: A, supervisor: S) -> Sender<Q>
    where
        A: FnOnce(Mailbox<Q>) -> Fut,
        Q: Queue<Msg = Msg>,
    {
        let pod = Arc::new(pod::Pod::new());
        let fut = actor(Mailbox::new(pod.clone()));
        let fut = async {
            let result = fut.await;
            supervisor.notify(result);
        };
        rt.spawn(fut);
        Sender::new(pod)
    }
}
