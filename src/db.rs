use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    role TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS zones (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS visits (
    id INTEGER PRIMARY KEY,
    visitor_name TEXT NOT NULL,
    company TEXT NOT NULL,
    visitor_user_id INTEGER,
    status TEXT NOT NULL,
    created_by INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS authorizations (
    id INTEGER PRIMARY KEY,
    visit_id INTEGER NOT NULL REFERENCES visits(id),
    code TEXT UNIQUE NOT NULL,
    code_hint TEXT NOT NULL,
    device_limit INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    valid_to TEXT NOT NULL,
    status TEXT NOT NULL,
    network_state TEXT NOT NULL DEFAULT 'pending',
    target_status TEXT,
    revoke_reason TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS auth_zones (
    auth_id INTEGER NOT NULL REFERENCES authorizations(id),
    zone_id INTEGER NOT NULL REFERENCES zones(id),
    PRIMARY KEY (auth_id, zone_id)
);

CREATE TABLE IF NOT EXISTS devices (
    id INTEGER PRIMARY KEY,
    auth_id INTEGER NOT NULL REFERENCES authorizations(id),
    mac TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    UNIQUE (auth_id, mac)
);

CREATE TABLE IF NOT EXISTS login_events (
    id INTEGER PRIMARY KEY,
    auth_id INTEGER,
    code_used TEXT NOT NULL,
    mac TEXT NOT NULL,
    zone_id INTEGER,
    result TEXT NOT NULL,
    detail TEXT,
    flagged INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notifications (
    id INTEGER PRIMARY KEY,
    audience TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL
);
"#;

const SEED: &str = r#"
INSERT OR IGNORE INTO users (id, username, role) VALUES
    (1, 'reception', 'front_desk'),
    (2, 'netadmin', 'network_admin');

INSERT OR IGNORE INTO zones (id, name, is_admin) VALUES
    (1, '大堂访客区', 0),
    (2, '会议区', 0),
    (3, '办公区', 0),
    (4, '管理区', 1);
"#;

pub async fn build_pool(db_url: &str) -> SqlitePool {
    let opts = SqliteConnectOptions::from_str(db_url)
        .expect("非法的数据库连接串")
        .create_if_missing(true)
        .foreign_keys(true);
    // 内存库必须单连接，否则每个连接看到不同的库
    let max = if db_url.contains(":memory:") { 1 } else { 5 };
    SqlitePoolOptions::new()
        .max_connections(max)
        .connect_with(opts)
        .await
        .expect("无法连接数据库")
}

pub async fn init(pool: &SqlitePool) {
    sqlx::raw_sql(SCHEMA).execute(pool).await.expect("建表失败");
    sqlx::raw_sql(SEED).execute(pool).await.expect("种子数据失败");
}
