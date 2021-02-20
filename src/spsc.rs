use crate::pod::Queue;
use std::{
    cell::UnsafeCell,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{channel, Receiver, Sender};

pub struct SPSC<T> {
    rx: UnsafeCell<Receiver<T>>,
    tx: UnsafeCell<Sender<T>>,
}

impl<T> Default for SPSC<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Queue for SPSC<T> {
    type Msg = T;

    fn new() -> Self {
        let (tx, rx) = channel(100);
        Self {
            rx: rx.into(),
            tx: tx.into(),
        }
    }

    fn poll(&self, context: &mut Context<'_>) -> Poll<Option<Self::Msg>> {
        let rx = unsafe { &mut *self.rx.get() };
        rx.poll_recv(context)
    }

    fn enqueue_mut(&self, message: T) -> bool {
        let tx = unsafe { &mut *self.tx.get() };
        tx.try_send(message).is_ok()
    }
}
