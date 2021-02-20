use crate::pod::{MpQueue, Queue};
use std::{
    cell::UnsafeCell,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{channel, Receiver, Sender};

pub struct MPSC<T> {
    rx: UnsafeCell<Receiver<T>>,
    tx: Sender<T>,
}

impl<T> Default for MPSC<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> MpQueue for MPSC<T> {
    fn enqueue(&self, message: T) -> bool {
        self.tx.try_send(message).is_ok()
    }
}

impl<T> Queue for MPSC<T> {
    type Msg = T;

    fn new() -> Self {
        let (tx, rx) = channel(100);
        Self { rx: rx.into(), tx }
    }

    fn poll(&self, context: &mut Context<'_>) -> Poll<Option<Self::Msg>> {
        let rx = unsafe { &mut *self.rx.get() };
        rx.poll_recv(context)
    }

    fn enqueue_mut(&self, message: T) -> bool {
        self.enqueue(message)
    }
}
