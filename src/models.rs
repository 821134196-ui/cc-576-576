use chrono::{DateTime, Utc};
use serde::Serialize;

/// 授权状态机
pub mod st {
    /// 已创建，等待访客签到后生效
    pub const PENDING: &str = "pending_checkin";
    /// 已生效（已下发到网络设备）
    pub const ACTIVE: &str = "active";
    /// 已过期（网络侧已回收）
    pub const EXPIRED: &str = "expired";
    /// 已回收（离场 / 取消）
    pub const REVOKED: &str = "revoked";
    /// 已被重新发码 / 改期取代
    pub const SUPERSEDED: &str = "superseded";
    /// 网络回收失败，待处理 —— 绝不能当作已停用
    pub const REVOKE_FAILED: &str = "revoke_failed";

    pub fn is_terminal(s: &str) -> bool {
        matches!(s, EXPIRED | REVOKED | SUPERSEDED)
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String, // front_desk | network_admin
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Zone {
    pub id: i64,
    pub name: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Visit {
    pub id: i64,
    pub visitor_name: String,
    pub company: String,
    pub visitor_user_id: Option<i64>,
    pub status: String, // booked | checked_in | checked_out | cancelled
    pub created_by: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Auth {
    pub id: i64,
    pub visit_id: i64,
    pub code: String,
    pub code_hint: String,
    pub device_limit: i64,
    pub valid_from: DateTime<Utc>,
    pub valid_to: DateTime<Utc>,
    pub status: String,
    /// 网络设备侧真实状态：pending(未下发) | applied(已下发) | revoked(已回收)
    /// 回收失败时保持 applied —— 设备上权限仍然存在，必须可见
    pub network_state: String,
    /// 回收失败时记录目标状态，供重试使用
    pub target_status: Option<String>,
    pub revoke_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Device {
    pub id: i64,
    pub auth_id: i64,
    pub mac: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct LoginEvent {
    pub id: i64,
    pub auth_id: Option<i64>,
    pub code_used: String,
    pub mac: String,
    pub zone_id: Option<i64>,
    pub result: String,
    pub detail: Option<String>,
    pub flagged: bool,
    pub created_at: DateTime<Utc>,
}

/// 对外暴露的授权视图 —— 不含完整临时码，只带尾号
#[derive(Debug, Serialize)]
pub struct AuthView {
    pub id: i64,
    pub visit_id: i64,
    pub code_hint: String,
    pub device_limit: i64,
    pub device_count: i64,
    pub valid_from: DateTime<Utc>,
    pub valid_to: DateTime<Utc>,
    pub status: String,
    pub network_state: String,
    pub revoke_reason: Option<String>,
    pub zones: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct VisitView {
    pub id: i64,
    pub visitor_name: String,
    pub company: String,
    pub visitor_user_id: Option<i64>,
    pub status: String,
    pub created_by: i64,
    pub created_at: DateTime<Utc>,
    pub auths: Vec<AuthView>,
}
