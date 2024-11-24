use est::AnyRes;
use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;

#[tokio::main]
async fn main() -> AnyRes {
    println!(
        "v{}",
        Surreal::new::<RocksDb>(".rocksdb").await?.version().await?
    );
    Ok(())
}
