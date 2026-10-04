use crate::auth::Actor;
use crate::error::{bad_request, conflict, forbidden, not_found, AppResult};
use crate::models::{st, Auth, AuthView, Device, LoginEvent, Visit, VisitView, Zone};
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/me", get(me))
        .route("/api/zones", get(list_zones))
        .route("/api/visits", post(create_visit).get(list_visits))
        .route("/api/visits/:id", get(get_visit))
        .route("/api/visits/:id/checkin", post(checkin))
        .route("/api/visits/:id/checkout", post(checkout))
        .route("/api/visits/:id/cancel", post(cancel))
        .route("/api/visits/:id/reschedule", post(reschedule))
        .route("/api/visits/:id/reissue", post(reissue))
        .route("/api/auths/:id", get(auth_status))
        .route("/api/auths/:id/reveal", post(reveal_code))
        .route("/api/auths/:id/retry-revoke", post(retry_revoke))
        .route("/api/network/connect", post(connect))
        .route("/api/notifications", get(notifications))
        .route("/api/admin/events", get(admin_events))
        .route("/api/admin/anomalies", get(admin_anomalies))
        .route("/api/admin/devices", get(admin_devices))
        .route("/api/admin/network/mock", post(admin_network_mock))
        .route("/api/admin/network/state", get(admin_network_state))
        .route("/api/admin/time", post(admin_time))
        .route("/api/admin/sweep", post(admin_sweep))
        .fallback_service(
            ServeDir::new("frontend/dist")
                .not_found_service(ServeFile::new("frontend/dist/index.html")),
        )
        .with_state(state)
}

// ---------- 通用辅助 ----------

pub async fn notify(state: &AppState, audience: &str, message: &str) {
    tracing::info!("[通知 -> {audience}] {message}");
    let _ = sqlx::query("INSERT INTO notifications (audience, message, created_at) VALUES (?, ?, ?)")
        .bind(audience)
        .bind(message)
        .bind(state.now())
        .execute(&state.pool)
        .await;
}

fn gen_code() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    let part: String = (0..8)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect();
    format!("VST-{}-{}", &part[..4], &part[4..])
}

