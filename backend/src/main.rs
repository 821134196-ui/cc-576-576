use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::Router;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use guestnet::clock::SystemClock;
use guestnet::state::AppState;
use guestnet::web::router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "guestnet_server=info,tower_http=warn".into()),
        )
        .init();

    let db_path = std::env::var("GUESTNET_DB").unwrap_or_else(|_| "guestnet.db".to_string());
    let url = format!("sqlite://{db_path}");
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 8787)));

    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "../frontend/dist".into());
    let serve = ServeDir::new(&static_dir)
        .fallback(ServeFile::new(format!("{static_dir}/index.html")));

    let app: Router = Router::new()
        .merge(router(AppState {
            pool,
            clock: Arc::new(SystemClock),
        }))
        .fallback_service(serve)
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("访客临时码系统已启动: http://{addr} (静态目录: {static_dir})");
    axum::serve(listener, app).await?;
    Ok(())
}
