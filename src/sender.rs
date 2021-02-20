use derive_more::From;
use std::sync::Arc;

use crate::{
    pod::{MpQueue, Pod, Queue},
    ActorId, MPSC, SPSC,
};

pub struct Sender<T> {
    pod: Arc<Pod<T>>,
}

impl<T: Queue> Sender<T> {
    pub(crate) fn new(pod: Arc<Pod<T>>) -> Self {
        Self { pod }
    }

    pub fn id(&self) -> ActorId {
        self.pod.id
    }

    pub fn send_mut(&mut self, msg: T::Msg) -> bool {
        self.pod.queue.enqueue_mut(msg)
    }
}

impl<T: MpQueue> Sender<T> {
    pub fn send(&self, msg: T::Msg) -> bool {
        self.pod.queue.enqueue(msg)
    }
}

#[derive(From)]
pub enum DynSender<T> {
    Single(Sender<SPSC<T>>),
    Multi(Sender<MPSC<T>>),
}

impl<T> DynSender<T> {
    pub fn id(&self) -> ActorId {
        match self {
            DynSender::Single(s) => s.id(),
            DynSender::Multi(s) => s.id(),
        }
    }

    pub fn send_mut(&mut self, msg: T) -> bool {
        match self {
            DynSender::Single(s) => s.send_mut(msg),
            DynSender::Multi(s) => s.send_mut(msg),
        }
    }
}
