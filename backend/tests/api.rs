//! 集成测试：在内存 SQLite + 可控时钟上端到端验证 HTTP API。
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use guestnet::clock::FakeClock;
use guestnet::{build_app, test_pool};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const T0: i64 = 1_700_000_000_000; // 固定基准时间（毫秒）

struct Harness {
    app: axum::Router,
    clock: FakeClock,
}

impl Harness {
    async fn new() -> Self {
        let pool = test_pool().await;
        let clock = FakeClock::new(T0);
        let app = build_app(pool, Arc::new(clock.clone()));
        Self { app, clock }
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        role: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(r) = role {
            b = b.header("x-role", r);
        }
        let req = match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let val = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, val)
    }

    async fn create_visitor(&self, name: &str) -> String {
        let (s, v) = self
            .call(
                Method::POST,
                "/api/visitors",
                None,
                Some(json!({"name": name, "companyId": "co-acme"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["visitor"]["id"].as_str().unwrap().to_string()
    }

    async fn issue(
        &self,
        visitor: &str,
        role: &str,
        zones: &[&str],
        limit: i64,
        minutes: i64,
    ) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            &format!("/api/visitors/{visitor}/grants"),
            Some(role),
            Some(json!({
                "zoneIds": zones,
                "deviceLimit": limit,
                "durationMinutes": minutes,
            })),
        )
        .await
    }

    async fn auth(&self, code: &str, mac: &str, user: Option<&str>, zone: Option<&str>) -> Value {
        let (s, v) = self
            .call(
                Method::POST,
                "/api/network/auth",
                None,
                Some(json!({
                    "code": code, "mac": mac,
                    "userId": user, "zoneId": zone,
                })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }
}

fn granted_code(v: &Value) -> String {
    v["code"].as_str().expect("创建响应应一次性返回完整临时码").to_string()
}

#[tokio::test]
async fn not_active_before_checkin_and_activates_after() {
    let h = Harness::new().await;
    let vid = h.create_visitor("王小明").await;

    // 发码时未签到 → pending，网络认证被拦截
    let (s, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 120).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = granted_code(&v);
    assert_eq!(v["grant"]["status"], "pending");
    assert_eq!(v["grant"]["effectiveState"], "pending");

    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "not_checked_in");

    // 签到后才允许生效
    let (s, v) = h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let grant = v["grants"][0].clone();
    assert_eq!(grant["status"], "active");
    assert_eq!(grant["effectiveState"], "active");

    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], true, "签到后应放行: {d}");
    assert_eq!(d["reason"], "ok");
}

#[tokio::test]
async fn exact_expiry_boundary_is_half_open() {
    let h = Harness::new().await;
    let vid = h.create_visitor("李到期").await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 60).await;
    let code = granted_code(&v);
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;

    // expires_at - 1ms：仍有效
    h.clock.set(T0 + 60 * 60_000 - 1);
    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], true, "到期前一毫秒应有效: {d}");

    // now == expires_at：立即失效（左闭右开），并触发惰性回收
    h.clock.set(T0 + 60 * 60_000);
    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "expired");

    let gid = d["grantId"].as_str().unwrap();
    let (_, g) = h.call(Method::GET, &format!("/api/grants/{gid}"), None, None).await;
    assert_eq!(g["grant"]["status"], "expired");
    assert_eq!(g["grant"]["aclApplied"], false, "到期后设备侧 ACL 应已回收");
}

#[tokio::test]
async fn check_out_revokes_access() {
    let h = Harness::new().await;
    let vid = h.create_visitor("赵离场").await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 240).await;
    let code = granted_code(&v);
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], true);

    // 离场 → 授权终结，设备规则回收
    let (s, v) = h.call(Method::POST, &format!("/api/visitors/{vid}/check-out"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["grants"][0]["status"], "checked_out");
    assert_eq!(v["grants"][0]["aclApplied"], false);

    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "already_revoked");

    // 前台查询授权状态可见已回收成功
    let gid = v["grants"][0]["id"].as_str().unwrap();
    let (_, g) = h.call(Method::GET, &format!("/api/grants/{gid}"), None, None).await;
    assert_eq!(g["grant"]["effectiveState"], "checked_out");
}

