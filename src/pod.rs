use std::task::{Context, Poll};

use crate::ActorId;

pub trait Queue {
    type Msg;

    fn new() -> Self;

    // poll can only be called from one thread at a time, but adding `&mut` here
    // allows the implementation to assume exclusive access, which is usually not
    // the case due to the enqueue operation => force impl to use unsafe
    fn poll(&self, context: &mut Context<'_>) -> Poll<Option<Self::Msg>>;

    fn enqueue_mut(&self, msg: Self::Msg) -> bool;
}

pub trait MpQueue: Queue {
    fn enqueue(&self, msg: Self::Msg) -> bool;
}

pub(crate) struct Pod<T> {
    pub(crate) id: ActorId,
    pub(crate) queue: T,
}
unsafe impl<T: Send> Sync for Pod<T> {}

impl<T: Queue> Pod<T> {
    pub(crate) fn new() -> Self {
        Self {
            id: ActorId::create(),
            queue: T::new(),
        }
    }
}
