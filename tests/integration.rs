use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use visitor_net::{build_app, build_state, AppState};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

const FRONT_DESK: i64 = 1;
const ADMIN: i64 = 2;

async fn spawn() -> (AppState, String) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let url = format!(
        "sqlite://{}/vn_test_{}_{}.db",
        std::env::temp_dir().display(),
        std::process::id(),
        n
    );
    let state = build_state(&url).await;
    let app = build_app(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, format!("http://{addr}"))
}

fn freeze(state: &AppState, t: DateTime<Utc>) {
    state.clock.lock().unwrap().frozen = Some(t);
}

async fn post(base: &str, user: i64, path: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("X-User-Id", user.to_string())
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn get(base: &str, user: i64, path: &str) -> (u16, Value) {
    let res = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header("X-User-Id", user.to_string())
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

/// 模拟设备尝试上网（网络设备回调，无需登录态）
async fn connect(base: &str, code: &str, mac: &str, zone: i64) -> Value {
    let res = reqwest::Client::new()
        .post(format!("{base}/api/network/connect"))
        .json(&json!({ "code": code, "mac": mac, "zone_id": zone }))
        .send()
        .await
        .unwrap();
    res.json().await.unwrap()
}

async fn create_visit(base: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Value {
    let (status, v) = post(
        base,
        FRONT_DESK,
        "/api/visits",
        json!({
            "visitor_name": "张三",
            "company": "ACME 科技",
            "zone_ids": [1],
            "device_limit": 1,
            "valid_from": from.to_rfc3339(),
            "valid_to": to.to_rfc3339(),
        }),
    )
    .await;
    assert_eq!(status, 201, "创建来访失败: {v}");
    v
}

async fn reveal(base: &str, auth_id: i64) -> String {
    let (status, v) = post(base, FRONT_DESK, &format!("/api/auths/{auth_id}/reveal"), json!({})).await;
    assert_eq!(status, 200, "查看完整码失败: {v}");
    v["code"].as_str().unwrap().to_string()
}

// ---------- 1. 到期与离场回收 ----------

#[tokio::test]
async fn expiry_and_checkout_revoke_access() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);

    // 创建来访（1 台设备限额），签到前不能上网
    let v = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let auth_id = v["auths"][0]["id"].as_i64().unwrap();
    assert_eq!(v["auths"][0]["status"], "pending_checkin");
    let code = reveal(&base, auth_id).await;

    let r = connect(&base, &code, "AA:01", 1).await;
    assert_eq!(r["result"], "denied_not_active", "签到前不允许生效");

    // 签到后生效
    let (s, _) = post(&base, FRONT_DESK, &format!("/api/visits/{}/checkin", v["id"]), json!({})).await;
    assert_eq!(s, 200);
    let r = connect(&base, &code, "AA:01", 1).await;
    assert_eq!(r["allow"], true, "签到后应可上网");

    // 设备超限：第二台设备被拦截并标记异常
    let r = connect(&base, &code, "BB:02", 1).await;
    assert_eq!(r["result"], "denied_device_limit");
    let (_, anomalies) = get(&base, ADMIN, "/api/admin/anomalies").await;
    assert!(anomalies.as_array().unwrap().iter().any(|e| e["result"] == "denied_device_limit"));

    // 到期：到达 valid_to 边界即过期，网络侧同步回收
    freeze(&state, t0 + Duration::seconds(3600));
    let r = connect(&base, &code, "AA:01", 1).await;
    assert_eq!(r["result"], "denied_expired", "到达 valid_to 应立即判定过期");
    let (_, a) = get(&base, FRONT_DESK, &format!("/api/auths/{auth_id}")).await;
    assert_eq!(a["status"], "expired");
    assert_eq!(a["network_state"], "revoked");
    let (_, net) = get(&base, ADMIN, "/api/admin/network/state").await;
    assert!(net["applied"].as_array().unwrap().is_empty(), "到期后设备上不应残留权限");

    // 离场回收：新访客签到后离场，原码立即失效
    freeze(&state, t0);
    let v2 = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let auth2 = v2["auths"][0]["id"].as_i64().unwrap();
    let code2 = reveal(&base, auth2).await;
    post(&base, FRONT_DESK, &format!("/api/visits/{}/checkin", v2["id"]), json!({})).await;
    assert_eq!(connect(&base, &code2, "CC:03", 1).await["allow"], true);
    post(&base, FRONT_DESK, &format!("/api/visits/{}/checkout", v2["id"]), json!({})).await;
    let r = connect(&base, &code2, "CC:03", 1).await;
    assert_eq!(r["result"], "denied_revoked", "离场后原码不得继续使用");
    let (_, a2) = get(&base, FRONT_DESK, &format!("/api/auths/{auth2}")).await;
    assert_eq!(a2["status"], "revoked");
    assert_eq!(a2["network_state"], "revoked");
}

// ---------- 2. 时间边界精确判断 ----------

#[tokio::test]
async fn time_boundary_is_exact() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);

    // 有效期 [t0+300, t0+900)
    let v = create_visit(&base, t0 + Duration::seconds(300), t0 + Duration::seconds(900)).await;
    let auth_id = v["auths"][0]["id"].as_i64().unwrap();
    let code = reveal(&base, auth_id).await;
    post(&base, FRONT_DESK, &format!("/api/visits/{}/checkin", v["id"]), json!({})).await;

    freeze(&state, t0 + Duration::seconds(299));
    assert_eq!(connect(&base, &code, "AA:01", 1).await["result"], "denied_not_yet", "生效前 1 秒不可用");

    freeze(&state, t0 + Duration::seconds(300));
    assert_eq!(connect(&base, &code, "AA:01", 1).await["allow"], true, "valid_from 边界应生效");

    freeze(&state, t0 + Duration::seconds(899));
    assert_eq!(connect(&base, &code, "AA:01", 1).await["allow"], true, "过期前 1 秒仍可用");

    freeze(&state, t0 + Duration::seconds(900));
    assert_eq!(connect(&base, &code, "AA:01", 1).await["result"], "denied_expired", "valid_to 边界即过期");
}

