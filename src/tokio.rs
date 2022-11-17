use crate::{ActoHandle, ActoId, ActoRuntime, Receiver, Sender};
use smol_str::SmolStr;
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
    runtime::{Builder, Runtime},
    sync::mpsc::{self, error::TrySendError},
};

/// An [`ActoRuntime`] based on [`tokio::runtime`] and [`tokio::sync::mpsc`] queues.
#[derive(Clone)]
pub struct ActoTokio(Arc<Inner>);

impl ActoTokio {
    pub fn new(name: impl Into<String>, num_threads: usize) -> std::io::Result<Self> {
        let name = name.into();
        let rt = Builder::new_multi_thread()
            .thread_name(&name)
            .worker_threads(num_threads)
            .enable_all()
            .build()?;
        Ok(Self(Arc::new(Inner {
            name,
            id: AtomicUsize::new(1),
            rt,
            mailbox_size: 128,
        })))
    }

    pub fn rt(&self) -> &Runtime {
        &self.0.rt
    }
}

struct Inner {
    name: String,
    id: AtomicUsize,
    rt: Runtime,
    mailbox_size: usize,
}

impl ActoRuntime for ActoTokio {
    type ActoHandle<O: Send + 'static> = TokioJoinHandle<O>;
    type Sender<M: Send + 'static> = TokioSender<M>;
    type Receiver<M: Send + 'static> = TokioReceiver<M>;

    fn name(&self) -> &str {
        &self.0.name
    }

    fn next_id(&self) -> usize {
        self.0.id.fetch_add(1, Ordering::Relaxed)
    }

    fn mailbox<M: Send + 'static>(&self) -> (Self::Sender<M>, Self::Receiver<M>) {
        let (tx, rx) = mpsc::channel(self.0.mailbox_size);
        (TokioSender(tx), TokioReceiver(rx))
    }

    fn spawn_task<T>(&self, id: ActoId, name: SmolStr, task: T) -> Self::ActoHandle<T::Output>
    where
        T: std::future::Future + Send + 'static,
        T::Output: Send + 'static,
    {
        TokioJoinHandle(id, name, self.0.rt.spawn(task))
    }
}

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

pub struct TokioReceiver<M>(mpsc::Receiver<M>);
impl<M: Send + 'static> Receiver<M> for TokioReceiver<M> {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<M> {
        self.0.poll_recv(cx).map(Option::unwrap)
    }
}

pub struct TokioJoinHandle<O>(ActoId, SmolStr, tokio::task::JoinHandle<O>);
impl<O: Send + 'static> ActoHandle for TokioJoinHandle<O> {
    type Output = O;

    fn id(&self) -> ActoId {
        self.0
    }

    fn name(&self) -> &str {
        &self.1
    }

    fn abort(&mut self) {
        self.2.abort();
    }

    fn is_finished(&mut self) -> bool {
        self.2.is_finished()
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<O, Box<dyn Any + Send + 'static>>> {
        Pin::new(&mut self.2)
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
    use tokio::sync::oneshot;

    #[test]
    fn run() {
        let sys = ActoTokio::new("test", 1).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = flag.clone();
        let (r, j) = sys.spawn_actor("super", |mut ctx| async move {
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
        let ret = sys.rt().block_on(join(j));
        assert_eq!(ret.unwrap(), 42);
    }

    #[test]
    fn child() {
        let sys = ActoTokio::new("test", 2).unwrap();
        let (r, j) = sys.spawn_actor("super", |mut ctx| async move {
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
                        let r = ctx.spawn_supervised("child", |mut ctx| async move {
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
        sys.rt().block_on(rx).unwrap();
        let (tx, rx) = oneshot::channel();
        r.send((2, tx)).unwrap();
        sys.rt().block_on(rx).unwrap();
        drop(r);
        let v = sys.rt().block_on(join(j)).unwrap();
        assert_eq!(v, vec![1, 1, 2, 2, 2, 4]);
    }
}
