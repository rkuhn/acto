use std::{future::Future, sync::Arc};

mod actor_id;
mod mailbox;
mod mpsc;
mod pod;
mod sender;
mod spsc;
mod supervisor;

pub use actor_id::ActorId;
pub use mailbox::{Mailbox, SenderGone};
pub use mpsc::MPSC;
pub use sender::{AnySender, MultiSender, SingleSender};
pub use spsc::SPSC;
pub use supervisor::Supervisor;

use pod::Pod;

pub trait Spawn {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}
impl Spawn for tokio::runtime::Runtime {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn(f);
    }
}
impl Spawn for tokio::runtime::Handle {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn(f);
    }
}

pub fn spawn_actor<RT, A, Fut, Msg, S>(rt: &RT, actor: A, supervisor: S) -> MultiSender<Msg>
where
    RT: Spawn,
    Msg: Send,
    A: FnOnce(Mailbox<MPSC<Msg>>) -> Fut,
    Fut: Future + Send + 'static,
    S: Supervisor<Fut::Output>,
{
    let pod = Arc::new(Pod::new());
    let fut = actor(Mailbox::new(pod.clone()));
    let fut = async {
        let result = fut.await;
        supervisor.notify(result);
    };
    rt.spawn(fut);
    MultiSender { pod }
}

pub fn spawn_actor_spsc<RT, A, Fut, Msg, S>(rt: &RT, actor: A, supervisor: S) -> SingleSender<Msg>
where
    RT: Spawn,
    Msg: Send,
    A: FnOnce(Mailbox<SPSC<Msg>>) -> Fut,
    Fut: Future + Send + 'static,
    S: Supervisor<Fut::Output>,
{
    let pod = Arc::new(Pod::new());
    let fut = actor(Mailbox::new(pod.clone()));
    let fut = async {
        let result = fut.await;
        supervisor.notify(result);
    };
    rt.spawn(fut);
    SingleSender { pod }
}
