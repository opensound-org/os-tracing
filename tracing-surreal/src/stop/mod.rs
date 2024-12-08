use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::{Connection, RecordId, Surreal};
use ulid::Ulid;

#[derive(Clone, Debug)]
pub struct Stop<C: Connection> {
    db: Surreal<C>,
    session_id: RecordId,
    formatted_timestamp: String,
}

impl<C: Connection> Stop<C> {
    pub async fn init(db: Surreal<C>, app: &str) -> surrealdb::Result<Self> {
        db.use_db(format!("app-tracing-{}", app)).await?;

        #[derive(Debug, Deserialize, Serialize)]
        struct SessionRecord {
            a_timestamp: DateTime<Local>,
            b_access_method: Option<String>,
            c_record_auth: Option<String>,
            d_http_origin: Option<String>,
            e_session_ip: Option<String>,
            f_session_id: Option<String>,
            g_session_token: Option<Value>,
        }

        #[derive(Debug, Deserialize, Serialize)]
        struct SessionId {
            id: RecordId,
        }

        let a_timestamp = Local::now();
        let b_access_method = db.run("session::ac").await?;
        let c_record_auth = db.run("session::rd").await?;
        let d_http_origin = db.run("session::origin").await?;
        let e_session_ip = db.run("session::ip").await?;
        let f_session_id = db.run("session::id").await?;
        let g_session_token = db.run("session::token").await?;
        let record = SessionRecord {
            a_timestamp,
            b_access_method,
            c_record_auth,
            d_http_origin,
            e_session_ip,
            f_session_id,
            g_session_token,
        };
        let id: Option<SessionId> = db
            .create((
                "sessions",
                Ulid::from_datetime(a_timestamp.into()).to_string(),
            ))
            .content(record)
            .await?;
        let session_id = id.unwrap().id;
        let formatted_timestamp = a_timestamp.format("%y%m%d-%H%M%S").to_string();

        Ok(Self {
            db,
            session_id,
            formatted_timestamp,
        })
    }

    pub async fn print(&self) {
        println!("{}", self.db.version().await.unwrap());
        println!("{}", self.session_id);
        println!("{}-clients", self.formatted_timestamp);
    }
}
