use crate::pod::Queue;

pub trait Supervisor<T>: Send + 'static {
    fn notify(self, result: T);
}

impl<T> Supervisor<T::Msg> for crate::Sender<T>
where
    T: Queue + Send + 'static,
    T::Msg: Send + 'static,
{
    fn notify(mut self, result: T::Msg) {
        self.send_mut(result);
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