// ---------- 3. 重新发码 / 改期 / 取消使旧码失效 ----------

#[tokio::test]
async fn reissue_invalidates_old_code() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);

    let v = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let visit_id = v["id"].as_i64().unwrap();
    let old_auth = v["auths"][0]["id"].as_i64().unwrap();
    let old_code = reveal(&base, old_auth).await;
    post(&base, FRONT_DESK, &format!("/api/visits/{visit_id}/checkin"), json!({})).await;
    assert_eq!(connect(&base, &old_code, "AA:01", 1).await["allow"], true);

    // 重新发码
    let (s, v2) = post(&base, FRONT_DESK, &format!("/api/visits/{visit_id}/reissue"), json!({})).await;
    assert_eq!(s, 200);
    let new_auth = v2["auths"][1]["id"].as_i64().unwrap();
    let new_code = reveal(&base, new_auth).await;
    assert_ne!(old_code, new_code);

    // 旧码失效，新码可用
    assert_eq!(connect(&base, &old_code, "AA:01", 1).await["result"], "denied_superseded");
    assert_eq!(connect(&base, &new_code, "AA:01", 1).await["allow"], true);
    let (_, old) = get(&base, FRONT_DESK, &format!("/api/auths/{old_auth}")).await;
    assert_eq!(old["status"], "superseded");
    let (_, net) = get(&base, ADMIN, "/api/admin/network/state").await;
    let applied: Vec<i64> = net["applied"].as_array().unwrap().iter().map(|x| x["auth_id"].as_i64().unwrap()).collect();
    assert!(applied.contains(&new_auth) && !applied.contains(&old_auth), "设备上应只有新码权限");
}