#[tokio::test]
async fn reissue_invalidates_old_code() {
    let h = Harness::new().await;
    let vid = h.create_visitor("孙重发").await;
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;

    let (_, v1) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 120).await;
    let code1 = granted_code(&v1);
    let gid1 = v1["grant"]["id"].as_str().unwrap().to_string();
    assert_eq!(h.auth(&code1, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], true);

    // 重新发码（来访改期/补办）
    let (s, v2) = h.issue(&vid, "front_desk", &["zone-lobby", "zone-lounge-5f"], 3, 180).await;
    assert_eq!(s, StatusCode::OK, "{v2}");
    let code2 = granted_code(&v2);
    assert_ne!(code1, code2);

    // 旧码立即失效
    let d = h.auth(&code1, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "already_revoked");

    let (_, g1) = h.call(Method::GET, &format!("/api/grants/{gid1}"), None, None).await;
    assert_eq!(g1["grant"]["status"], "reissued");
    assert_eq!(g1["grant"]["replacedBy"], v2["grant"]["id"]);
    assert_eq!(g1["grant"]["aclApplied"], false, "旧码的设备规则必须回收");

    // 新码正常可用，并继承新的区域与设备限制
    assert_eq!(h.auth(&code2, "BB:01", Some("u2"), Some("zone-lounge-5f")).await["allowed"], true);
}

