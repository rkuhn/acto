use crate::{actor::ActoAborted, ActoHandle, ActoId, ActoRuntime, MailboxSize, Receiver, Sender};
use parking_lot::Mutex;
use smol_str::SmolStr;
use std::{
    any::Any,
    future::{poll_fn, Future},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
};

/// An [`ActoRuntime`] based on [`tokio::runtime`] and [`tokio::sync::mpsc`] queues.
#[derive(Clone)]
pub struct AcTokio {
    inner: Arc<Inner>,
    mailbox_size: usize,
}

impl AcTokio {
    pub fn new(name: impl Into<String>, num_threads: usize) -> std::io::Result<Self> {
        let name = name.into();
        let rt = Builder::new_multi_thread()
            .thread_name(&name)
            .worker_threads(num_threads)
            .enable_all()
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                name,
                rt,
                dropping: AtomicUsize::default(),
                waiting: Mutex::new(Vec::new()),
            }),
            mailbox_size: 128,
        })
    }

    pub fn rt(&self) -> &Runtime {
        &self.inner.rt
    }

    /// Wait for all aborted actors to be dropped
    pub fn drop_done(&self) -> DropDone {
        DropDone(self.inner.clone())
    }
}

struct Inner {
    name: String,
    rt: Runtime,
    dropping: AtomicUsize,
    waiting: Mutex<Vec<Waker>>,
}

impl Inner {
    fn notify_drop(&self) {
        let mut waiting = self.waiting.lock();
        for waker in waiting.drain(..) {
            waker.wake();
        }
    }
}

impl ActoRuntime for AcTokio {
    type ActoHandle<O: Send + 'static> = TokioJoinHandle<O>;
    type Sender<M: Send + 'static> = TokioSender<M>;
    type Receiver<M: Send + 'static> = TokioReceiver<M>;

    fn name(&self) -> &str {
        &self.inner.name
    }

    fn mailbox<M: Send + 'static>(&self) -> (Self::Sender<M>, Self::Receiver<M>) {
        let (tx, rx) = mpsc::channel(self.mailbox_size);
        (TokioSender(tx), TokioReceiver(rx))
    }

    fn spawn_task<T>(&self, id: ActoId, name: SmolStr, task: T) -> Self::ActoHandle<T::Output>
    where
        T: std::future::Future + Send + 'static,
        T::Output: Send + 'static,
    {
        TokioJoinHandle(
            id,
            name,
            Some(self.inner.rt.spawn(task)),
            self.inner.clone(),
        )
    }
}

pub struct DropDone(Arc<Inner>);

impl Future for DropDone {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0.dropping.load(Ordering::Relaxed) == 0 {
            Poll::Ready(())
        } else {
            self.0.waiting.lock().push(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl MailboxSize for AcTokio {
    type Output = Self;

    fn with_mailbox_size(&self, mailbox_size: usize) -> Self::Output {
        Self {
            inner: self.inner.clone(),
            mailbox_size,
        }
    }
}

pub struct TokioSender<M>(mpsc::Sender<M>);
impl<M: Send + 'static> Sender<M> for TokioSender<M> {
    fn send(&self, msg: M) -> bool {
        self.0.try_send(msg).is_ok()
    }
}

pub struct TokioReceiver<M>(mpsc::Receiver<M>);
impl<M: Send + 'static> Receiver<M> for TokioReceiver<M> {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<M> {
        self.0.poll_recv(cx).map(Option::unwrap)
    }
}

pub struct TokioJoinHandle<O>(
    ActoId,
    SmolStr,
    Option<tokio::task::JoinHandle<O>>,
    Arc<Inner>,
);

impl<O: Send + 'static> TokioJoinHandle<O> {
    pub fn join(mut self) -> impl Future<Output = Result<O, Box<dyn Any + Send + 'static>>> {
        poll_fn(move |cx| self.poll(cx))
    }
}

impl<O: Send + 'static> ActoHandle for TokioJoinHandle<O> {
    type Output = O;

    fn id(&self) -> ActoId {
        self.0
    }

    fn name(&self) -> &str {
        &self.1
    }

    fn abort(&mut self) {
        let inner = self.3.clone();
        inner.dropping.fetch_add(1, Ordering::Relaxed);
        let handle = self.2.take();
        if let Some(handle) = handle {
            handle.abort();
            self.3.rt.spawn(async move {
                handle.await.ok();
                let d = inner.dropping.fetch_sub(1, Ordering::Relaxed);
                if d == 1 {
                    inner.notify_drop();
                }
            });
        }
    }

    fn is_finished(&mut self) -> bool {
        self.2.as_ref().map(|h| h.is_finished()).unwrap_or(true)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<O, Box<dyn Any + Send + 'static>>> {
        if let Some(ref mut handle) = self.2 {
            Pin::new(handle)
                .poll(cx)
                .map(|r| r.map_err(|err| Box::new(err) as Box<dyn Any + Send + 'static>))
        } else {
            Poll::Ready(Err(Box::new(ActoAborted::new(self.1.as_str()))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AcTokio;
    use crate::{join, ActoCell, ActoId, ActoInput, ActoRuntime, SupervisionRef};
    use std::{
        collections::BTreeMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tokio::sync::oneshot;

    #[test]
    fn run() {
        let sys = AcTokio::new("test", 1).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = flag.clone();
        let SupervisionRef { me: r, handle: j } =
            sys.spawn_actor("super", |mut ctx: ActoCell<_, _>| async move {
                flag2.store(true, Ordering::Relaxed);
                ctx.recv().await;
                flag2.store(false, Ordering::Relaxed);
                42
            });
        std::thread::sleep(Duration::from_millis(100));
        assert!(flag.load(Ordering::Relaxed));
        r.send(());
        std::thread::sleep(Duration::from_millis(100));
        assert!(!flag.load(Ordering::Relaxed));
        let ret = sys.rt().block_on(join(j));
        assert_eq!(ret.unwrap(), 42);
    }

    #[test]
    fn child() {
        let sys = AcTokio::new("test", 2).unwrap();
        let SupervisionRef { me: r, handle: j } = sys.spawn_actor(
            "super",
            |mut ctx: ActoCell<_, _, Result<i32, i32>>| async move {
                let mut v: Vec<i32> = vec![];
                let mut running = BTreeMap::<ActoId, (i32, oneshot::Sender<()>)>::new();
                loop {
                    match ctx.recv().await {
                        ActoInput::NoMoreSenders => break,
                        ActoInput::Supervision { id, result, .. } => {
                            let (arg, tx) = running.remove(&id).unwrap();
                            v.push(arg);
                            v.push(result.unwrap().unwrap_or_else(|x| x));
                            tx.send(()).unwrap();
                        }
                        ActoInput::Message((x, tx)) => {
                            v.push(x);
                            let r = ctx.spawn_supervised(
                                "child",
                                |mut ctx: ActoCell<_, _>| async move {
                                    if let ActoInput::Message(x) = ctx.recv().await {
                                        Ok(2 * x)
                                    } else {
                                        Err(5)
                                    }
                                },
                            );
                            r.send(x);
                            running.insert(r.id(), (x, tx));
                        }
                    }
                }
                v
            },
        );
        let (tx, rx) = oneshot::channel();
        r.send((1, tx));
        sys.rt().block_on(rx).unwrap();
        let (tx, rx) = oneshot::channel();
        r.send((2, tx));
        sys.rt().block_on(rx).unwrap();
        drop(r);
        let v = sys.rt().block_on(join(j)).unwrap();
        assert_eq!(v, vec![1, 1, 2, 2, 2, 4]);
    }
}
