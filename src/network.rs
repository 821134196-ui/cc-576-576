use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// 本地模拟的网络设备（准入控制器）接口。
/// apply 下发权限，revoke 回收权限；可通过 fail_revoke 模拟设备故障。
pub struct MockNetwork {
    fail_revoke: AtomicBool,
    /// auth_id -> code，表示当前真实下发到设备上的权限
    applied: Mutex<BTreeMap<i64, String>>,
}

impl MockNetwork {
    pub fn new() -> Self {
        Self {
            fail_revoke: AtomicBool::new(false),
            applied: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn apply(&self, auth_id: i64, code: &str) -> Result<(), String> {
        self.applied.lock().unwrap().insert(auth_id, code.to_string());
        tracing::info!("[模拟网络设备] 下发权限 auth#{auth_id}");
        Ok(())
    }

    pub async fn revoke(&self, auth_id: i64) -> Result<(), String> {
        if self.fail_revoke.load(Ordering::SeqCst) {
            tracing::warn!("[模拟网络设备] 回收 auth#{auth_id} 失败：设备无响应");
            return Err("网络设备无响应（模拟故障）".to_string());
        }
        self.applied.lock().unwrap().remove(&auth_id);
        tracing::info!("[模拟网络设备] 回收权限 auth#{auth_id}");
        Ok(())
    }

    pub fn set_fail_revoke(&self, fail: bool) {
        self.fail_revoke.store(fail, Ordering::SeqCst);
    }

    pub fn fail_revoke(&self) -> bool {
        self.fail_revoke.load(Ordering::SeqCst)
    }

    pub fn applied_list(&self) -> Vec<(i64, String)> {
        self.applied
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }
}
