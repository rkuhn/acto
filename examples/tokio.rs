use acto::{ActoCell, ActoHandle, ActoInput, ActoRuntime, SupervisionRef};

async fn actor(mut ctx: ActoCell<i32, impl ActoRuntime>) {
    println!("main actor started");
    while let ActoInput::Message(m) = ctx.recv().await {
        ctx.spawn_supervised("subordinate", |mut ctx: ActoCell<_, _>| async move {
            println!("spawned actor for {:?}", ctx.recv().await);
        })
        .send(m);

        let r = ctx.spawn("worker", |mut ctx: ActoCell<_, _>| async move {
            match ctx.recv().await {
                ActoInput::NoMoreSenders => "no send".to_owned(),
                ActoInput::Supervision { .. } => unreachable!(),
                ActoInput::Message(m) => {
                    println!("received {}", m);
                    "send".to_owned()
                }
            }
        });
        r.me.send(5 * m);
        let result = r.handle.join().await;
        println!("actor result: {:?}", result);
    }
}

#[cfg(all(feature = "tokio", feature = "rt-multi-thread"))]
fn main() {
    let system = acto::AcTokio::new_multi_thread("theMain", 2).unwrap();
    let SupervisionRef { me: r, handle: j } = system.spawn_actor("supervisor", actor);
    r.send(1);
    r.send(2);
    let x = system.with_rt(|rt| rt.block_on(j.join()));
    println!("result: {:?}", x);
}

#[cfg(not(all(feature = "tokio", feature = "rt-multi-thread")))]
fn main() {
    println!("This example requires the 'tokio' feature");
}
