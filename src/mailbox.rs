use derive_more::{Display, Error};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use crate::{
    mpsc::MPSC,
    pod::{Pod, Queue},
    MultiSender,
};

#[derive(Debug, Display, Error)]
pub struct SenderGone;

pub struct Mailbox<T> {
    pod: Arc<Pod<T>>,
}

impl<T: Queue> Mailbox<T> {
    pub(crate) fn new(pod: Arc<Pod<T>>) -> Self {
        Self { pod }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> MailboxFuture<'_, T> {
        MailboxFuture { mb: self }
    }
}

impl<Msg> Mailbox<MPSC<Msg>> {
    pub fn me(&self) -> MultiSender<Msg> {
        MultiSender {
            pod: self.pod.clone(),
        }
    }
}

pub struct MailboxFuture<'a, T> {
    mb: &'a mut Mailbox<T>,
}

impl<T: Queue> Future for MailboxFuture<'_, T> {
    type Output = Result<T::Msg, SenderGone>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let q = &self.mb.pod.queue;
        match q.poll(cx) {
            Poll::Ready(Some(t)) => Poll::Ready(Ok(t)),
            Poll::Ready(None) => Poll::Ready(Err(SenderGone)),
            Poll::Pending => Poll::Pending,
        }
    }
}
