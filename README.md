# 共享办公访客网络临时码系统

前台为已签到访客开通限时上网权限；网络管理员检查异常登录与设备使用；访客离场或到期后权限自动回收。

- 后端：Rust + Axum + SQLx（SQLite）
- 前端：Svelte（Vite 构建，由后端托管静态资源）
- 网络设备与通知接口：本地模拟（`MockNetwork` + 通知表/日志）

## 一条命令启动

```bash
./start.sh
```

启动后访问 http://localhost:3000 （数据库文件 `visitor_net.db` 自动创建）。

开发模式（前后端热更新）：终端1 `cargo run`，终端2 `cd frontend && npm run dev`（Vite 代理 /api 到 3000）。

## 角色（本地模拟登录，页面右上角切换）

| 用户 | X-User-Id | 角色 |
|---|---|---|
| reception | 1 | 前台 front_desk |
| netadmin | 2 | 网络管理员 network_admin |

## 核心规则

- **签到生效**：发码时选择来访公司、可用区域、设备数、有效时间；授权初始为 `pending_checkin`，签到后才下发到网络设备生效。
- **时间边界**：有效期为 `[valid_from, valid_to)` —— 到达 `valid_to` 即刻过期；后台每 10 秒扫描回收，设备接入时也会即时判定（懒过期）。
- **旧码失效**：会议取消、改期、重新发码都会先回收旧授权（`revoked`/`superseded`）再签发新码。
- **设备超限拦截**：同一码接入设备数超过限额即拒绝并标记异常（疑似多人共用）。
- **越权拦截**：前台不能给自己开通管理区域权限，也不能给自己开超过 24 小时的长期权限；发码仅限前台角色，异常登录/设备视图仅限网管角色。
- **回收失败可见**：网络设备回收失败时授权进入 `revoke_failed`，且 `network_state` 保持 `applied`（权限实际仍在设备上），绝不显示为已停用；支持手动重试，扫描器也会自动重试。
- **完整码默认隐藏**：列表/详情接口只返回尾号 4 位，完整码需前台显式调用 `reveal` 查看。

## 验证

```bash
cargo test
```

7 个集成测试覆盖需求要求的验证点：

| 测试 | 验证内容 |
|---|---|
| `expiry_and_checkout_revoke_access` | 签到前不可用、到期边界即回收、离场后原码失效、设备超限拦截并告警 |
| `time_boundary_is_exact` | `valid_from` 前 1 秒拒绝、边界放行、`valid_to` 边界即过期 |
| `reissue_invalidates_old_code` | 重新发码后旧码 `superseded`、设备上只剩新码权限 |
| `cancel_and_reschedule_kill_old_codes` | 取消/改期使旧码失效，新码按新窗口生效 |
| `front_desk_cannot_escalate_self_privileges` | 前台给自己开管理区/长期权限被 403 拦截；角色互斥 |
| `revoke_failure_is_visible_not_silent` | 回收失败显示 `revoke_failed` 而非已停用，重试与扫描补回收 |
| `code_is_masked_by_default` | 列表接口不泄露完整临时码 |

## API 摘要

前台：`POST /api/visits`（发码）、`POST /api/visits/:id/checkin|checkout|cancel|reschedule|reissue`、`GET /api/visits`、`GET /api/auths/:id`（开通/回收状态查询）、`POST /api/auths/:id/reveal|retry-revoke`

网管：`GET /api/admin/anomalies|events|devices`、`GET|POST /api/admin/network/state|mock`（模拟设备故障）、`POST /api/admin/time`（演示时间偏移）、`POST /api/admin/sweep`

设备模拟：`POST /api/network/connect`（设备持码尝试上网，返回放行/拒绝及原因）
