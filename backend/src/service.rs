//! 核心业务逻辑：发码、签到激活、离场/到期回收、网络认证拦截、异常记录。
//!
//! 状态机（grants.status）：
//! ```text
//!  pending --签到/下发成功--> active
//!  pending --签到/下发失败--> provision_failed（可重试激活）
//!  active  --到期/离场/取消/改期/重发 且设备回收成功--> expired | checked_out | revoked | reissued
//!  active  --同上但设备回收失败--> revoke_pending（intended_status 记录目标态，绝不视为已停用）
//!  revoke_pending --设备恢复后重试成功--> 目标终结态
//! ```
//! 时间边界：有效区间为左闭右开 `[valid_from, expires_at)`，`now >= expires_at` 立即拒绝。

use rand::Rng;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{SqlitePool, Sqlite};
use sqlx::Transaction;

use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::sim;
use crate::state::{
    AppState, ADMIN_MAX_DURATION_MIN, FRONT_DESK_MAX_DURATION_MIN, MAX_DEVICE_LIMIT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    FrontDesk,
    NetworkAdmin,
}

impl Role {
    pub fn parse(header: Option<&str>) -> AppResult<Role> {
        match header {
            Some("front_desk") => Ok(Role::FrontDesk),
            Some("network_admin") => Ok(Role::NetworkAdmin),
            Some(other) => Err(AppError::Unauthorized(format!("unknown role: {other}"))),
            None => Err(AppError::Unauthorized(
                "missing X-Role header (front_desk|network_admin)".into(),
            )),
        }
    }

    pub fn require_admin(role: Role) -> AppResult<()> {
        match role {
            Role::NetworkAdmin => Ok(()),
            Role::FrontDesk => Err(AppError::Forbidden(
                "仅网络管理员可执行该操作".into(),
            )),
        }
    }
}

const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ"; // 去除易混字符 0/O/1/I/L

pub fn new_id() -> String {
    let mut buf = [0u8; 12];
    rand::thread_rng().fill(&mut buf);
    hex::encode(buf)
}

fn gen_code() -> String {
    let mut rng = rand::thread_rng();
    (0..10)
        .map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

fn hash_code(code: &str) -> String {
    let mut h = Sha256::new();
    h.update(code.trim().as_bytes());
    hex::encode(h.finalize())
}

// ---------- 视图 DTO ----------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantView {
    #[serde(flatten)]
    pub row: GrantRow,
    pub zones: Vec<String>,
    pub company_name: String,
    pub visitor_name: String,
    pub device_count: i64,
    /// 模拟设备上 ACL 是否仍在。
    pub acl_applied: bool,
    /// 结合当前时间推导出的实时状态（active 但已过到期时间 => expired_pending_sweep）。
    pub effective_state: String,
    pub now: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyView {
    pub id: i64,
    pub ts: i64,
    pub result: String,
    pub reason: String,
    pub mac: Option<String>,
    pub user_id: Option<String>,
    pub zone_id: Option<String>,
    pub code_last4: Option<String>,
    pub visitor_name: Option<String>,
    pub company_name: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub grant_id: String,
    pub mac: String,
    pub user_id: Option<String>,
    pub device_name: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub visitor_name: String,
    pub code_last4: String,
    pub device_limit: i64,
    pub grant_status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthDecision {
    pub allowed: bool,
    pub reason: String,
    pub anomaly: bool,
    pub grant_id: Option<String>,
}

// ================= 基础查询 =================

pub async fn list_companies(pool: &SqlitePool) -> AppResult<Vec<Company>> {
    Ok(sqlx::query_as("SELECT id, name FROM companies ORDER BY name")
        .fetch_all(pool)
        .await?)
}

pub async fn list_zones(pool: &SqlitePool) -> AppResult<Vec<Zone>> {
    Ok(sqlx::query_as("SELECT id, name, is_admin FROM zones ORDER BY is_admin, id")
        .fetch_all(pool)
        .await?)
}

pub async fn create_visitor(pool: &SqlitePool, req: &CreateVisitorReq, now: i64) -> AppResult<Visitor> {
    let company: Option<Company> = sqlx::query_as("SELECT id, name FROM companies WHERE id = ?1")
        .bind(&req.company_id)
        .fetch_optional(pool)
        .await?;
    if company.is_none() {
        return Err(AppError::BadRequest("来访公司不存在".into()));
    }
    let name = req.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("访客姓名不能为空".into()));
    }
    let id = new_id();
    sqlx::query(
        "INSERT INTO visitors (id, name, company_id, checked_in, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
    )
    .bind(&id)
    .bind(name)
    .bind(&req.company_id)
    .bind(now)
    .execute(pool)
    .await?;
    get_visitor(pool, &id).await
}

