//! 本地模拟的网络设备（AC/网关）与通知接口。
//!
//! 真实系统中 provision/revoke 会调用网络设备控制器下发/回收 ACL；
//! 这里通过 SQLite 表模拟：`device_acls` 表示设备上当前生效的规则，
//! `sim_faults` 用于故障注入（模拟设备离线/超时），便于验证回收失败的处理。

use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

/// 向网络设备下发访问规则。设备故障时返回 Err，调用方必须把授权标记为开通失败。
pub async fn provision(pool: &SqlitePool, grant_id: &str, now_ms: i64) -> AppResult<()> {
    let armed: Option<(i64,)> =
        sqlx::query_as("SELECT armed FROM sim_faults WHERE kind = 'provision'")
            .fetch_optional(pool)
            .await?;
    if armed.map(|(a,)| a).unwrap_or(0) == 1 {
        return Err(AppError::Internal(
            "network controller unreachable: ACL provision timeout".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO device_acls (grant_id, applied, updated_at) VALUES (?1, 1, ?2)
         ON CONFLICT(grant_id) DO UPDATE SET applied = 1, updated_at = ?2",
    )
    .bind(grant_id)
    .bind(now_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// 回收设备上的访问规则。设备故障时返回 Err —— 调用方绝不能把授权当作已停用，
/// 必须保留/转入 `revoke_pending` 待处理状态。
pub async fn revoke(pool: &SqlitePool, grant_id: &str, now_ms: i64) -> AppResult<()> {
    let armed: Option<(i64,)> = sqlx::query_as("SELECT armed FROM sim_faults WHERE kind = 'revoke'")
        .fetch_optional(pool)
        .await?;
    if armed.map(|(a,)| a).unwrap_or(0) == 1 {
        return Err(AppError::Internal(
            "network controller unreachable: ACL revoke timeout".into(),
        ));
    }
    let res = sqlx::query(
        "UPDATE device_acls SET applied = 0, updated_at = ?2 WHERE grant_id = ?1 AND applied = 1",
    )
    .bind(grant_id)
    .bind(now_ms)
    .execute(pool)
    .await?;
    // 设备上没有规则（例如从未成功下发）视为回收成功。
    let _ = res.rows_affected();
    Ok(())
}

/// 查询设备侧规则是否仍然存在（模拟网管排查）。
pub async fn acl_applied(pool: &SqlitePool, grant_id: &str) -> AppResult<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT applied FROM device_acls WHERE grant_id = ?1")
            .bind(grant_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(a,)| a == 1).unwrap_or(false))
}

/// 故障注入开关（测试/演示用）。
pub async fn set_fault(pool: &SqlitePool, kind: &str, armed: bool) -> AppResult<()> {
    sqlx::query("UPDATE sim_faults SET armed = ?2 WHERE kind = ?1")
        .bind(kind)
        .bind(armed as i64)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn notify(
    pool: &SqlitePool,
    ts_ms: i64,
    channel: &str,
    kind: &str,
    message: &str,
    ref_id: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO notifications (ts, channel, kind, message, ref_id) VALUES (?1,?2,?3,?4,?5)",
    )
    .bind(ts_ms)
    .bind(channel)
    .bind(kind)
    .bind(message)
    .bind(ref_id)
    .execute(pool)
    .await?;
    Ok(())
}