#[tokio::test]
async fn front_desk_cannot_grant_admin_zone_or_long_duration() {
    let h = Harness::new().await;
    let vid = h.create_visitor("周越权").await;
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;

    // 1) 前台尝试开通管理员区域 → 403
    let (s, v) = h.issue(&vid, "front_desk", &["zone-admin-core"], 2, 120).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "管理员区域必须拦截: {v}");
    assert_eq!(v["error"], "forbidden");

    // 2) 前台尝试夹带队列：普通区域 + 管理员区域，同样整体拒绝
    let (s, _) = h.issue(&vid, "front_desk", &["zone-lobby", "zone-admin-core"], 2, 120).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // 3) 前台尝试开通超过 8 小时的“长期权限” → 403
    let (s, _) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 8 * 60 + 1).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // 4) 无角色 / 未知角色 → 401，且不会创建授权
    let (s, _) = h
        .call(
            Method::POST,
            &format!("/api/visitors/{vid}/grants"),
            None,
            Some(json!({"zoneIds": ["zone-lobby"], "deviceLimit": 2, "durationMinutes": 120})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // 5) 管理员可以授权管理员区域（对照）
    let (s, v) = h.issue(&vid, "network_admin", &["zone-admin-core"], 2, 120).await;
    assert_eq!(s, StatusCode::OK, "管理员授权应成功: {v}");

    // 6) 确认被拒的三次请求没有产生任何授权
    let (_, list) = h.call(Method::GET, "/api/grants", None, None).await;
    let statuses: Vec<&str> = list["grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses.len(), 1, "只有管理员那一次成功: {statuses:?}");
    assert_eq!(statuses[0], "active");
}

#[tokio::test]
async fn device_limit_and_shared_user_are_blocked() {
    let h = Harness::new().await;
    let vid = h.create_visitor("吴多设备").await;
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 1, 120).await;
    let code = granted_code(&v);

    // 第 1 台设备放行
    assert_eq!(h.auth(&code, "MAC-A", Some("u1"), Some("zone-lobby")).await["allowed"], true);
    // 同一用户的第 2 台设备：超过设备数，拦截并记异常
    let d = h.auth(&code, "MAC-B", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "too_many_devices");
    assert_eq!(d["anomaly"], true);
    // 同一台设备重复认证仍然放行
    assert_eq!(h.auth(&code, "MAC-A", Some("u1"), Some("zone-lobby")).await["allowed"], true);

    // 另一人使用同一临时码：多人共用拦截
    let d = h.auth(&code, "MAC-C", Some("intruder"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "shared_by_multiple_users");
    assert_eq!(d["anomaly"], true);

    // 非授权区域拦截
    let (_, v2) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 120).await;
    let code2 = granted_code(&v2);
    let d = h.auth(&code2, "MAC-D", Some("u2"), Some("zone-meeting-3f")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "zone_forbidden");
}

#[tokio::test]
async fn failed_revoke_is_pending_not_stopped() {
    let h = Harness::new().await;
    let vid = h.create_visitor("郑故障").await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 240).await;
    let gid = v["grant"]["id"].as_str().unwrap().to_string();
    let code = granted_code(&v);
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], true);

    // 注入设备回收故障后离场
    let (s, f) = h
        .call(
            Method::POST,
            "/api/sim/faults",
            Some("network_admin"),
            Some(json!({"kind": "revoke", "armed": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{f}");

    let (s, v) = h.call(Method::POST, &format!("/api/visitors/{vid}/check-out"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // 关键：回收失败绝不能显示为已停用
    assert_eq!(v["grants"][0]["status"], "revoke_pending");
    assert_eq!(v["grants"][0]["intendedStatus"], "checked_out");
    assert!(v["grants"][0]["provisionError"].as_str().unwrap().contains("revoke"));
    assert_eq!(v["grants"][0]["aclApplied"], true, "设备侧规则其实还在，不能谎称已回收");

    // 待处理期间继续尝试联网：拒绝并报异常
    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "revoke_pending");
    assert_eq!(d["anomaly"], true);

    // 前台在授权列表里同样看到“待处理”，不会被当作已停用
    let (_, g) = h.call(Method::GET, &format!("/api/grants/{gid}"), None, None).await;
    assert_eq!(g["grant"]["effectiveState"], "revoke_pending");

    // 管理员待处理列表能看到它
    let (_, pend) = h.call(Method::GET, "/api/admin/pending-revokes", Some("network_admin"), None).await;
    assert_eq!(pend["pendingRevokes"].as_array().unwrap().len(), 1);
    // 前台看不到管理员视图
    let (s, _) = h.call(Method::GET, "/api/admin/pending-revokes", Some("front_desk"), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // 设备恢复后管理员重试回收 → 落到目标终结态 checked_out
    let (_, _) = h
        .call(
            Method::POST,
            "/api/sim/faults",
            Some("network_admin"),
            Some(json!({"kind": "revoke", "armed": false})),
        )
        .await;
    let (s, v) = h.call(Method::POST, &format!("/api/grants/{gid}/retry-revoke"), Some("network_admin"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["grant"]["status"], "checked_out");
    assert_eq!(v["grant"]["aclApplied"], false);
}

#[tokio::test]
async fn provision_failure_is_surfaced_and_retryable() {
    let h = Harness::new().await;
    let vid = h.create_visitor("冯开通失败").await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 240).await;
    let gid = v["grant"]["id"].as_str().unwrap().to_string();
    let code = granted_code(&v);

    // 设备故障时签到：授权进入 provision_failed，前台明确看到失败
    let (_, _) = h
        .call(Method::POST, "/api/sim/faults", Some("network_admin"),
            Some(json!({"kind": "provision", "armed": true})))
        .await;
    let (s, v) = h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["grants"][0]["status"], "provision_failed");
    assert!(v["grants"][0]["provisionError"].is_string());

    let d = h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "provision_failed");

    // 恢复后重试开通
    let (_, _) = h
        .call(Method::POST, "/api/sim/faults", Some("network_admin"),
            Some(json!({"kind": "provision", "armed": false})))
        .await;
    // 激活时间以重试时刻为准，有效期不会因为故障被“吃掉”
    h.clock.set(T0 + 30 * 60_000);
    let (s, v) = h.call(Method::POST, &format!("/api/grants/{gid}/activate"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["grant"]["status"], "active");
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], true);

    // 到期时间相对激活时刻计算：激活后 240 分钟才到期（T0+270min），T0+269min 仍可用
    h.clock.set(T0 + 269 * 60_000);
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], true);
}

