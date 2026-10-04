use crate::handlers::revoke_auth;
use crate::models::{st, Auth};
use crate::AppState;
use std::time::Duration;

/// 扫描一轮：到期授权自动回收；回收失败的重试。
/// 返回 (到期处理数, 重试数)
pub async fn sweep_once(state: &AppState) -> (usize, usize) {
    let now = state.now();
    let mut expired = 0;
    let rows = sqlx::query_as::<_, Auth>(
        "SELECT * FROM authorizations WHERE status IN ('active', 'pending_checkin')",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    for a in rows {
        if now >= a.valid_to {
            let _ = revoke_auth(state, a.id, st::EXPIRED, "到期自动回收").await;
            expired += 1;
        }
    }

    let mut retried = 0;
    let failed = sqlx::query_as::<_, Auth>(
        "SELECT * FROM authorizations WHERE status = 'revoke_failed'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    for a in failed {
        let target = a.target_status.clone().unwrap_or_else(|| st::REVOKED.to_string());
        let reason = a.revoke_reason.clone().unwrap_or_else(|| "回收重试".to_string());
        let _ = revoke_auth(state, a.id, &target, &reason).await;
        retried += 1;
    }
    (expired, retried)
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let (expired, retried) = sweep_once(&state).await;
            if expired > 0 || retried > 0 {
                tracing::info!("[回收扫描] 到期处理 {expired} 条，失败重试 {retried} 条");
            }
        }
    });
}
