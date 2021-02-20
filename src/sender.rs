use derive_more::From;
use std::sync::Arc;

use crate::{mpsc::MPSC, pod::Pod, spsc::SPSC, ActorId};

/// A handle to send to an actor bound to an MPSC mailbox.
///
/// It can be cloned or shared.
#[derive(Clone)]
pub struct MultiSender<T> {
    pub(crate) pod: Arc<Pod<MPSC<T>>>,
}

impl<T: Send + 'static> MultiSender<T> {
    pub fn send(&self, msg: T) -> bool {
        self.pod.queue.enqueue(msg)
    }

    pub fn id(&self) -> ActorId {
        self.pod.id
    }
}

/// A handle to send to an actor bound to an SPSC mailbox.
///
/// It cannot be shared nor cloned.
pub struct SingleSender<T> {
    pub(crate) pod: Arc<Pod<SPSC<T>>>,
}

impl<T: Send + 'static> SingleSender<T> {
    pub fn send(&mut self, msg: T) -> bool {
        self.pod.queue.enqueue(msg)
    }

    pub fn id(&self) -> ActorId {
        self.pod.id
    }
}

#[derive(From)]
pub enum AnySender<T> {
    Single(SingleSender<T>),
    Multi(MultiSender<T>),
}

impl<T: Send + 'static> AnySender<T> {
    pub fn send(&mut self, msg: T) -> bool {
        match self {
            AnySender::Single(s) => s.send(msg),
            AnySender::Multi(s) => s.send(msg),
        }
    }
}