pub async fn get_visitor(pool: &SqlitePool, visitor_id: &str) -> AppResult<Visitor> {
    sqlx::query_as(
        "SELECT id, name, company_id, checked_in, check_in_at, check_out_at, created_at
         FROM visitors WHERE id = ?1",
    )
    .bind(visitor_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("访客不存在".into()))
}

// ================= 发码 =================

/// 校验发码参数；前台不得授权管理员区域，不得超过前台时长上限（防止开“长期权限”）。
fn validate_grant_request(
    role: Role,
    zones: &[Zone],
    zone_ids: &[String],
    device_limit: i64,
    duration_minutes: i64,
) -> AppResult<()> {
    if zone_ids.is_empty() {
        return Err(AppError::BadRequest("至少选择一个可用区域".into()));
    }
    for zid in zone_ids {
        let z = zones.iter().find(|z| &z.id == zid);
        let z = z.ok_or_else(|| AppError::BadRequest(format!("区域不存在: {zid}")))?;
        if z.is_admin == 1 && role == Role::FrontDesk {
            return Err(AppError::Forbidden(format!(
                "前台无权为「{}」等管理员区域开通网络权限", z.name
            )));
        }
    }
    if !(1..=MAX_DEVICE_LIMIT).contains(&device_limit) {
        return Err(AppError::BadRequest(format!(
            "设备数量须在 1..={MAX_DEVICE_LIMIT} 之间"
        )));
    }
    if duration_minutes <= 0 {
        return Err(AppError::BadRequest("有效时间必须大于 0 分钟".into()));
    }
    let max = match role {
        Role::FrontDesk => FRONT_DESK_MAX_DURATION_MIN,
        Role::NetworkAdmin => ADMIN_MAX_DURATION_MIN,
    };
    if duration_minutes > max {
        return Err(AppError::Forbidden(format!(
            "{}开通时长上限为 {} 分钟（{:.1} 小时），不允许开通长期权限",
            match role {
                Role::FrontDesk => "前台",
                Role::NetworkAdmin => "网络管理员",
            },
            max,
            max as f64 / 60.0
        )));
    }
    Ok(())
}

