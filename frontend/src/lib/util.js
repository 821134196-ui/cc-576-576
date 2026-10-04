export function fmtTime(ms) {
  if (ms == null) return '—';
  return new Date(ms).toLocaleString('zh-CN', { hour12: false });
}

export function fmtMinutes(m) {
  if (m == null) return '—';
  if (m < 60) return `${m} 分钟`;
  const h = m / 60;
  return Number.isInteger(h) ? `${h} 小时` : `${h.toFixed(1)} 小时`;
}

export const STATUS_TEXT = {
  pending: '待签到生效',
  active: '使用中',
  provision_failed: '开通失败',
  revoke_pending: '回收失败 · 待处理',
  expired: '已到期',
  checked_out: '已离场回收',
  revoked: '已取消',
  reissued: '已重新发码',
  expired_pending_sweep: '已到期待扫描'
};

export const STATUS_CLASS = {
  pending: 'st-pending',
  active: 'st-active',
  provision_failed: 'st-failed',
  revoke_pending: 'st-danger',
  expired: 'st-done',
  checked_out: 'st-done',
  revoked: 'st-done',
  reissued: 'st-done',
  expired_pending_sweep: 'st-warn'
};

export const REASON_TEXT = {
  ok: '正常放行',
  not_checked_in: '访客未签到，未生效',
  provision_failed: '网络设备开通失败',
  revoke_pending: '权限回收中（设备侧失败），禁止联网',
  expired: '已到期',
  already_revoked: '授权已失效（离场/取消/重发）',
  shared_by_multiple_users: '多人共用同一临时码，已拦截',
  too_many_devices: '设备数量超过限制，已拦截',
  zone_forbidden: '该区域不在授权范围',
  unknown_code: '临时码无法识别'
};
