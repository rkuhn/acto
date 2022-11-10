use crate::{join, tokio::ActoTokio, ActoCell, ActoInput, ActoRuntime};
use std::sync::Arc;
use tokio::sync::oneshot;

#[test]
fn supervisor_termination() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let sys = ActoTokio::new(rt.handle(), "test");

    let probe = Arc::new(());

    let (tx, rx) = oneshot::channel();
    let probe2 = probe.clone();
    let (r, j) = sys.spawn_actor(move |mut cell| async move {
        let probe3 = probe2.clone();
        let _r = cell.spawn_supervised(move |mut cell: ActoCell<i32, _>| async move {
            cell.recv().await;
            probe3
        });
        let probe3 = probe2.clone();
        let _r = cell.spawn_supervised(move |mut cell: ActoCell<i32, _>| async move {
            cell.recv().await;
            probe3
        });
        tx.send(()).ok();
        cell.recv().await
    });

    rt.block_on(rx).unwrap();
    assert_eq!(Arc::strong_count(&probe), 4);

    r.send(()).ok();
    let msg = rt.block_on(join(j)).unwrap();
    assert_eq!(msg, ActoInput::Message(()));
    assert_eq!(Arc::strong_count(&probe), 1);
}
