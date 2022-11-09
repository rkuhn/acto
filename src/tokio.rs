use crate::{ActoHandle, ActoId, ActoRuntime, Receiver, Sender};
use std::{
    any::Any,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::{
    runtime::Handle,
    sync::mpsc::{self, error::TrySendError},
};

/// An [`ActoRuntime`] based on [`tokio::runtime`] and [`tokio::sync::mpsc`] queues.
#[derive(Clone)]
pub struct ActoTokio(Arc<Inner>);

impl ActoTokio {
    pub fn new(handle: &Handle, name: impl Into<String>) -> Self {
        Self(Arc::new(Inner {
            name: name.into(),
            id: AtomicUsize::new(1),
            handle: handle.clone(),
            mailbox_size: 128,
        }))
    }
}

struct Inner {
    name: String,
    id: AtomicUsize,
    handle: Handle,
    mailbox_size: usize,
}

impl ActoRuntime for ActoTokio {
    type ActoHandle<O: Send + 'static> = TokioJoinHandle<O>;
    type Sender<M: Send + 'static> = TokioSender<M>;
    type Receiver<M: Send + 'static> = TokioReceiver<M>;

    fn next_id(&self) -> usize {
        self.0.id.fetch_add(1, Ordering::Relaxed)
    }

    fn mailbox<M: Send + 'static>(&self) -> (Self::Sender<M>, Self::Receiver<M>) {
        let (tx, rx) = mpsc::channel(self.0.mailbox_size);
        (TokioSender(tx), TokioReceiver(rx))
    }

    fn spawn_task<T>(&self, id: ActoId, task: T) -> Self::ActoHandle<T::Output>
    where
        T: std::future::Future + Send + 'static,
        T::Output: Send + 'static,
    {
        TokioJoinHandle(id, self.0.handle.spawn(task))
    }

    fn fmt(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        write!(f, "{}", self.0.name)
    }
}

#[doc(hidden)]
pub struct TokioSender<M>(mpsc::Sender<M>);
impl<M: Send + 'static> Sender<M> for TokioSender<M> {
    fn send(&self, msg: M) -> Result<(), M> {
        match self.0.try_send(msg) {
            Ok(_) => Ok(()),
            Err(TrySendError::Closed(m)) => Err(m),
            Err(TrySendError::Full(m)) => Err(m),
        }
    }
}

#[doc(hidden)]
pub struct TokioReceiver<M>(mpsc::Receiver<M>);
impl<M: Send + 'static> Receiver<M> for TokioReceiver<M> {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<M> {
        self.0.poll_recv(cx).map(Option::unwrap)
    }
}

#[doc(hidden)]
pub struct TokioJoinHandle<O>(ActoId, tokio::task::JoinHandle<O>);
impl<O: Send + 'static> ActoHandle for TokioJoinHandle<O> {
    type Output = O;

    fn id(&self) -> ActoId {
        self.0
    }

    fn abort(self) {
        self.1.abort();
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<O, Box<dyn Any + Send + 'static>>> {
        Pin::new(&mut self.1)
            .poll(cx)
            .map(|r| r.map_err(|err| Box::new(err) as Box<dyn Any + Send + 'static>))
    }
}

#[cfg(test)]
mod tests {
    use super::ActoTokio;
    use crate::{join, ActoId, ActoInput, ActoRuntime};
    use std::{
        collections::BTreeMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tokio::{runtime::Runtime, sync::oneshot};
    use tracing_subscriber::EnvFilter;

    #[test]
    fn run() {
        let rt = Runtime::new().unwrap();
        let sys = ActoTokio::new(rt.handle(), "test");
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = flag.clone();
        let (r, j) = sys.spawn_actor(|mut ctx| async move {
            flag2.store(true, Ordering::Relaxed);
            ctx.recv().await;
            flag2.store(false, Ordering::Relaxed);
            42
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(flag.load(Ordering::Relaxed));
        r.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!flag.load(Ordering::Relaxed));
        let ret = rt.block_on(join(j));
        assert_eq!(ret.unwrap(), 42);
    }

    #[test]
    fn child() {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::builder().parse("trace").unwrap())
            .init();
        let rt = Runtime::new().unwrap();
        let sys = ActoTokio::new(rt.handle(), "test");
        let (r, j) = sys.spawn_actor(|mut ctx| async move {
            let mut v: Vec<i32> = vec![];
            let mut running = BTreeMap::<ActoId, (i32, oneshot::Sender<()>)>::new();
            loop {
                match ctx.recv().await {
                    ActoInput::NoMoreSenders => break,
                    ActoInput::Supervision(id, x) => {
                        let (arg, tx) = running.remove(&id).unwrap();
                        let res = x.downcast::<Result<i32, i32>>().unwrap();
                        v.push(arg);
                        v.push(res.unwrap_or_else(|x| x));
                        tx.send(()).unwrap();
                    }
                    ActoInput::Message((x, tx)) => {
                        v.push(x);
                        let r = ctx.spawn_supervised(|mut ctx| async move {
                            if let ActoInput::Message(x) = ctx.recv().await {
                                Ok(2 * x)
                            } else {
                                Err(5)
                            }
                        });
                        r.send(x).unwrap();
                        running.insert(r.id(), (x, tx));
                    }
                }
            }
            v
        });
        let (tx, rx) = oneshot::channel();
        r.send((1, tx)).unwrap();
        rt.block_on(rx).unwrap();
        let (tx, rx) = oneshot::channel();
        r.send((2, tx)).unwrap();
        rt.block_on(rx).unwrap();
        drop(r);
        let v = rt.block_on(join(j)).unwrap();
        assert_eq!(v, vec![1, 1, 2, 2, 2, 4]);
    }
}
