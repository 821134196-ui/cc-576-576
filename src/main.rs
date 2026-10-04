use visitor_net::{build_app, build_state, sweeper};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "visitor_net=info,tower_http=info".into()),
        )
        .init();

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://visitor_net.db".to_string());
    let state = build_state(&db_url).await;
    sweeper::spawn(state.clone());
    let app = build_app(state);

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("端口绑定失败");
    println!("访客网络临时码系统已启动: http://localhost:3000");
    println!("  前台账号 X-User-Id: 1 (reception)   网管账号 X-User-Id: 2 (netadmin)");
    axum::serve(listener, app).await.expect("服务异常退出");
}
