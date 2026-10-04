# 共享办公访客网络临时码系统

前台为已签到访客开通**限时**上网权限；网络管理员检查异常登录与设备使用；
访客离场、到期、会议取消/改期或重新发码后，旧权限立即失效。

- **后端**：Rust + Axum + SQLx + SQLite（`backend/`）
- **前端**：Svelte 4 + Vite（`frontend/`）
- **网络设备 / 通知**：本地模拟（设备 ACL 表 `device_acls`、故障注入表 `sim_faults`、通知表 `notifications`）

## 一条命令本地启动

```bash
./run.sh
```

脚本会构建前端、自动执行 SQLite 迁移并在 `http://127.0.0.1:8787` 启动服务。
浏览器打开后右上角可切换 **前台 / 网络管理员** 身份（演示用 `X-Role` 请求头模拟登录角色）。

前端独立开发模式（热更新）：

```bash
cd frontend && npm install && npm run dev   # http://127.0.0.1:5173，/api 代理到 8787
```

运行后端测试：

```bash
cd backend && cargo test
```

## 业务规则与实现要点

| 需求 | 实现 |
| --- | --- |
| 发码选择公司/区域/设备数/有效时间 | `POST /api/visitors/:id/grants`，公司随访客登记，区域为快照 JSON |
| 签到后才允许生效 | 未签到授权为 `pending`；`check-in` 时才向模拟设备下发 ACL 并置 `active`，有效期自激活时刻起算 |
| 卡片写清限制、默认隐藏完整码 | DB 只存 SHA-256 哈希与后 4 位；完整码仅在发码响应中返回一次，卡片默认打码，可点“显示”临时查看 |
| 离场/到期不可再用 | `check-out` 回收全部未终结授权；时间区间左闭右开 `[valid_from, expires_at)`，`now >= expires_at` 立即拒绝并惰性回收，另有管理员扫描接口 |
| 取消/改期/重发旧码失效 | 重发自动把旧授权置 `reissued` 并撤下设备规则；手动回收支持 `meeting_canceled`/`reschedule` |
| 多人共用拦截 | 首个使用者标识绑定授权，他人用同码返回 `shared_by_multiple_users` 并记异常 |
| 设备数超限拦截 | 设备按 MAC 去重，第 `limit+1` 台返回 `too_many_devices` 并记异常 |
| 前台不能开管理员区域/长期权限 | 管理员区域 `zones.is_admin=1`，前台请求直接 **403**；前台时长上限 8 小时（管理员 7 天） |
| 回收失败必须显式失败 | 设备回收失败 → 状态 `revoke_pending` + `intendedStatus` + 错误信息，卡片/列表显示“回收失败·待处理”，**绝不显示为已停用**，期间继续联网被拒并记异常；设备恢复后管理员“重试回收” |
| 开通失败显式可见 | 签到时下发失败 → `provision_failed`，可重试，有效期以重试激活时刻起算 |
| 网管检查异常/设备 | `/api/admin/anomalies`、`/api/admin/events`、`/api/admin/devices`、`/api/admin/pending-revokes`（仅 `network_admin`） |
| 前台查询开通/回收结果 | 访客详情与授权卡片实时展示状态、设备侧 ACL 是否仍在、失败原因 |

## 验证过的场景

`cargo test` 共 11 个集成测试（内存 SQLite + 可控毫秒时钟），含题目要求的三项：

1. **到期与离场回收**：`exact_expiry_boundary_is_half_open`（到期前 1ms 放行、到期当刻拒绝并回收设备规则）、`check_out_revokes_access`、`sweep_retires_due_grants_and_handles_failure`
2. **重新发码后旧码失效**：`reissue_invalidates_old_code`（旧码 `already_revoked`，旧授权 `reissued` 且设备 ACL 撤下）
3. **前台越权被阻止**：`front_desk_cannot_grant_admin_zone_or_long_duration`（管理员区域、夹带区域、超 8 小时均 403，且不产生授权）

另含设备数/多人共用/越区拦截、回收失败 `revoke_pending` 全链路、开通失败重试、未知码异常、卡片不泄露完整码等。

## 主要接口

```
GET  /api/meta                        公司、区域
POST /api/visitors                    登记访客
POST /api/visitors/:id/check-in       签到（激活待生效授权）
POST /api/visitors/:id/check-out      离场（回收授权）
POST /api/visitors/:id/grants         发码（X-Role: front_desk|network_admin）
POST /api/grants/:id/revoke           取消/改期回收 {reason}
POST /api/grants/:id/activate         开通失败后重试
POST /api/grants/:id/retry-revoke     管理员重试设备回收
POST /api/network/auth                模拟设备认证 {code, mac, userId, zoneId}
GET  /api/admin/anomalies|events|devices|pending-revokes   网管视图
POST /api/admin/sweep                 扫描并回收到期授权
POST /api/sim/faults                  故障注入 {kind: provision|revoke, armed}
```

## 授权状态机

```
pending ──签到且下发成功──▶ active ──到期/离场/取消/改期/重发且设备回收成功──▶ expired | checked_out | revoked | reissued
   │                           │
   └──签到但下发失败──▶ provision_failed ──重试成功──▶ active
                               │
                  设备回收失败──▶ revoke_pending（intended_status=目标态）──设备恢复后重试──▶ 目标终结态
```