/// 让旧授权失效（重新发码/取消/离场的公共路径）。
/// - active 的旧码先尝试在网络设备上回收；
/// - 回收成功/设备上本无规则 → 进入给定终结态；
/// - 回收失败 → revoke_pending（intended_status 记录目标态），绝不当作已停用。
async fn retire_grant(
    pool: &SqlitePool,
    g: &GrantRow,
    target_status: &str,
    reason: &str,
    replaced_by: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let needs_device_revoke = g.status == "active" || g.status == "revoke_pending";
    if needs_device_revoke {
        match sim::revoke(pool, &g.id, now).await {
            Ok(()) => {
                sqlx::query(
                    "UPDATE grants SET status = ?2, intended_status = NULL, revoke_reason = ?3,
                        replaced_by = COALESCE(?4, replaced_by), ended_at = ?5, provision_error = NULL
                     WHERE id = ?1",
                )
                .bind(&g.id)
                .bind(target_status)
                .bind(reason)
                .bind(replaced_by)
                .bind(now)
                .execute(pool)
                .await?;
            }
            Err(e) => {
                sqlx::query(
                    "UPDATE grants SET status = 'revoke_pending', intended_status = ?2,
                        revoke_reason = ?3, replaced_by = COALESCE(?4, replaced_by),
                        provision_error = ?5
                     WHERE id = ?1",
                )
                .bind(&g.id)
                .bind(target_status)
                .bind(reason)
                .bind(replaced_by)
                .bind(e.message().to_string())
                .execute(pool)
                .await?;
                sim::notify(
                    pool, now, "admin", "revoke_failed",
                    &format!("授权 {} 回收失败：{}，需人工处理", tail4(&g.code_last4), e.message()),
                    Some(&g.id),
                )
                .await?;
            }
        }
    } else {
        // pending / provision_failed / 已终结：设备上没有有效规则，直接终结。
        sqlx::query(
            "UPDATE grants SET status = ?2, revoke_reason = ?3,
                replaced_by = COALESCE(?4, replaced_by), ended_at = COALESCE(ended_at, ?5)
             WHERE id = ?1",
        )
        .bind(&g.id)
        .bind(target_status)
        .bind(reason)
        .bind(replaced_by)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn tail4(last4: &str) -> String {
    format!("****{}", last4)
}

#[allow(clippy::too_many_arguments)]
pub async fn issue_grant(
    state: &AppState,
    visitor_id: &str,
    req: &IssueGrantReq,
    role: Role,
) -> AppResult<(GrantView, String)> {
    let pool = &state.pool;
    let now = state.clock.now_ms();

    let visitor = get_visitor(pool, visitor_id).await?;
    let zones = list_zones(pool).await?;
    validate_grant_request(
        role,
        &zones,
        &req.zone_ids,
        req.device_limit,
        req.duration_minutes,
    )?;

    let new_id = new_id();
    let code = gen_code();
    let code_hash = hash_code(&code);
    let last4: String = code.chars().skip(code.chars().count() - 4).collect();

    // 1) 旧码全部失效（会议取消/改期/重发语义由 revoke_grant 单独提供，重发必然作废旧码）。
    let old_grants = list_nonterminal_grants(pool, visitor_id).await?;
    for g in &old_grants {
        retire_grant(pool, g, "reissued", "reissued", Some(&new_id), now).await?;
    }

    // 2) 写入新授权。
    let role_str = if role == Role::FrontDesk { "front_desk" } else { "network_admin" };
    let mut tx: Transaction<'_, Sqlite> = pool.begin().await?;
    sqlx::query(
        "INSERT INTO grants (id, code_hash, code_last4, visitor_id, company_id, zone_ids,
            device_limit, duration_minutes, status, created_by_role, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,?10)",
    )
    .bind(&new_id)
    .bind(&code_hash)
    .bind(&last4)
    .bind(visitor_id)
    .bind(&visitor.company_id)
    .bind(serde_json::to_string(&req.zone_ids).unwrap())
    .bind(req.device_limit)
    .bind(req.duration_minutes)
    .bind(role_str)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    sim::notify(
        pool, now, "front_desk", "grant_issued",
        &format!("已为访客 {} 生成临时码 ****{}，等待签到生效", visitor.name, last4),
        Some(&new_id),
    )
    .await?;

    // 3) 已签到则立即激活（签到后才允许生效的补充分支）。
    if visitor.checked_in == 1 {
        activate_grant(pool, &new_id, now).await?;
    }

    let view = get_grant_view(pool, &new_id, now).await?;
    Ok((view, code))
}

/// 在网络设备上下发规则并把授权激活。下发失败必须显式呈现为 provision_failed。
pub async fn activate_grant(pool: &SqlitePool, grant_id: &str, now: i64) -> AppResult<()> {
    let g = get_grant_row(pool, grant_id).await?;
    if g.status == "active" {
        return Ok(());
    }
    if g.status != "pending" && g.status != "provision_failed" {
        return Err(AppError::Conflict(format!(
            "授权当前状态为 {}，无法激活", g.status
        )));
    }
    let visitor = get_visitor(pool, &g.visitor_id).await?;
    if visitor.checked_in != 1 {
        return Err(AppError::Conflict("访客尚未签到，临时码不能生效".into()));
    }
    match sim::provision(pool, grant_id, now).await {
        Ok(()) => {
            let expires_at = now + g.duration_minutes * 60_000;
            sqlx::query(
                "UPDATE grants SET status = 'active', valid_from = ?2, expires_at = ?3,
                    activated_at = COALESCE(activated_at, ?2), provision_error = NULL
                 WHERE id = ?1",
            )
            .bind(grant_id)
            .bind(now)
            .bind(expires_at)
            .execute(pool)
            .await?;
            sim::notify(
                pool, now, "visitor", "grant_active",
                &format!("临时码 ****{} 已开通，可使用至 {} 分钟后", g.code_last4, g.duration_minutes),
                Some(grant_id),
            )
            .await?;
        }
        Err(e) => {
            sqlx::query(
                "UPDATE grants SET status = 'provision_failed', provision_error = ?2 WHERE id = ?1",
            )
            .bind(grant_id)
            .bind(e.message().to_string())
            .execute(pool)
            .await?;
            sim::notify(
                pool, now, "front_desk", "provision_failed",
                &format!("临时码 ****{} 开通失败：{}，请重试或联系网管", g.code_last4, e.message()),
                Some(grant_id),
            )
            .await?;
            // 失败以正常返回携带状态，由调用方通过状态查询看到失败；不返回 500 让前端丢上下文。
        }
    }
    Ok(())
}

// ================= 签到 / 离场 =================

pub async fn check_in(state: &AppState, visitor_id: &str) -> AppResult<Visitor> {
    let pool = &state.pool;
    let now = state.clock.now_ms();
    let visitor = get_visitor(pool, visitor_id).await?;
    if visitor.checked_in == 1 {
        return Err(AppError::Conflict("访客已签到，请勿重复签到".into()));
    }
    sqlx::query("UPDATE visitors SET checked_in = 1, check_in_at = ?2 WHERE id = ?1")
        .bind(visitor_id)
        .bind(now)
        .execute(pool)
        .await?;

    // 签到后，所有待生效授权才允许下发。
    let pending: Vec<GrantRow> = sqlx::query_as(
        "SELECT * FROM grants WHERE visitor_id = ?1 AND status IN ('pending','provision_failed')",
    )
    .bind(visitor_id)
    .fetch_all(pool)
    .await?;
    for g in pending {
        activate_grant(pool, &g.id, now).await?;
    }
    get_visitor(pool, visitor_id).await
}

pub async fn check_out(state: &AppState, visitor_id: &str) -> AppResult<Visitor> {
    let pool = &state.pool;
    let now = state.clock.now_ms();
    let visitor = get_visitor(pool, visitor_id).await?;
    if visitor.checked_in == 0 && visitor.check_out_at.is_some() {
        return Err(AppError::Conflict("访客已离场".into()));
    }
    sqlx::query(
        "UPDATE visitors SET checked_in = 0, check_out_at = ?2 WHERE id = ?1",
    )
    .bind(visitor_id)
    .bind(now)
    .execute(pool)
    .await?;

    // 离场回收所有未终结授权（设备回收失败会进入 revoke_pending，不会被当作已停用）。
    let active = list_nonterminal_grants(pool, visitor_id).await?;
    for g in active {
        retire_grant(pool, &g, "checked_out", "checked_out", None, now).await?;
    }
    sim::notify(
        pool, now, "front_desk", "checked_out",
        &format!("访客 {} 已离场，其临时权限已回收（若设备侧失败将显示待处理）", visitor.name),
        Some(visitor_id),
    )
    .await?;
    get_visitor(pool, visitor_id).await
}

// ================= 手动回收（取消会议 / 改期） =================

pub async fn revoke_grant(
    state: &AppState,
    grant_id: &str,
    reason: &str,
) -> AppResult<GrantView> {
    let pool = &state.pool;
    let now = state.clock.now_ms();
    let g = get_grant_row(pool, grant_id).await?;
    let reason = match reason {
        "meeting_canceled" | "reschedule" | "manual" => reason,
        _ => "manual",
    };
    if matches!(g.status.as_str(), "expired" | "checked_out" | "revoked" | "reissued")
        && g.status != "revoke_pending"
    {
        return Err(AppError::Conflict(format!("授权已终结（{}）", g.status)));
    }
    retire_grant(pool, &g, "revoked", reason, None, now).await?;
    get_grant_view(pool, grant_id, now).await
}

/// 管理员对回收失败的授权重试设备回收。
pub async fn retry_revoke(state: &AppState, grant_id: &str) -> AppResult<GrantView> {
    let pool = &state.pool;
    let now = state.clock.now_ms();
    let g = get_grant_row(pool, grant_id).await?;
    if g.status != "revoke_pending" {
        return Err(AppError::Conflict("该授权不在待处理回收状态".into()));
    }
    let target = g.intended_status.clone().unwrap_or_else(|| "revoked".into());
    let reason = g.revoke_reason.clone().unwrap_or_else(|| "manual".into());
    retire_grant(pool, &g, &target, &reason, g.replaced_by.as_deref(), now).await?;
    get_grant_view(pool, grant_id, now).await
}

// ================= 到期扫描 =================

/// 扫描所有已到期但仍 active 的授权并回收。返回处理的 id 列表。
pub async fn sweep_expired(state: &AppState) -> AppResult<Vec<String>> {
    let pool = &state.pool;
    let now = state.clock.now_ms();
    let due: Vec<GrantRow> = sqlx::query_as(
        "SELECT * FROM grants WHERE status IN ('active','revoke_pending')
         AND expires_at IS NOT NULL AND expires_at <= ?1",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    let mut ids = Vec::new();
    for g in due {
        retire_grant(pool, &g, "expired", "expired", None, now).await?;
        ids.push(g.id);
    }
    Ok(ids)
}

// ================= 网络认证（模拟设备上报） =================

async fn log_event(
    pool: &SqlitePool,
    ts: i64,
    grant: Option<&GrantRow>,
    mac: &str,
    user_id: Option<&str>,
    zone_id: Option<&str>,
    result: &str,
    reason: &str,
    anomaly: bool,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO auth_events
            (ts, grant_id, visitor_id, code_last4, mac, user_id, zone_id, result, reason, anomaly)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )
    .bind(ts)
    .bind(grant.map(|g| g.id.clone()))
    .bind(grant.map(|g| g.visitor_id.clone()))
    .bind(grant.map(|g| g.code_last4.clone()))
    .bind(mac)
    .bind(user_id)
    .bind(zone_id)
    .bind(result)
    .bind(reason)
    .bind(anomaly as i64)
    .execute(pool)
    .await?;
    if anomaly {
        let who = grant
            .map(|g| format!("****{}", g.code_last4))
            .unwrap_or_else(|| "未知临时码".into());
        sim::notify(
            pool, ts, "admin", "anomaly",
            &format!("异常登录：{}（{}），MAC={}", reason, who, mac),
            grant.map(|g| g.id.as_str()),
        )
        .await?;
    }
    Ok(())
}

async fn deny(
    pool: &SqlitePool,
    ts: i64,
    grant: Option<&GrantRow>,
    req: &NetworkAuthReq,
    reason: &str,
    anomaly: bool,
) -> AuthDecision {
    let _ = log_event(
        pool, ts, grant, &req.mac, req.user_id.as_deref(), req.zone_id.as_deref(),
        "denied", reason, anomaly,
    )
    .await;
    AuthDecision {
        allowed: false,
        reason: reason.into(),
        anomaly,
        grant_id: grant.map(|g| g.id.clone()),
    }
}

pub async fn authenticate(state: &AppState, req: &NetworkAuthReq) -> AppResult<AuthDecision> {
    let pool = &state.pool;
    let now = state.clock.now_ms();
    let code = req.code.trim();
    if code.is_empty() || req.mac.trim().is_empty() {
        return Err(AppError::BadRequest("code 与 mac 不能为空".into()));
    }

    let grant: Option<GrantRow> =
        sqlx::query_as("SELECT * FROM grants WHERE code_hash = ?1")
            .bind(hash_code(code))
            .fetch_optional(pool)
            .await?;
    let Some(g) = grant else {
        return Ok(deny(pool, now, None, req, "unknown_code", true).await);
    };

    // 1) 状态门控
    match g.status.as_str() {
        "pending" => {
            return Ok(deny(pool, now, Some(&g), req, "not_checked_in", false).await);
        }
        "provision_failed" => {
            return Ok(deny(pool, now, Some(&g), req, "provision_failed", false).await);
        }
        "revoke_pending" => {
            // 回收尚未完成：绝不能放行，且属于需要关注的异常使用尝试。
            return Ok(deny(pool, now, Some(&g), req, "revoke_pending", true).await);
        }
        "expired" | "checked_out" | "revoked" | "reissued" => {
            return Ok(deny(pool, now, Some(&g), req, "already_revoked", true).await);
        }
        "active" => {}
        other => {
            return Ok(deny(pool, now, Some(&g), req, other, true).await);
        }
    }

    // 2) 时间边界：左闭右开。now == expires_at 即失效。
    if let Some(exp) = g.expires_at {
        if now >= exp {
            // 惰性回收：到期瞬间完成设备侧回收（失败则 revoke_pending）。
            retire_grant(pool, &g, "expired", "expired", None, now).await?;
            let g2 = get_grant_row(pool, &g.id).await?;
            return Ok(deny(pool, now, Some(&g2), req, "expired", false).await);
        }
    }

    // 3) 区域门控
    if let Some(zid) = &req.zone_id {
        if !g.zones().iter().any(|z| z == zid) {
            return Ok(deny(pool, now, Some(&g), req, "zone_forbidden", true).await);
        }
    }

    // 4) 多人共用拦截：同一临时码绑定唯一使用者标识。
    if let Some(uid) = &req.user_id {
        match &g.bound_user_id {
            Some(bound) if bound != uid => {
                return Ok(deny(pool, now, Some(&g), req, "shared_by_multiple_users", true).await);
            }
            None => {
                sqlx::query("UPDATE grants SET bound_user_id = ?2 WHERE id = ?1")
                    .bind(&g.id)
                    .bind(uid)
                    .execute(pool)
                    .await?;
            }
            _ => {}
        }
    }

    // 5) 设备数量限制（按 MAC 去重）。
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM devices WHERE grant_id = ?1")
        .bind(&g.id)
        .fetch_one(pool)
        .await?;
    let exists: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM devices WHERE grant_id = ?1 AND mac = ?2")
            .bind(&g.id)
            .bind(&req.mac)
            .fetch_one(pool)
            .await?;
    if exists.0 == 0 && count.0 >= g.device_limit {
        return Ok(deny(pool, now, Some(&g), req, "too_many_devices", true).await);
    }

    // 6) 放行并登记设备。
    if exists.0 == 0 {
        sqlx::query(
            "INSERT INTO devices (id, grant_id, mac, user_id, device_name, first_seen, last_seen)
             VALUES (?1,?2,?3,?4,?5,?6,?6)",
        )
        .bind(new_id())
        .bind(&g.id)
        .bind(&req.mac)
        .bind(req.user_id.as_deref())
        .bind(req.device_name.as_deref())
        .bind(now)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("UPDATE devices SET last_seen = ?3, user_id = COALESCE(user_id, ?2) WHERE grant_id = ?1 AND mac = ?4")
            .bind(&g.id)
            .bind(req.user_id.as_deref())
            .bind(now)
            .bind(&req.mac)
            .execute(pool)
            .await?;
    }
    log_event(pool, now, Some(&g), &req.mac, req.user_id.as_deref(), req.zone_id.as_deref(), "allowed", "ok", false).await?;
    Ok(AuthDecision {
        allowed: true,
        reason: "ok".into(),
        anomaly: false,
        grant_id: Some(g.id),
    })
}

// ================= 查询 =================

async fn list_nonterminal_grants(pool: &SqlitePool, visitor_id: &str) -> AppResult<Vec<GrantRow>> {
    Ok(sqlx::query_as(
        "SELECT * FROM grants
         WHERE visitor_id = ?1
           AND status IN ('pending','active','provision_failed','revoke_pending')
         ORDER BY created_at",
    )
    .bind(visitor_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_grant_row(pool: &SqlitePool, id: &str) -> AppResult<GrantRow> {
    sqlx::query_as("SELECT * FROM grants WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("授权不存在".into()))
}

pub async fn get_grant_view(pool: &SqlitePool, id: &str, now: i64) -> AppResult<GrantView> {
    let g = get_grant_row(pool, id).await?;
    grant_view_from_row(pool, g, now).await
}

async fn grant_view_from_row(pool: &SqlitePool, g: GrantRow, now: i64) -> AppResult<GrantView> {
    let company: Option<(String,)> = sqlx::query_as("SELECT name FROM companies WHERE id = ?1")
        .bind(&g.company_id)
        .fetch_optional(pool)
        .await?;
    let visitor: Option<(String,)> = sqlx::query_as("SELECT name FROM visitors WHERE id = ?1")
        .bind(&g.visitor_id)
        .fetch_optional(pool)
        .await?;
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM devices WHERE grant_id = ?1")
        .bind(&g.id)
        .fetch_one(pool)
        .await?;
    let acl = sim::acl_applied(pool, &g.id).await?;

    let effective_state = if g.status == "active" {
        match g.expires_at {
            Some(exp) if now >= exp => "expired_pending_sweep",
            _ => "active",
        }
    } else {
        g.status.as_str()
    }
    .to_string();

    Ok(GrantView {
        zones: g.zones(),
        company_name: company.map(|c| c.0).unwrap_or_default(),
        visitor_name: visitor.map(|v| v.0).unwrap_or_default(),
        device_count: count.0,
        acl_applied: acl,
        effective_state,
        now,
        row: g,
    })
}

pub async fn list_grants(pool: &SqlitePool, now: i64) -> AppResult<Vec<GrantView>> {
    let rows: Vec<GrantRow> = sqlx::query_as("SELECT * FROM grants ORDER BY created_at DESC")
        .fetch_all(pool)
        .await?;
    let mut out = Vec::new();
    for r in rows {
        out.push(grant_view_from_row(pool, r, now).await?);
    }
    Ok(out)
}

pub async fn list_anomalies(pool: &SqlitePool, limit: i64) -> AppResult<Vec<AnomalyView>> {
    Ok(sqlx::query_as(
        "SELECT e.id, e.ts, e.result, e.reason, e.mac, e.user_id, e.zone_id, e.code_last4,
                v.name AS visitor_name, c.name AS company_name
         FROM auth_events e
         LEFT JOIN visitors v ON v.id = e.visitor_id
         LEFT JOIN companies c ON c.id = (SELECT company_id FROM visitors WHERE id = e.visitor_id)
         WHERE e.anomaly = 1
         ORDER BY e.ts DESC, e.id DESC
         LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn list_all_events(pool: &SqlitePool, limit: i64) -> AppResult<Vec<AnomalyView>> {
    Ok(sqlx::query_as(
        "SELECT e.id, e.ts, e.result, e.reason, e.mac, e.user_id, e.zone_id, e.code_last4,
                v.name AS visitor_name, c.name AS company_name
         FROM auth_events e
         LEFT JOIN visitors v ON v.id = e.visitor_id
         LEFT JOIN companies c ON c.id = (SELECT company_id FROM visitors WHERE id = e.visitor_id)
         ORDER BY e.ts DESC, e.id DESC
         LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn list_devices(pool: &SqlitePool) -> AppResult<Vec<DeviceView>> {
    Ok(sqlx::query_as(
        "SELECT d.id, d.grant_id, d.mac, d.user_id, d.device_name, d.first_seen, d.last_seen,
                v.name AS visitor_name, g.code_last4, g.device_limit, g.status AS grant_status
         FROM devices d
         JOIN grants g ON g.id = d.grant_id
         JOIN visitors v ON v.id = g.visitor_id
         ORDER BY d.last_seen DESC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn list_pending_revokes(pool: &SqlitePool, now: i64) -> AppResult<Vec<GrantView>> {
    let rows: Vec<GrantRow> =
        sqlx::query_as("SELECT * FROM grants WHERE status = 'revoke_pending' ORDER BY created_at")
            .fetch_all(pool)
            .await?;
    let mut out = Vec::new();
    for r in rows {
        out.push(grant_view_from_row(pool, r, now).await?);
    }
    Ok(out)
}

pub async fn list_notifications(pool: &SqlitePool, limit: i64) -> AppResult<Vec<NotificationRow>> {
    Ok(sqlx::query_as(
        "SELECT id, ts, channel, kind, message, ref_id FROM notifications
         ORDER BY id DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}
