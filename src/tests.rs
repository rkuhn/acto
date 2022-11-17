use crate::{join, tokio::ActoTokio, ActoCell, ActoHandle, ActoInput, ActoRuntime};
use std::{sync::Arc, time::Duration};
use tokio::sync::oneshot;

macro_rules! assert_timed {
    ($cond:expr $(,$($arg:tt)+)?) => {
        for _ in 0..300 {
            std::thread::sleep(Duration::from_millis(10));
            if $cond { break; }
        }
        assert!($cond $(,$($arg)*)?);
    };
}

#[test]
fn supervisor_termination() {
    tracing_subscriber::fmt().with_env_filter("trace").init();
    let sys = ActoTokio::new("test", 2).unwrap();

    let probe = Arc::new(());

    let (tx, rx) = oneshot::channel();
    let probe2 = probe.clone();
    let (r, j) = sys.spawn_actor("super", move |mut cell| async move {
        let probe3 = probe2.clone();
        let _r = cell.spawn_supervised("c1", move |mut cell: ActoCell<i32, _>| async move {
            cell.recv().await;
            probe3
        });
        let probe3 = probe2.clone();
        let _r = cell.spawn_supervised("c2", move |mut cell: ActoCell<i32, _>| async move {
            cell.recv().await;
            probe3
        });
        tx.send(()).ok();
        cell.recv().await
    });
    assert_eq!(r.name(), "super(test/1)");

    sys.rt().block_on(rx).unwrap();
    assert_eq!(Arc::strong_count(&probe), 4);

    r.send(()).ok();
    let msg = sys.rt().block_on(join(j)).unwrap();
    assert_eq!(msg, ActoInput::Message(()));
    assert_timed!(
        Arc::strong_count(&probe) == 1,
        " was {}",
        Arc::strong_count(&probe)
    );
}

#[test]
fn termination_info() {
    let sys = ActoTokio::new("test", 2).unwrap();
    let (r, mut j) = sys.spawn_actor("buh", |mut cell: ActoCell<(), _>| async move {
        loop {
            cell.recv().await;
        }
    });
    assert!(!r.is_gone());
    assert!(!j.is_finished());
    j.abort();
    assert_timed!(j.is_finished());
    assert!(r.is_gone());
}
