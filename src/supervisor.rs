use crate::{MultiSender, SingleSender};

pub trait Supervisor<T>: Send + 'static {
    fn notify(self, result: T);
}

impl<T: Send + 'static> Supervisor<T> for MultiSender<T> {
    fn notify(self, result: T) {
        self.send(result);
    }
}

impl<T: Send + 'static> Supervisor<T> for SingleSender<T> {
    fn notify(mut self, result: T) {
        self.send(result);
    }
}

impl<T> Supervisor<T> for () {
    fn notify(self, _: T) {}
}

impl<T: Send + 'static> Supervisor<T> for std::sync::mpsc::Sender<T> {
    fn notify(self, result: T) {
        let _ = self.send(result);
    }
}

impl<T: Send + 'static> Supervisor<T> for tokio::sync::mpsc::Sender<T> {
    fn notify(self, result: T) {
        let _ = self.send(result);
    }
}

impl<T: Send + 'static> Supervisor<T> for tokio::sync::oneshot::Sender<T> {
    fn notify(self, result: T) {
        let _ = self.send(result);
    }
}

impl<T: Send + Sync + 'static> Supervisor<T> for tokio::sync::watch::Sender<T> {
    fn notify(self, result: T) {
        let _ = self.send(result);
    }
}

impl<T: Send + 'static> Supervisor<T> for tokio::sync::broadcast::Sender<T> {
    fn notify(self, result: T) {
        let _ = self.send(result);
    }
}
