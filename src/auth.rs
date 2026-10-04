use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// 本地模拟登录：通过 X-User-Id 请求头标识当前操作人。
/// 种子用户：1=reception(前台) 2=netadmin(网络管理员)
#[derive(Debug, Clone)]
pub struct Actor {
    pub id: i64,
    pub username: String,
    pub role: String,
}

impl Actor {
    pub fn require(&self, role: &str) -> AppResult<()> {
        if self.role != role {
            return Err(AppError::Forbidden(format!(
                "需要 {role} 角色，当前为 {}",
                self.role
            )));
        }
        Ok(())
    }
}

#[axum::async_trait]
impl FromRequestParts<AppState> for Actor {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let id = parts
            .headers
            .get("x-user-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| AppError::Unauthorized("缺少有效的 X-User-Id 请求头".into()))?;
        let user = sqlx::query_as::<_, User>("SELECT id, username, role FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::Unauthorized(format!("用户 {id} 不存在")))?;
        Ok(Actor {
            id: user.id,
            username: user.username,
            role: user.role,
        })
    }
}
