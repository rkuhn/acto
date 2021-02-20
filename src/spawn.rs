use std::{future::Future, thread};

pub trait Spawn {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}
impl Spawn for tokio::runtime::Runtime {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn(f);
    }
}
impl Spawn for tokio::runtime::Handle {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn(f);
    }
}

pub struct OwnThread;
impl Spawn for OwnThread {
    fn spawn<F>(&self, f: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            rt.block_on(f);
            println!("thread stopped");
        });
    }
}
