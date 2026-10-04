pub mod auth;
pub mod db;
pub mod error;
pub mod handlers;
pub mod models;
pub mod network;
pub mod sweeper;

use axum::Router;
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};

/// 可控制的时钟：默认真实时间，可冻结或加偏移（演示与测试用）
pub struct Clock {
    pub offset_secs: i64,
    pub frozen: Option<DateTime<Utc>>,
}

pub struct AppStateInner {
    pub pool: SqlitePool,
    pub network: network::MockNetwork,
    pub clock: Mutex<Clock>,
}

pub type AppState = Arc<AppStateInner>;

impl AppStateInner {
    pub fn now(&self) -> DateTime<Utc> {
        let c = self.clock.lock().unwrap();
        match c.frozen {
            Some(t) => t,
            None => Utc::now() + Duration::seconds(c.offset_secs),
        }
    }
}

pub async fn build_state(db_url: &str) -> AppState {
    let pool = db::build_pool(db_url).await;
    db::init(&pool).await;
    Arc::new(AppStateInner {
        pool,
        network: network::MockNetwork::new(),
        clock: Mutex::new(Clock {
            offset_secs: 0,
            frozen: None,
        }),
    })
}

pub fn build_app(state: AppState) -> Router {
    handlers::router(state)
}
