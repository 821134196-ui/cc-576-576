use std::sync::Arc;

use sqlx::SqlitePool;

use crate::clock::Clock;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub clock: Arc<dyn Clock>,
}

/// 前台可授权的最大时长（分钟）——8 小时，禁止给自己/访客开“长期权限”。
pub const FRONT_DESK_MAX_DURATION_MIN: i64 = 8 * 60;
/// 管理员发码的最大时长（分钟）——7 天。
pub const ADMIN_MAX_DURATION_MIN: i64 = 7 * 24 * 60;
/// 单授权最大设备数。
pub const MAX_DEVICE_LIMIT: i64 = 10;
