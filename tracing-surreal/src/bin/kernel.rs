use est::AnyRes;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Surreal,
};
use tracing_surreal::stop::Stop;

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
    Stop::init(db().await?, "test").await?.print().await;
    Ok(())
}