#[tokio::test]
async fn unknown_and_manually_revoked_codes_are_anomalies_visible_to_admin() {
    let h = Harness::new().await;

    // 未知码 → 拒绝并记异常
    let d = h.auth("NOT-A-CODE", "ZZ:99", None, None).await;
    assert_eq!(d["allowed"], false);
    assert_eq!(d["reason"], "unknown_code");
    assert_eq!(d["anomaly"], true);

    let vid = h.create_visitor("钱取消").await;
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 120).await;
    let code = granted_code(&v);
    let gid = v["grant"]["id"].as_str().unwrap().to_string();
    // 先成功联网，留下设备使用记录
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], true);

    // 会议取消，手动回收
    let (s, v) = h
        .call(
            Method::POST,
            &format!("/api/grants/{gid}/revoke"),
            Some("front_desk"),
            Some(json!({"reason": "meeting_canceled"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["grant"]["status"], "revoked");
    assert_eq!(v["grant"]["revokeReason"], "meeting_canceled");
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], false);

    // 管理员查看异常登录与设备使用情况
    let (s, a) = h.call(Method::GET, "/api/admin/anomalies", Some("network_admin"), None).await;
    assert_eq!(s, StatusCode::OK);
    let reasons: Vec<&str> = a["anomalies"].as_array().unwrap().iter()
        .map(|e| e["reason"].as_str().unwrap()).collect();
    assert!(reasons.contains(&"unknown_code"), "{reasons:?}");
    assert!(reasons.contains(&"already_revoked"), "{reasons:?}");

    let (s, d) = h.call(Method::GET, "/api/admin/devices", Some("network_admin"), None).await;
    assert_eq!(s, StatusCode::OK);
    // 已尝试连过的设备仍可在设备使用视图中审计到（含其授权状态）
    assert!(d["devices"].as_array().unwrap().iter().any(|x| x["mac"] == "AA:01"));
}

#[tokio::test]
async fn grant_card_never_exposes_full_code_or_hash() {
    let h = Harness::new().await;
    let vid = h.create_visitor("冯卡片").await;
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 120).await;
    let code = granted_code(&v);

    // 列表/详情只出现后 4 位，不含完整码与哈希
    let (_, list) = h.call(Method::GET, "/api/grants", None, None).await;
    let text = list.to_string();
    assert!(!text.contains(&code), "列表泄露完整临时码");
    assert!(!text.contains("codeHash"), "列表泄露码哈希");
    assert!(list["grants"][0]["codeLast4"].as_str().is_some());
}

#[tokio::test]
async fn sweep_retires_due_grants_and_handles_failure() {
    let h = Harness::new().await;
    let vid = h.create_visitor("陈扫描").await;
    let (_, v) = h.issue(&vid, "front_desk", &["zone-lobby"], 2, 60).await;
    let gid = v["grant"]["id"].as_str().unwrap().to_string();
    let code = granted_code(&v);
    h.call(Method::POST, &format!("/api/visitors/{vid}/check-in"), None, None).await;

    // 到期后由管理员触发扫描
    h.clock.set(T0 + 61 * 60_000);
    let (s, w) = h.call(Method::POST, "/api/admin/sweep", Some("network_admin"), None).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(w["retired"][0], gid);

    let (_, g) = h.call(Method::GET, &format!("/api/grants/{gid}"), None, None).await;
    assert_eq!(g["grant"]["status"], "expired");
    assert_eq!(h.auth(&code, "AA:01", Some("u1"), Some("zone-lobby")).await["allowed"], false);
}
