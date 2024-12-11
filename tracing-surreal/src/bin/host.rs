use est::AnyRes;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Surreal,
};
use tracing_surreal::{stop::Stop, tmp::server::ServerBuilder};

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
    let stop = Stop::init(db().await?, "test").await?;
    let server = ServerBuilder::from_stop_default(&stop).start().await?;
    println!("{}", server.get_local_addr());
    server.join().await??;
    Ok(())
}
