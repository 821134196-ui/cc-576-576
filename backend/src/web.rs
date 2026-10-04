use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::service::{self, Role};
use crate::sim;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/meta", get(meta))
        .route("/api/visitors", get(list_visitors).post(create_visitor))
        .route("/api/visitors/:id", get(get_visitor_detail))
        .route("/api/visitors/:id/check-in", post(check_in))
        .route("/api/visitors/:id/check-out", post(check_out))
        .route("/api/visitors/:id/grants", post(issue_grant))
        .route("/api/grants", get(list_grants))
        .route("/api/grants/:id", get(get_grant))
        .route("/api/grants/:id/revoke", post(revoke_grant))
        .route("/api/grants/:id/activate", post(activate))
        .route("/api/grants/:id/retry-revoke", post(retry_revoke))
        .route("/api/network/auth", post(network_auth))
        .route("/api/admin/anomalies", get(list_anomalies))
        .route("/api/admin/events", get(list_events))
        .route("/api/admin/devices", get(list_devices))
        .route("/api/admin/pending-revokes", get(list_pending_revokes))
        .route("/api/admin/sweep", post(sweep))
        .route("/api/notifications", get(list_notifications))
        .route("/api/sim/faults", post(set_fault))
        .with_state(state)
}

fn role(headers: &HeaderMap) -> AppResult<Role> {
    Role::parse(headers.get("x-role").and_then(|v| v.to_str().ok()))
}

fn require_admin(headers: &HeaderMap) -> AppResult<Role> {
    let r = role(headers)?;
    Role::require_admin(r)?;
    Ok(r)
}

async fn meta(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let companies = service::list_companies(&s.pool).await?;
    let zones = service::list_zones(&s.pool).await?;
    Ok(Json(json!({ "companies": companies, "zones": zones, "now": s.clock.now_ms() })))
}

async fn create_visitor(
    State(s): State<AppState>,
    Json(req): Json<CreateVisitorReq>,
) -> AppResult<Json<Value>> {
    let v = service::create_visitor(&s.pool, &req, s.clock.now_ms()).await?;
    Ok(Json(json!({ "visitor": v })))
}

async fn list_visitors(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<Visitor> = sqlx::query_as(
        "SELECT id, name, company_id, checked_in, check_in_at, check_out_at, created_at
         FROM visitors ORDER BY created_at DESC",
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(json!({ "visitors": rows })))
}

async fn visitor_detail_json(s: &AppState, id: &str) -> AppResult<Value> {
    let v = service::get_visitor(&s.pool, id).await?;
    let grants = service::list_grants(&s.pool, s.clock.now_ms()).await?;
    let grants: Vec<_> = grants.into_iter().filter(|g| g.row.visitor_id == id).collect();
    Ok(json!({ "visitor": v, "grants": grants, "now": s.clock.now_ms() }))
}

async fn get_visitor_detail(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    Ok(Json(visitor_detail_json(&s, &id).await?))
}

async fn check_in(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    service::check_in(&s, &id).await?;
    Ok(Json(visitor_detail_json(&s, &id).await?))
}

async fn check_out(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    service::check_out(&s, &id).await?;
    Ok(Json(visitor_detail_json(&s, &id).await?))
}

async fn issue_grant(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<IssueGrantReq>,
) -> AppResult<Json<Value>> {
    let r = role(&headers)?;
    let (view, code) = service::issue_grant(&s, &id, &req, r).await?;
    // 完整临时码仅在创建响应中返回一次，数据库只存哈希与后 4 位。
    Ok(Json(json!({ "grant": view, "code": code })))
}

async fn list_grants(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let grants = service::list_grants(&s.pool, s.clock.now_ms()).await?;
    Ok(Json(json!({ "grants": grants, "now": s.clock.now_ms() })))
}

async fn get_grant(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let g = service::get_grant_view(&s.pool, &id, s.clock.now_ms()).await?;
    Ok(Json(json!({ "grant": g, "now": s.clock.now_ms() })))
}

async fn revoke_grant(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<RevokeReq>,
) -> AppResult<Json<Value>> {
    // 前台与管理员均可手动回收（会议取消/改期）。
    let _ = role(&headers)?;
    let g = service::revoke_grant(
        &s,
        &id,
        req.reason.as_deref().unwrap_or("manual"),
    )
    .await?;
    Ok(Json(json!({ "grant": g })))
}

async fn activate(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    service::activate_grant(&s.pool, &id, s.clock.now_ms()).await?;
    let g = service::get_grant_view(&s.pool, &id, s.clock.now_ms()).await?;
    Ok(Json(json!({ "grant": g })))
}

async fn retry_revoke(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    let g = service::retry_revoke(&s, &id).await?;
    Ok(Json(json!({ "grant": g })))
}

async fn network_auth(
    State(s): State<AppState>,
    Json(req): Json<NetworkAuthReq>,
) -> AppResult<Json<Value>> {
    let decision = service::authenticate(&s, &req).await?;
    Ok(Json(json!(decision)))
}

async fn list_anomalies(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    let items = service::list_anomalies(&s.pool, 200).await?;
    Ok(Json(json!({ "anomalies": items })))
}

async fn list_events(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    let items = service::list_all_events(&s.pool, 200).await?;
    Ok(Json(json!({ "events": items })))
}

async fn list_devices(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    let items = service::list_devices(&s.pool).await?;
    Ok(Json(json!({ "devices": items })))
}

async fn list_pending_revokes(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    let items = service::list_pending_revokes(&s.pool, s.clock.now_ms()).await?;
    Ok(Json(json!({ "pendingRevokes": items })))
}

async fn sweep(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    let ids = service::sweep_expired(&s).await?;
    Ok(Json(json!({ "retired": ids })))
}

async fn list_notifications(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let items = service::list_notifications(&s.pool, 100).await?;
    Ok(Json(json!({ "notifications": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FaultReq {
    kind: String,
    armed: bool,
}

async fn set_fault(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<FaultReq>,
) -> AppResult<Json<Value>> {
    let _ = require_admin(&headers)?;
    if req.kind != "provision" && req.kind != "revoke" {
        return Err(AppError::BadRequest("kind 必须是 provision | revoke".into()));
    }
    sim::set_fault(&s.pool, &req.kind, req.armed).await?;
    Ok(Json(json!({ "ok": true, "kind": req.kind, "armed": req.armed })))
}
