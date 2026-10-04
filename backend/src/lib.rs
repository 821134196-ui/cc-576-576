pub mod clock;
pub mod error;
pub mod models;
pub mod service;
pub mod sim;
pub mod state;
pub mod web;

use std::str::FromStr;
use std::sync::Arc;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::web::router;
use crate::state::AppState;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// 连接 SQLite。内存库使用单连接，保证各处理器看到同一份数据。
pub async fn connect_sqlite(url: &str) -> Result<SqlitePool, BoxError> {
    let is_memory = url.contains(":memory:") || url.contains("mode=memory");
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(if is_memory { 1 } else { 5 })
        .connect_with(opts)
        .await?;
    Ok(pool)
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), BoxError> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

/// 测试辅助：建内存库 + 迁移。
pub async fn test_pool() -> SqlitePool {
    let pool = connect_sqlite("sqlite::memory:")
        .await
        .expect("connect memory sqlite");
    run_migrations(&pool).await.expect("migrate");
    pool
}

pub fn build_app(pool: SqlitePool, clock: Arc<dyn Clock>) -> axum::Router {
    router(AppState { pool, clock })
}
