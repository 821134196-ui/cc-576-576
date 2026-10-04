use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub is_admin: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Visitor {
    pub id: String,
    pub name: String,
    pub company_id: String,
    pub checked_in: i64,
    pub check_in_at: Option<i64>,
    pub check_out_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct GrantRow {
    pub id: String,
    #[serde(skip_serializing)]
    pub code_hash: String,
    pub code_last4: String,
    pub visitor_id: String,
    pub company_id: String,
    pub zone_ids: String,
    pub device_limit: i64,
    pub duration_minutes: i64,
    pub valid_from: Option<i64>,
    pub expires_at: Option<i64>,
    pub status: String,
    pub intended_status: Option<String>,
    pub revoke_reason: Option<String>,
    pub replaced_by: Option<String>,
    pub provision_error: Option<String>,
    pub bound_user_id: Option<String>,
    pub created_by_role: String,
    pub created_at: i64,
    pub activated_at: Option<i64>,
    pub ended_at: Option<i64>,
}

impl GrantRow {
    pub fn zones(&self) -> Vec<String> {
        serde_json::from_str(&self.zone_ids).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    pub id: String,
    pub grant_id: String,
    pub mac: String,
    pub user_id: Option<String>,
    pub device_name: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AuthEventRow {
    pub id: i64,
    pub ts: i64,
    pub grant_id: Option<String>,
    pub visitor_id: Option<String>,
    pub code_last4: Option<String>,
    pub mac: Option<String>,
    pub user_id: Option<String>,
    pub zone_id: Option<String>,
    pub result: String,
    pub reason: String,
    pub anomaly: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct NotificationRow {
    pub id: i64,
    pub ts: i64,
    pub channel: String,
    pub kind: String,
    pub message: String,
    pub ref_id: Option<String>,
}

// ---------- 请求 DTO ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVisitorReq {
    pub name: String,
    pub company_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueGrantReq {
    pub zone_ids: Vec<String>,
    pub device_limit: i64,
    pub duration_minutes: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAuthReq {
    pub code: String,
    pub mac: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub zone_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeReq {
    #[serde(default)]
    pub reason: Option<String>,
}
