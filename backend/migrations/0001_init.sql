-- 访客网络临时码系统 初始结构
-- 所有时间存储为 Unix 毫秒整数（UTC），到期判定使用整数比较，边界精确：
-- 有效区间为左闭右开 [valid_from, expires_at)，即 now >= expires_at 立即失效。

CREATE TABLE companies (
    id      TEXT PRIMARY KEY,
    name    TEXT NOT NULL
);

CREATE TABLE zones (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    is_admin    INTEGER NOT NULL DEFAULT 0  -- 1 = 网络管理员区域，前台不可授权
);

CREATE TABLE visitors (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    company_id   TEXT NOT NULL REFERENCES companies(id),
    checked_in   INTEGER NOT NULL DEFAULT 0,
    check_in_at  INTEGER,
    check_out_at INTEGER,
    created_at   INTEGER NOT NULL
);

-- 授权（临时码）。一个访客可有多条历史授权；重新发码/取消/离场后旧码全部失效。
CREATE TABLE grants (
    id                TEXT PRIMARY KEY,
    code_hash         TEXT NOT NULL UNIQUE,       -- SHA-256(临时码)
    code_last4        TEXT NOT NULL,              -- 仅存后 4 位用于卡片展示
    visitor_id        TEXT NOT NULL REFERENCES visitors(id),
    company_id        TEXT NOT NULL REFERENCES companies(id),
    zone_ids          TEXT NOT NULL,              -- JSON 数组，创建时快照
    device_limit      INTEGER NOT NULL,
    duration_minutes  INTEGER NOT NULL,           -- 可上网时长，自签到激活时刻起算
    valid_from        INTEGER,                    -- 实际生效时间（签到激活后写入）
    expires_at        INTEGER,                    -- 到期毫秒时间戳；now >= expires_at 即失效
    status            TEXT NOT NULL,              -- pending|active|provision_failed|expired|checked_out|revoked|reissued|revoke_pending
    intended_status   TEXT,                       -- 回收失败时计划落到的终结状态
    revoke_reason     TEXT,                       -- checked_out|expired|meeting_canceled|reschedule|reissued|manual
    replaced_by       TEXT,                       -- 重新发码时指向新授权
    provision_error   TEXT,
    bound_user_id     TEXT,                       -- 首个使用者绑定，他人用同码即拦截
    created_by_role   TEXT NOT NULL,              -- front_desk | network_admin
    created_at        INTEGER NOT NULL,
    activated_at      INTEGER,
    ended_at          INTEGER
);
CREATE INDEX idx_grants_visitor ON grants(visitor_id);

CREATE TABLE devices (
    id          TEXT PRIMARY KEY,
    grant_id    TEXT NOT NULL REFERENCES grants(id),
    mac         TEXT NOT NULL,
    user_id     TEXT,
    device_name TEXT,
    first_seen  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL,
    UNIQUE(grant_id, mac)
);
CREATE INDEX idx_devices_grant ON devices(grant_id);

-- 网络认证事件（模拟 NAS/RADIUS 上报）
CREATE TABLE auth_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          INTEGER NOT NULL,
    grant_id    TEXT,                              -- 码无法识别时为空
    visitor_id  TEXT,
    code_last4  TEXT,
    mac         TEXT,
    user_id     TEXT,
    zone_id     TEXT,
    result      TEXT NOT NULL,                     -- allowed | denied
    reason      TEXT NOT NULL,                     -- ok / 各类拒绝原因
    anomaly     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_events_grant ON auth_events(grant_id);
CREATE INDEX idx_events_anom ON auth_events(anomaly, ts);

-- 模拟通知中心
CREATE TABLE notifications (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    ts        INTEGER NOT NULL,
    channel   TEXT NOT NULL,                       -- visitor | admin | front_desk
    kind      TEXT NOT NULL,
    message   TEXT NOT NULL,
    ref_id    TEXT
);

-- 模拟网络设备（AC/网关）故障注入：armed=1 时对应下发/回收调用失败
CREATE TABLE sim_faults (
    kind   TEXT PRIMARY KEY,                    -- provision | revoke
    armed  INTEGER NOT NULL DEFAULT 0
);
INSERT INTO sim_faults (kind, armed) VALUES ('provision', 0), ('revoke', 0);

-- 模拟设备侧 ACL 下发状态
CREATE TABLE device_acls (
    grant_id   TEXT PRIMARY KEY,
    applied    INTEGER NOT NULL DEFAULT 0,         -- 1=规则仍在设备上，0=已回收
    updated_at INTEGER NOT NULL
);

-- 初始数据：来访公司与区域
INSERT INTO companies (id, name) VALUES
    ('co-acme', '安米科技有限公司'),
    ('co-boring', '博宁机械股份公司');

INSERT INTO zones (id, name, is_admin) VALUES
    ('zone-lobby', '大堂会客区', 0),
    ('zone-meeting-3f', '三层会议区', 0),
    ('zone-lounge-5f', '五层休息区', 0),
    ('zone-admin-core', '网管核心机房区', 1);
