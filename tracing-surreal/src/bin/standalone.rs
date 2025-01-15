use est::AnyRes;
use std::time::Duration;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Surreal,
};
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_surreal::{stop::Stop, tracing_msg::TracingLayerDefault};

async fn db() -> AnyRes<Surreal<Client>> {
    let db = Surreal::new::<Ws>("localhost:8000").await?;

    db.signin(Root {
        username: "root",
        password: "root",
    })
    .await?;
    db.use_ns("root").await?;
    Ok(db)
}

#[tokio::main]
async fn main() -> AnyRes {
    let (stop, mut routine_obs) = Stop::builder_default(db().await?, "test").init().await?;
    let (layer, routine_msg) = stop
        .tracing_layer_default()
        .close_transport_on_shutdown()
        .build();
    tracing_subscriber::registry().with(layer).init();
    let shutdown_trigger = CancellationToken::new();
    let shutdown_waiter = shutdown_trigger.clone();
    let mut routine_trace = tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs_f64(1.0));

        for i in 0..5 {
            tokio::select! {
                _ = shutdown_waiter.cancelled() => {
                    println!("shutdown routine_trace");
                    break;
                }
                _ = interval.tick() => {
                    tracing::info!(index = i, "msg {}", i);
                    println!("tick");
                }
            }
        }
    });

    let grace_type = tokio::select! {
        res = &mut routine_obs => {
            println!("routine_obs exited");
            shutdown_trigger.cancel();
            routine_trace.await.ok();
            res??
        }
        _ = &mut routine_trace => {
            println!("routine_trace exited");
            println!("{:?}", stop.query_last_n(3).await);
            routine_msg.graceful_shutdown().await??;
            routine_obs.graceful_shutdown().await??
        }
    };

    println!("{:?}", grace_type);
    Ok(())
}