async fn load_visit(state: &AppState, id: i64) -> AppResult<Visit> {
    sqlx::query_as::<_, Visit>("SELECT * FROM visits WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| not_found(format!("来访记录 {id} 不存在")))
}

async fn load_auth(state: &AppState, id: i64) -> AppResult<Auth> {
    sqlx::query_as::<_, Auth>("SELECT * FROM authorizations WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| not_found(format!("授权 {id} 不存在")))
}

async fn auth_view(state: &AppState, a: &Auth) -> AppResult<AuthView> {
    let zones: Vec<(String,)> =
        sqlx::query_as("SELECT z.name FROM auth_zones az JOIN zones z ON z.id = az.zone_id WHERE az.auth_id = ? ORDER BY z.id")
            .bind(a.id)
            .fetch_all(&state.pool)
            .await?;
    let (device_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM devices WHERE auth_id = ?")
        .bind(a.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(AuthView {
        id: a.id,
        visit_id: a.visit_id,
        code_hint: a.code_hint.clone(),
        device_limit: a.device_limit,
        device_count,
        valid_from: a.valid_from,
        valid_to: a.valid_to,
        status: a.status.clone(),
        network_state: a.network_state.clone(),
        revoke_reason: a.revoke_reason.clone(),
        zones: zones.into_iter().map(|z| z.0).collect(),
        created_at: a.created_at,
    })
}

async fn visit_view(state: &AppState, v: &Visit) -> AppResult<VisitView> {
    let auths = sqlx::query_as::<_, Auth>("SELECT * FROM authorizations WHERE visit_id = ? ORDER BY id")
        .bind(v.id)
        .fetch_all(&state.pool)
        .await?;
    let mut views = Vec::new();
    for a in &auths {
        views.push(auth_view(state, a).await?);
    }
    Ok(VisitView {
        id: v.id,
        visitor_name: v.visitor_name.clone(),
        company: v.company.clone(),
        visitor_user_id: v.visitor_user_id,
        status: v.status.clone(),
        created_by: v.created_by,
        created_at: v.created_at,
        auths: views,
    })
}

async fn fetch_zones(state: &AppState, ids: &[i64]) -> AppResult<Vec<Zone>> {
    if ids.is_empty() {
        return Err(bad_request("至少选择一个可用区域"));
    }
    let mut out = Vec::new();
    for id in ids {
        let z = sqlx::query_as::<_, Zone>("SELECT * FROM zones WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| bad_request(format!("区域 {id} 不存在")))?;
        out.push(z);
    }
    Ok(out)
}

/// 有效时间窗口校验：valid_to 必须严格大于 valid_from 且大于当前时间
fn validate_window(state: &AppState, from: &DateTime<Utc>, to: &DateTime<Utc>) -> AppResult<()> {
    if to <= from {
        return Err(bad_request("有效结束时间必须晚于开始时间"));
    }
    if *to <= state.now() {
        return Err(bad_request("有效结束时间必须晚于当前时间"));
    }
    if *to - *from < Duration::minutes(5) {
        return Err(bad_request("有效时间至少 5 分钟"));
    }
    if *to - *from > Duration::days(30) {
        return Err(bad_request("访客授权最长 30 天"));
    }
    Ok(())
}

/// 越权规则：前台不能给自己开通管理区域权限，也不能给自己开超过 24 小时的长期权限
fn enforce_self_rules(actor: &Actor, visitor_user_id: Option<i64>, zones: &[Zone], from: &DateTime<Utc>, to: &DateTime<Utc>) -> AppResult<()> {
    if visitor_user_id != Some(actor.id) {
        return Ok(());
    }
    if zones.iter().any(|z| z.is_admin) {
        return Err(forbidden("越权操作：前台不能为自己开通管理区域权限"));
    }
    if *to - *from > Duration::hours(24) {
        return Err(forbidden("越权操作：前台不能为自己开通超过 24 小时的长期权限"));
    }
    Ok(())
}

async fn insert_auth(
    state: &AppState,
    visit_id: i64,
    zone_ids: &[i64],
    device_limit: i64,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> AppResult<(i64, String)> {
    let mut last_err = None;
    for _ in 0..5 {
        let code = gen_code();
        let hint = code[code.len() - 4..].to_string();
        let res = sqlx::query(
            "INSERT INTO authorizations (visit_id, code, code_hint, device_limit, valid_from, valid_to, status, network_state, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 'pending_checkin', 'pending', ?)",
        )
        .bind(visit_id)
        .bind(&code)
        .bind(&hint)
        .bind(device_limit)
        .bind(from)
        .bind(to)
        .bind(state.now())
        .execute(&state.pool)
        .await;
        match res {
            Ok(r) => {
                let id = r.last_insert_rowid();
                for z in zone_ids {
                    sqlx::query("INSERT INTO auth_zones (auth_id, zone_id) VALUES (?, ?)")
                        .bind(id)
                        .bind(z)
                        .execute(&state.pool)
                        .await?;
                }
                return Ok((id, code));
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap().into())
}

/// 回收一条授权：调用网络设备回收权限。
/// 成功 -> status=target；失败 -> status=revoke_failed 且 network_state 保持 applied（权限仍在设备上，必须可见）。
pub async fn revoke_auth(state: &AppState, auth_id: i64, target: &str, reason: &str) -> AppResult<()> {
    let a = load_auth(state, auth_id).await?;
    if st::is_terminal(&a.status) && a.network_state != "applied" {
        return Ok(()); // 已终止且网络侧已回收
    }
    if a.network_state == "applied" {
        match state.network.revoke(a.id).await {
            Ok(()) => {
                sqlx::query("UPDATE authorizations SET status = ?, network_state = 'revoked', target_status = NULL, revoke_reason = ? WHERE id = ?")
                    .bind(target)
                    .bind(reason)
                    .bind(a.id)
                    .execute(&state.pool)
                    .await?;
                notify(state, "front_desk", &format!("授权 #{} 已回收（{reason}）", a.id)).await;
            }
            Err(e) => {
                sqlx::query("UPDATE authorizations SET status = 'revoke_failed', target_status = ?, revoke_reason = ? WHERE id = ?")
                    .bind(target)
                    .bind(reason)
                    .bind(a.id)
                    .execute(&state.pool)
                    .await?;
                notify(state, "network_admin", &format!("⚠️ 授权 #{} 网络回收失败：{e}（{reason}），权限仍在设备上", a.id)).await;
                notify(state, "front_desk", &format!("授权 #{} 回收失败，待处理", a.id)).await;
            }
        }
    } else {
        // 从未下发到设备，直接落终态
        sqlx::query("UPDATE authorizations SET status = ?, target_status = NULL, revoke_reason = ? WHERE id = ?")
            .bind(target)
            .bind(reason)
            .bind(a.id)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

/// 回收一次来访下所有未终止的授权
async fn revoke_all(state: &AppState, visit_id: i64, target: &str, reason: &str) -> AppResult<()> {
    let auths = sqlx::query_as::<_, Auth>(
        "SELECT * FROM authorizations WHERE visit_id = ? AND status IN ('pending_checkin', 'active', 'revoke_failed')",
    )
    .bind(visit_id)
    .fetch_all(&state.pool)
    .await?;
    for a in auths {
        revoke_auth(state, a.id, target, reason).await?;
    }
    Ok(())
}

// ---------- 基础接口 ----------

async fn me(actor: Actor) -> Json<Value> {
    Json(json!({ "id": actor.id, "username": actor.username, "role": actor.role }))
}

async fn list_zones(State(s): State<AppState>, _actor: Actor) -> AppResult<Json<Vec<Zone>>> {
    let zones = sqlx::query_as::<_, Zone>("SELECT * FROM zones ORDER BY id")
        .fetch_all(&s.pool)
        .await?;
    Ok(Json(zones))
}

// ---------- 来访与授权 ----------

#[derive(Deserialize)]
struct CreateVisit {
    visitor_name: String,
    company: String,
    /// 若访客是系统用户本人（如前台给自己开），传其用户 id
    visitor_user_id: Option<i64>,
    zone_ids: Vec<i64>,
    device_limit: i64,
    valid_from: DateTime<Utc>,
    valid_to: DateTime<Utc>,
}

async fn create_visit(
    State(s): State<AppState>,
    actor: Actor,
    Json(req): Json<CreateVisit>,
) -> AppResult<(StatusCode, Json<VisitView>)> {
    actor.require("front_desk")?;
    if req.visitor_name.trim().is_empty() || req.company.trim().is_empty() {
        return Err(bad_request("访客姓名与来访公司不能为空"));
    }
    if !(1..=10).contains(&req.device_limit) {
        return Err(bad_request("设备数量需在 1-10 之间"));
    }
    validate_window(&s, &req.valid_from, &req.valid_to)?;
    let zones = fetch_zones(&s, &req.zone_ids).await?;
    enforce_self_rules(&actor, req.visitor_user_id, &zones, &req.valid_from, &req.valid_to)?;

    let now = s.now();
    let res = sqlx::query(
        "INSERT INTO visits (visitor_name, company, visitor_user_id, status, created_by, created_at) VALUES (?, ?, ?, 'booked', ?, ?)",
    )
    .bind(req.visitor_name.trim())
    .bind(req.company.trim())
    .bind(req.visitor_user_id)
    .bind(actor.id)
    .bind(now)
    .execute(&s.pool)
    .await?;
    let visit_id = res.last_insert_rowid();
    let (auth_id, _) = insert_auth(&s, visit_id, &req.zone_ids, req.device_limit, req.valid_from, req.valid_to).await?;
    notify(&s, "front_desk", &format!("授权 #{auth_id} 已创建，访客签到后生效")).await;

    let v = load_visit(&s, visit_id).await?;
    Ok((StatusCode::CREATED, Json(visit_view(&s, &v).await?)))
}

async fn list_visits(State(s): State<AppState>, _actor: Actor) -> AppResult<Json<Vec<VisitView>>> {
    let visits = sqlx::query_as::<_, Visit>("SELECT * FROM visits ORDER BY id DESC LIMIT 100")
        .fetch_all(&s.pool)
        .await?;
    let mut out = Vec::new();
    for v in &visits {
        out.push(visit_view(&s, v).await?);
    }
    Ok(Json(out))
}

async fn get_visit(State(s): State<AppState>, _actor: Actor, Path(id): Path<i64>) -> AppResult<Json<VisitView>> {
    let v = load_visit(&s, id).await?;
    Ok(Json(visit_view(&s, &v).await?))
}

async fn checkin(State(s): State<AppState>, actor: Actor, Path(id): Path<i64>) -> AppResult<Json<VisitView>> {
    actor.require("front_desk")?;
    let v = load_visit(&s, id).await?;
    if v.status != "booked" {
        return Err(conflict(format!("当前状态 {} 不允许签到", v.status)));
    }
    let now = s.now();
    let pendings = sqlx::query_as::<_, Auth>(
        "SELECT * FROM authorizations WHERE visit_id = ? AND status = 'pending_checkin'",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    for a in pendings {
        if now >= a.valid_to {
            revoke_auth(&s, a.id, st::EXPIRED, "到期自动回收").await?;
            continue;
        }
        s.network.apply(a.id, &a.code).await.map_err(crate::error::AppError::Internal)?;
        sqlx::query("UPDATE authorizations SET status = 'active', network_state = 'applied' WHERE id = ?")
            .bind(a.id)
            .execute(&s.pool)
            .await?;
        notify(&s, "front_desk", &format!("授权 #{} 已开通生效", a.id)).await;
    }
    sqlx::query("UPDATE visits SET status = 'checked_in' WHERE id = ?")
        .bind(id)
        .execute(&s.pool)
        .await?;
    let v = load_visit(&s, id).await?;
    Ok(Json(visit_view(&s, &v).await?))
}

async fn checkout(State(s): State<AppState>, actor: Actor, Path(id): Path<i64>) -> AppResult<Json<VisitView>> {
    actor.require("front_desk")?;
    let v = load_visit(&s, id).await?;
    if v.status != "checked_in" {
        return Err(conflict(format!("当前状态 {} 不允许离场登记", v.status)));
    }
    revoke_all(&s, id, st::REVOKED, "访客离场回收").await?;
    sqlx::query("UPDATE visits SET status = 'checked_out' WHERE id = ?")
        .bind(id)
        .execute(&s.pool)
        .await?;
    let v = load_visit(&s, id).await?;
    Ok(Json(visit_view(&s, &v).await?))
}

async fn cancel(State(s): State<AppState>, actor: Actor, Path(id): Path<i64>) -> AppResult<Json<VisitView>> {
    actor.require("front_desk")?;
    let v = load_visit(&s, id).await?;
    if v.status != "booked" && v.status != "checked_in" {
        return Err(conflict(format!("当前状态 {} 不允许取消", v.status)));
    }
    revoke_all(&s, id, st::REVOKED, "会议取消").await?;
    sqlx::query("UPDATE visits SET status = 'cancelled' WHERE id = ?")
        .bind(id)
        .execute(&s.pool)
        .await?;
    let v = load_visit(&s, id).await?;
    Ok(Json(visit_view(&s, &v).await?))
}

#[derive(Deserialize)]
struct Reschedule {
    valid_from: DateTime<Utc>,
    valid_to: DateTime<Utc>,
}

async fn reschedule(
    State(s): State<AppState>,
    actor: Actor,
    Path(id): Path<i64>,
    Json(req): Json<Reschedule>,
) -> AppResult<Json<VisitView>> {
    actor.require("front_desk")?;
    let v = load_visit(&s, id).await?;
    if v.status != "booked" && v.status != "checked_in" {
        return Err(conflict(format!("当前状态 {} 不允许改期", v.status)));
    }
    let latest = latest_auth(&s, id).await?;
    let zones = auth_zone_ids(&s, latest.id).await?;
    replace_auth(&s, &actor, &v, zones, latest.device_limit, req.valid_from, req.valid_to, "来访改期，旧码失效").await?;
    let v = load_visit(&s, id).await?;
    Ok(Json(visit_view(&s, &v).await?))
}

#[derive(Deserialize)]
struct Reissue {
    device_limit: Option<i64>,
    zone_ids: Option<Vec<i64>>,
    valid_from: Option<DateTime<Utc>>,
    valid_to: Option<DateTime<Utc>>,
}

async fn reissue(
    State(s): State<AppState>,
    actor: Actor,
    Path(id): Path<i64>,
    Json(req): Json<Reissue>,
) -> AppResult<Json<VisitView>> {
    actor.require("front_desk")?;
    let v = load_visit(&s, id).await?;
    if v.status != "booked" && v.status != "checked_in" {
        return Err(conflict(format!("当前状态 {} 不允许重新发码", v.status)));
    }
    let latest = latest_auth(&s, id).await?;
    let zone_ids = match &req.zone_ids {
        Some(z) => z.clone(),
        None => auth_zone_ids(&s, latest.id).await?,
    };
    let device_limit = req.device_limit.unwrap_or(latest.device_limit);
    if !(1..=10).contains(&device_limit) {
        return Err(bad_request("设备数量需在 1-10 之间"));
    }
    let (from, to) = match (req.valid_from, req.valid_to) {
        (Some(f), Some(t)) => (f, t),
        (None, None) => (latest.valid_from, latest.valid_to),
        _ => return Err(bad_request("valid_from 与 valid_to 必须同时提供")),
    };
    replace_auth(&s, &actor, &v, zone_ids, device_limit, from, to, "重新发码，旧码失效").await?;
    let v = load_visit(&s, id).await?;
    Ok(Json(visit_view(&s, &v).await?))
}

async fn latest_auth(state: &AppState, visit_id: i64) -> AppResult<Auth> {
    sqlx::query_as::<_, Auth>("SELECT * FROM authorizations WHERE visit_id = ? ORDER BY id DESC LIMIT 1")
        .bind(visit_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| not_found("该来访没有授权记录"))
}

async fn auth_zone_ids(state: &AppState, auth_id: i64) -> AppResult<Vec<i64>> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT zone_id FROM auth_zones WHERE auth_id = ? ORDER BY zone_id")
        .bind(auth_id)
        .fetch_all(&state.pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// 使旧码失效并签发新授权（改期 / 重新发码共用）
async fn replace_auth(
    s: &AppState,
    actor: &Actor,
    visit: &Visit,
    zone_ids: Vec<i64>,
    device_limit: i64,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    reason: &str,
) -> AppResult<i64> {
    validate_window(s, &from, &to)?;
    let zones = fetch_zones(s, &zone_ids).await?;
    enforce_self_rules(actor, visit.visitor_user_id, &zones, &from, &to)?;
    revoke_all(s, visit.id, st::SUPERSEDED, reason).await?;
    let (new_id, code) = insert_auth(s, visit.id, &zone_ids, device_limit, from, to).await?;
    if visit.status == "checked_in" {
        s.network.apply(new_id, &code).await.map_err(crate::error::AppError::Internal)?;
        sqlx::query("UPDATE authorizations SET status = 'active', network_state = 'applied' WHERE id = ?")
            .bind(new_id)
            .execute(&s.pool)
            .await?;
    }
    notify(s, "front_desk", &format!("授权 #{new_id} 已签发（{reason}）")).await;
    Ok(new_id)
}

// ---------- 授权查询 ----------

async fn auth_status(State(s): State<AppState>, _actor: Actor, Path(id): Path<i64>) -> AppResult<Json<AuthView>> {
    let a = load_auth(&s, id).await?;
    Ok(Json(auth_view(&s, &a).await?))
}

/// 默认不下发完整临时码，前台显式查看时才返回
async fn reveal_code(State(s): State<AppState>, actor: Actor, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    actor.require("front_desk")?;
    let a = load_auth(&s, id).await?;
    Ok(Json(json!({ "id": a.id, "code": a.code })))
}

async fn retry_revoke(State(s): State<AppState>, _actor: Actor, Path(id): Path<i64>) -> AppResult<Json<AuthView>> {
    let a = load_auth(&s, id).await?;
    if a.status != st::REVOKE_FAILED {
        return Err(conflict("该授权不处于回收失败状态"));
    }
    let target = a.target_status.clone().unwrap_or_else(|| st::REVOKED.to_string());
    let reason = a.revoke_reason.clone().unwrap_or_else(|| "手动重试回收".to_string());
    revoke_auth(&s, id, &target, &format!("{reason}（重试）")).await?;
    let a = load_auth(&s, id).await?;
    Ok(Json(auth_view(&s, &a).await?))
}

// ---------- 模拟网络设备回传：设备尝试上网 ----------

#[derive(Deserialize)]
struct ConnectReq {
    code: String,
    mac: String,
    zone_id: i64,
}

async fn connect(State(s): State<AppState>, Json(req): Json<ConnectReq>) -> AppResult<Json<Value>> {
    let now = s.now();
    let auth = sqlx::query_as::<_, Auth>("SELECT * FROM authorizations WHERE code = ?")
        .bind(&req.code)
        .fetch_optional(&s.pool)
        .await?;

    let (result, detail, flagged, auth_id): (&str, Option<String>, bool, Option<i64>) = match auth {
        None => ("denied_unknown", Some("临时码不存在".into()), false, None),
        Some(a) => {
            let aid = Some(a.id);
            // 时间边界精确判断：有效期为 [valid_from, valid_to)，到达 valid_to 即过期
            if (a.status == st::ACTIVE || a.status == st::PENDING) && now >= a.valid_to {
                revoke_auth(&s, a.id, st::EXPIRED, "到期自动回收").await?;
                ("denied_expired", Some("临时码已过期".into()), false, aid)
            } else if a.status == st::PENDING {
                ("denied_not_active", Some("访客未签到，权限未生效".into()), false, aid)
            } else if a.status == st::SUPERSEDED {
                ("denied_superseded", Some("旧码已被重新发码/改期取代".into()), true, aid)
            } else if a.status == st::REVOKED || a.status == st::EXPIRED {
                ("denied_revoked", Some("临时码已停用".into()), true, aid)
            } else if a.status == st::REVOKE_FAILED {
                // 回收失败：设备上权限可能仍在，门户侧拦截并告警
                ("denied_revoke_pending", Some("该码回收待处理，已拦截并通知管理员".into()), true, aid)
            } else if now < a.valid_from {
                ("denied_not_yet", Some("未到生效时间".into()), false, aid)
            } else {
                let in_zone: Option<(i64,)> = sqlx::query_as(
                    "SELECT 1 FROM auth_zones WHERE auth_id = ? AND zone_id = ?",
                )
                .bind(a.id)
                .bind(req.zone_id)
                .fetch_optional(&s.pool)
                .await?;
                if in_zone.is_none() {
                    ("denied_zone", Some("该区域不在授权范围内".into()), true, aid)
                } else {
                    let existing: Option<(i64,)> = sqlx::query_as(
                        "SELECT id FROM devices WHERE auth_id = ? AND mac = ?",
                    )
                    .bind(a.id)
                    .bind(&req.mac)
                    .fetch_optional(&s.pool)
                    .await?;
                    if existing.is_some() {
                        sqlx::query("UPDATE devices SET last_seen = ? WHERE auth_id = ? AND mac = ?")
                            .bind(now)
                            .bind(a.id)
                            .bind(&req.mac)
                            .execute(&s.pool)
                            .await?;
                        ("ok", None, false, aid)
                    } else {
                        let (cnt,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM devices WHERE auth_id = ?")
                            .bind(a.id)
                            .fetch_one(&s.pool)
                            .await?;
                        if cnt >= a.device_limit {
                            (
                                "denied_device_limit",
                                Some(format!("设备数量超过限制（{}/{}），疑似多人共用", cnt + 1, a.device_limit)),
                                true,
                                aid,
                            )
                        } else {
                            sqlx::query("INSERT INTO devices (auth_id, mac, first_seen, last_seen) VALUES (?, ?, ?, ?)")
                                .bind(a.id)
                                .bind(&req.mac)
                                .bind(now)
                                .bind(now)
                                .execute(&s.pool)
                                .await?;
                            ("ok", None, false, aid)
                        }
                    }
                }
            }
        }
    };

    sqlx::query(
        "INSERT INTO login_events (auth_id, code_used, mac, zone_id, result, detail, flagged, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(auth_id)
    .bind(&req.code)
    .bind(&req.mac)
    .bind(req.zone_id)
    .bind(result)
    .bind(&detail)
    .bind(flagged)
    .bind(now)
    .execute(&s.pool)
    .await?;

    if flagged {
        notify(
            &s,
            "network_admin",
            &format!("异常登录告警：码 {}… 设备 {} 结果 {}（{}）",
                &req.code.chars().take(8).collect::<String>(),
                req.mac,
                result,
                detail.clone().unwrap_or_default()),
        )
        .await;
    }

    Ok(Json(json!({
        "allow": result == "ok",
        "result": result,
        "detail": detail,
    })))
}

// ---------- 通知 ----------

async fn notifications(State(s): State<AppState>, actor: Actor) -> AppResult<Json<Value>> {
    let rows: Vec<(i64, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, message, created_at FROM notifications WHERE audience = ? ORDER BY id DESC LIMIT 50",
    )
    .bind(&actor.role)
    .fetch_all(&s.pool)
    .await?;
    let list: Vec<Value> = rows
        .into_iter()
        .map(|(id, message, created_at)| json!({ "id": id, "message": message, "created_at": created_at }))
        .collect();
    Ok(Json(json!(list)))
}

// ---------- 网络管理员 ----------

async fn admin_events(State(s): State<AppState>, actor: Actor) -> AppResult<Json<Vec<LoginEvent>>> {
    actor.require("network_admin")?;
    let events = sqlx::query_as::<_, LoginEvent>("SELECT * FROM login_events ORDER BY id DESC LIMIT 200")
        .fetch_all(&s.pool)
        .await?;
    Ok(Json(events))
}

async fn admin_anomalies(State(s): State<AppState>, actor: Actor) -> AppResult<Json<Vec<LoginEvent>>> {
    actor.require("network_admin")?;
    let events = sqlx::query_as::<_, LoginEvent>(
        "SELECT * FROM login_events WHERE flagged = 1 ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(events))
}

async fn admin_devices(State(s): State<AppState>, actor: Actor) -> AppResult<Json<Vec<Device>>> {
    actor.require("network_admin")?;
    let devices = sqlx::query_as::<_, Device>("SELECT * FROM devices ORDER BY last_seen DESC LIMIT 200")
        .fetch_all(&s.pool)
        .await?;
    Ok(Json(devices))
}

#[derive(Deserialize)]
struct MockReq {
    fail_revoke: bool,
}

async fn admin_network_mock(State(s): State<AppState>, actor: Actor, Json(req): Json<MockReq>) -> AppResult<Json<Value>> {
    actor.require("network_admin")?;
    s.network.set_fail_revoke(req.fail_revoke);
    Ok(Json(json!({ "fail_revoke": req.fail_revoke })))
}

async fn admin_network_state(State(s): State<AppState>, actor: Actor) -> AppResult<Json<Value>> {
    actor.require("network_admin")?;
    let applied: Vec<Value> = s
        .network
        .applied_list()
        .into_iter()
        .map(|(id, code)| json!({ "auth_id": id, "code": code }))
        .collect();
    Ok(Json(json!({
        "fail_revoke": s.network.fail_revoke(),
        "applied": applied,
    })))
}

#[derive(Deserialize)]
struct TimeReq {
    offset_secs: Option<i64>,
    frozen: Option<DateTime<Utc>>,
    unfreeze: Option<bool>,
}

/// 演示用时间控制：冻结时间或设置偏移
async fn admin_time(State(s): State<AppState>, actor: Actor, Json(req): Json<TimeReq>) -> AppResult<Json<Value>> {
    actor.require("network_admin")?;
    {
        let mut c = s.clock.lock().unwrap();
        if let Some(t) = req.frozen {
            c.frozen = Some(t);
        }
        if req.unfreeze == Some(true) {
            c.frozen = None;
        }
        if let Some(off) = req.offset_secs {
            c.offset_secs = off;
        }
    }
    Ok(Json(json!({ "now": s.now() })))
}

async fn admin_sweep(State(s): State<AppState>, actor: Actor) -> AppResult<Json<Value>> {
    actor.require("network_admin")?;
    let (expired, retried) = crate::sweeper::sweep_once(&s).await;
    Ok(Json(json!({ "expired": expired, "retried": retried, "now": s.now() })))
}
