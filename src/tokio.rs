use crate::{
    actor::{Dropper, Receiver, Sender, TaskFuture},
    ActorError, Runtime,
};
use std::{boxed::Box, task::Poll};
use tokio::{runtime::Handle, sync::mpsc};

#[derive(Clone)]
pub struct Tokio {
    runtime: Handle,
    queue_size: usize,
    error_reporter: Option<fn(ActorError)>,
}

impl Tokio {
    pub fn new(runtime: Handle, queue_size: usize) -> Self {
        Self {
            runtime,
            queue_size,
            error_reporter: None,
        }
    }

    pub fn from_current(queue_size: usize) -> Self {
        Self {
            runtime: Handle::current(),
            queue_size,
            error_reporter: None,
        }
    }

    pub fn with_error_reporter(self, error_reporter: fn(ActorError)) -> Self {
        Self {
            error_reporter: Some(error_reporter),
            ..self
        }
    }
}

impl Runtime for Tokio {
    fn new_actor(&self, msg_size: usize, dropper: Dropper) -> (Sender, Receiver) {
        let (tx, mut rx) = mpsc::channel(self.queue_size);
        let tell = Box::new(move |bytes: *const u8| {
            let bytes = unsafe { std::slice::from_raw_parts(bytes, msg_size) }.into();
            let _ = tx.try_send(Message { bytes, dropper });
        });
        let recv: Receiver = Box::new(move |bytes, cx| {
            // wtf
            match rx.poll_recv(cx) {
                Poll::Ready(Some(msg)) => {
                    unsafe { std::ptr::copy_nonoverlapping(msg.bytes.as_ptr(), bytes, msg_size) };
                    msg.dispose();
                    Poll::Ready(())
                }
                Poll::Ready(None) => unreachable!(),
                Poll::Pending => Poll::Pending,
            }
        });
        (tell, recv)
    }

    fn spawn(&self, task: TaskFuture) {
        let error_reporter = self.error_reporter;
        self.runtime.spawn(async move {
            match task.await {
                Ok(_) => {}
                Err(e) => {
                    if let Some(reporter) = error_reporter {
                        (reporter)(e);
                    }
                }
            }
        });
    }
}

struct Message {
    bytes: Box<[u8]>,
    dropper: unsafe fn(*mut u8),
}

impl Message {
    fn dispose(mut self) {
        fn no_drop(_x: *mut u8) {}
        self.dropper = no_drop;
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        unsafe { (self.dropper)(self.bytes.as_mut_ptr()) };
    }
}

#[cfg(all(test, feature = "with_tokio"))]
mod tests {
    use super::*;
    use crate::{spawn, ActorCell, ActorRef, ActorResult, NoActorRef};
    use futures::poll;
    use std::{
        task::Poll,
        time::{Duration, Instant},
    };
    use tokio::{
        sync::oneshot,
        time::{sleep, timeout},
    };

    async fn actor(mut ctx: ActorCell<(String, ActorRef<String>)>) -> ActorResult {
        loop {
            let (name, sender) = ctx.receive().await?;
            if name == "exit" {
                break Ok(());
            }
            let responder = ctx.spawn(|mut ctx| async move {
                let m = ctx.receive().await?;
                sender.tell(format!("Hello {}!", m));
                Ok(())
            });
            responder.tell(name);
        }
    }

    #[tokio::test]
    async fn smoke() {
        let rt = Tokio::from_current(12);
        let aref = spawn(rt.clone(), actor);

        let (tx, rx) = oneshot::channel();
        let receiver = spawn(rt.clone(), |mut ctx| async move {
            let msg = ctx.receive().await?;
            let _ = tx.send(msg);
            Ok(())
        });
        aref.tell(("Fred".to_owned(), receiver));
        assert_eq!(rx.await.unwrap(), "Hello Fred!");

        let (tx, rx) = oneshot::channel();
        let receiver = spawn(rt, |mut ctx| async move {
            let msg = ctx.receive().await?;
            let _ = tx.send(msg);
            Ok(())
        });
        aref.tell(("Barney".to_owned(), receiver.clone()));
        assert_eq!(rx.await.unwrap(), "Hello Barney!");

        aref.tell(("exit".to_owned(), receiver));
        let now = Instant::now();
        while now.elapsed() < Duration::from_secs(3) {
            if aref.is_dead() {
                return;
            }
            sleep(Duration::from_millis(200)).await;
        }
        panic!("actor did not stop");
    }

    #[tokio::test]
    async fn dropped() {
        let (tx, mut rx) = oneshot::channel();
        let aref = spawn(
            Tokio::from_current(12),
            |mut ctx: ActorCell<()>| async move {
                let result = ctx.receive().await;
                let _ = tx.send(result);
                Ok(())
            },
        );

        sleep(Duration::from_millis(200)).await;
        match poll!(&mut rx) {
            Poll::Pending => {}
            x => panic!("unexpected result: {:?}", x),
        }

        drop(aref);
        let err = timeout(Duration::from_secs(3), rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(err, NoActorRef);
    }
}