#[tokio::test]
async fn cancel_and_reschedule_kill_old_codes() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);

    // 会议取消 -> 码失效
    let v1 = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let v1_id = v1["id"].as_i64().unwrap();
    let code1 = reveal(&base, v1["auths"][0]["id"].as_i64().unwrap()).await;
    post(&base, FRONT_DESK, &format!("/api/visits/{v1_id}/checkin"), json!({})).await;
    assert_eq!(connect(&base, &code1, "AA:01", 1).await["allow"], true);
    post(&base, FRONT_DESK, &format!("/api/visits/{v1_id}/cancel"), json!({})).await;
    assert_eq!(connect(&base, &code1, "AA:01", 1).await["result"], "denied_revoked");
    // 已取消的来访不允许重新发码
    let (s, _) = post(&base, FRONT_DESK, &format!("/api/visits/{v1_id}/reissue"), json!({})).await;
    assert_eq!(s, 409);

    // 改期 -> 旧码失效，新码按新窗口生效
    let v2 = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let v2_id = v2["id"].as_i64().unwrap();
    let old_auth2 = v2["auths"][0]["id"].as_i64().unwrap();
    let code2 = reveal(&base, old_auth2).await;
    post(&base, FRONT_DESK, &format!("/api/visits/{v2_id}/checkin"), json!({})).await;
    assert_eq!(connect(&base, &code2, "BB:02", 1).await["allow"], true);

    let (s, v2r) = post(
        &base,
        FRONT_DESK,
        &format!("/api/visits/{v2_id}/reschedule"),
        json!({ "valid_from": (t0 + Duration::seconds(7200)).to_rfc3339(), "valid_to": (t0 + Duration::seconds(10800)).to_rfc3339() }),
    )
    .await;
    assert_eq!(s, 200);
    let new_auth2 = v2r["auths"][1]["id"].as_i64().unwrap();
    let new_code2 = reveal(&base, new_auth2).await;
    assert_eq!(connect(&base, &code2, "BB:02", 1).await["result"], "denied_superseded", "改期后旧码必须失效");
    assert_eq!(connect(&base, &new_code2, "BB:02", 1).await["result"], "denied_not_yet", "新窗口未开始不能用");
    freeze(&state, t0 + Duration::seconds(7200));
    assert_eq!(connect(&base, &new_code2, "BB:02", 1).await["allow"], true, "新窗口开始后可用");
}

// ---------- 4. 前台越权创建被阻止 ----------

#[tokio::test]
async fn front_desk_cannot_escalate_self_privileges() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);

    // 给自己开管理区权限 -> 403
    let (s, v) = post(
        &base,
        FRONT_DESK,
        "/api/visits",
        json!({ "visitor_name": "reception", "company": "本公司", "visitor_user_id": FRONT_DESK,
                "zone_ids": [4], "device_limit": 1,
                "valid_from": t0.to_rfc3339(), "valid_to": (t0 + Duration::hours(2)).to_rfc3339() }),
    )
    .await;
    assert_eq!(s, 403, "应阻止前台给自己开管理区权限: {v}");
    assert!(v["error"].as_str().unwrap().contains("越权"));

    // 给自己开 48 小时长期权限 -> 403
    let (s, v) = post(
        &base,
        FRONT_DESK,
        "/api/visits",
        json!({ "visitor_name": "reception", "company": "本公司", "visitor_user_id": FRONT_DESK,
                "zone_ids": [1], "device_limit": 1,
                "valid_from": t0.to_rfc3339(), "valid_to": (t0 + Duration::hours(48)).to_rfc3339() }),
    )
    .await;
    assert_eq!(s, 403, "应阻止前台给自己开长期权限: {v}");

    // 给自己开 2 小时大堂权限 -> 允许
    let (s, v) = post(
        &base,
        FRONT_DESK,
        "/api/visits",
        json!({ "visitor_name": "reception", "company": "本公司", "visitor_user_id": FRONT_DESK,
                "zone_ids": [1], "device_limit": 1,
                "valid_from": t0.to_rfc3339(), "valid_to": (t0 + Duration::hours(2)).to_rfc3339() }),
    )
    .await;
    assert_eq!(s, 201, "合规的短期自用权限应允许: {v}");

    // 网络管理员不能执行前台发码操作
    let (s, _) = post(
        &base,
        ADMIN,
        "/api/visits",
        json!({ "visitor_name": "李四", "company": "ACME", "zone_ids": [1], "device_limit": 1,
                "valid_from": t0.to_rfc3339(), "valid_to": (t0 + Duration::hours(2)).to_rfc3339() }),
    )
    .await;
    assert_eq!(s, 403, "网管不应能发码");

    // 前台不能访问网管的异常登录接口
    let (s, _) = get(&base, FRONT_DESK, "/api/admin/anomalies").await;
    assert_eq!(s, 403, "前台不应能查看异常登录");
}

// ---------- 5. 回收失败必须可见，不能当作已停用 ----------

#[tokio::test]
async fn revoke_failure_is_visible_not_silent() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);

    let v = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let visit_id = v["id"].as_i64().unwrap();
    let auth_id = v["auths"][0]["id"].as_i64().unwrap();
    post(&base, FRONT_DESK, &format!("/api/visits/{visit_id}/checkin"), json!({})).await;

    // 模拟网络设备故障，离场回收失败
    post(&base, ADMIN, "/api/admin/network/mock", json!({ "fail_revoke": true })).await;
    post(&base, FRONT_DESK, &format!("/api/visits/{visit_id}/checkout"), json!({})).await;

    // 前台查询：必须显示回收失败，而不是已停用
    let (_, a) = get(&base, FRONT_DESK, &format!("/api/auths/{auth_id}")).await;
    assert_eq!(a["status"], "revoke_failed", "回收失败不得标记为已回收");
    assert_eq!(a["network_state"], "applied", "设备上权限实际仍存在，必须可见");
    let (_, net) = get(&base, ADMIN, "/api/admin/network/state").await;
    assert_eq!(net["applied"].as_array().unwrap().len(), 1, "故障期间权限仍挂在设备上");

    // 故障恢复后手动重试 -> 真正回收
    post(&base, ADMIN, "/api/admin/network/mock", json!({ "fail_revoke": false })).await;
    let (s, a) = post(&base, FRONT_DESK, &format!("/api/auths/{auth_id}/retry-revoke"), json!({})).await;
    assert_eq!(s, 200);
    assert_eq!(a["status"], "revoked");
    assert_eq!(a["network_state"], "revoked");
    let (_, net) = get(&base, ADMIN, "/api/admin/network/state").await;
    assert!(net["applied"].as_array().unwrap().is_empty());

    // 扫描器路径：到期时设备故障 -> revoke_failed；恢复后扫描自动补回收
    let v2 = create_visit(&base, t0, t0 + Duration::seconds(600)).await;
    let visit2 = v2["id"].as_i64().unwrap();
    let auth2 = v2["auths"][0]["id"].as_i64().unwrap();
    post(&base, FRONT_DESK, &format!("/api/visits/{visit2}/checkin"), json!({})).await;
    post(&base, ADMIN, "/api/admin/network/mock", json!({ "fail_revoke": true })).await;
    freeze(&state, t0 + Duration::seconds(600));
    post(&base, ADMIN, "/api/admin/sweep", json!({})).await;
    let (_, a2) = get(&base, FRONT_DESK, &format!("/api/auths/{auth2}")).await;
    assert_eq!(a2["status"], "revoke_failed", "到期回收失败必须显示失败/待处理");

    post(&base, ADMIN, "/api/admin/network/mock", json!({ "fail_revoke": false })).await;
    post(&base, ADMIN, "/api/admin/sweep", json!({})).await;
    let (_, a2) = get(&base, FRONT_DESK, &format!("/api/auths/{auth2}")).await;
    assert_eq!(a2["status"], "expired", "故障恢复后扫描应补完成回收");
    assert_eq!(a2["network_state"], "revoked");
}

// ---------- 6. 访客卡片默认隐藏完整码 ----------

#[tokio::test]
async fn code_is_masked_by_default() {
    let (state, base) = spawn().await;
    let t0 = Utc::now();
    freeze(&state, t0);
    let v = create_visit(&base, t0, t0 + Duration::seconds(3600)).await;
    let auth_id = v["auths"][0]["id"].as_i64().unwrap();

    // 列表/详情接口不得包含完整码字段
    let (_, list) = get(&base, FRONT_DESK, "/api/visits").await;
    let serialized = list.to_string();
    let code = reveal(&base, auth_id).await;
    assert!(!serialized.contains(&code), "列表接口不应泄露完整临时码");
    assert!(v["auths"][0].get("code").is_none(), "授权视图只应包含尾号");
    assert_eq!(v["auths"][0]["code_hint"].as_str().unwrap().len(), 4);
}
