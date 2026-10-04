<script>
  import { onMount, onDestroy } from 'svelte'
  import { api, setUser } from './api.js'

  const USERS = [
    { id: 1, name: '前台 reception', role: 'front_desk' },
    { id: 2, name: '网管 netadmin', role: 'network_admin' }
  ]
  const STATUS_TEXT = {
    pending_checkin: '待签到',
    active: '生效中',
    expired: '已过期',
    revoked: '已回收',
    superseded: '已被取代',
    revoke_failed: '回收失败'
  }
  const NET_TEXT = { pending: '未下发', applied: '已下发', revoked: '已回收' }
  const VISIT_TEXT = { booked: '已预约', checked_in: '已签到', checked_out: '已离场', cancelled: '已取消' }
  const RESULT_TEXT = {
    ok: '放行', denied_unknown: '码不存在', denied_not_active: '未签到未生效',
    denied_expired: '已过期', denied_revoked: '已停用', denied_superseded: '旧码已取代',
    denied_not_yet: '未到生效时间', denied_zone: '区域越权', denied_device_limit: '设备超限',
    denied_revoke_pending: '回收待处理拦截'
  }

  let currentUser = USERS[0]
  let zones = []
  let visits = []
  let notifications = []
  let anomalies = []
  let events = []
  let devices = []
  let netState = { fail_revoke: false, applied: [] }
  let error = ''
  let notice = ''
  let revealed = {} // auth_id -> 完整码
  let rescheduling = null // visit id
  let rescheduleFrom = ''
  let rescheduleTo = ''

  // 新建来访表单
  let form = { visitor_name: '', company: '', for_self: false, zone_ids: [], device_limit: 1, valid_from: '', valid_to: '' }

  function toLocalInput(d) {
    const pad = (n) => String(n).padStart(2, '0')
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
  }
  function fmt(s) {
    if (!s) return ''
    return new Date(s).toLocaleString('zh-CN', { hour12: false })
  }

  function resetFormDefaults() {
    const now = new Date()
    form.valid_from = toLocalInput(now)
    form.valid_to = toLocalInput(new Date(now.getTime() + 4 * 3600 * 1000))
  }

  async function run(fn, okMsg) {
    error = ''
    try {
      await fn()
      if (okMsg) { notice = okMsg; setTimeout(() => (notice = ''), 3000) }
      await refresh()
    } catch (e) {
      error = e.message
    }
  }

  async function refresh() {
    try {
      if (currentUser.role === 'front_desk') {
        ;[visits, notifications] = await Promise.all([api.listVisits(), api.notifications()])
      } else {
        ;[anomalies, events, devices, netState] = await Promise.all([
          api.anomalies(), api.events(), api.devices(), api.netState()
        ])
      }
    } catch (e) { /* 轮询失败静默，操作时再报错 */ }
  }

  async function switchUser(u) {
    currentUser = u
    setUser(u.id)
    error = ''
    await refresh()
  }

  async function submitVisit() {
    await run(async () => {
      await api.createVisit({
        visitor_name: form.visitor_name,
        company: form.company,
        visitor_user_id: form.for_self ? currentUser.id : null,
        zone_ids: form.zone_ids.map(Number),
        device_limit: Number(form.device_limit),
        valid_from: new Date(form.valid_from).toISOString(),
        valid_to: new Date(form.valid_to).toISOString()
      })
      form.visitor_name = ''
      form.company = ''
      form.for_self = false
      form.zone_ids = []
    }, '来访已登记，临时码已生成（签到后生效）')
  }

  async function toggleReveal(auth) {
    if (revealed[auth.id]) {
      revealed = { ...revealed, [auth.id]: undefined }
    } else {
      await run(async () => {
        const r = await api.reveal(auth.id)
        revealed = { ...revealed, [auth.id]: r.code }
      })
    }
  }

  function startReschedule(visit) {
    rescheduling = visit.id
    const now = new Date()
    rescheduleFrom = toLocalInput(now)
    rescheduleTo = toLocalInput(new Date(now.getTime() + 4 * 3600 * 1000))
  }

  async function confirmReschedule(visit) {
    await run(async () => {
      await api.reschedule(visit.id, new Date(rescheduleFrom).toISOString(), new Date(rescheduleTo).toISOString())
      rescheduling = null
    }, '已改期，旧码失效并签发新码')
  }

  // 演示用时间控制
  let offsetHours = 0
  async function applyOffset() {
    await run(async () => {
      await api.setTime({ offset_secs: Math.round(Number(offsetHours) * 3600), unfreeze: true })
    }, `时间已偏移 ${offsetHours} 小时`)
  }
  async function resetTime() {
    await run(async () => { await api.setTime({ offset_secs: 0, unfreeze: true }) }, '时间已恢复正常')
  }

  onMount(async () => {
    resetFormDefaults()
    zones = await api.zones()
    await refresh()
    const timer = setInterval(refresh, 4000)
    return () => clearInterval(timer)
  })
</script>

<main>
  <header>
    <h1>🔐 访客网络临时码系统</h1>
    <div class="user-switch">
      {#each USERS as u}
        <button class:active={currentUser.id === u.id} on:click={() => switchUser(u)}>{u.name}</button>
      {/each}
    </div>
  </header>

  {#if error}<div class="banner error">⚠️ {error}</div>{/if}
  {#if notice}<div class="banner ok">✅ {notice}</div>{/if}

  {#if currentUser.role === 'front_desk'}
    <!-- ================= 前台视图 ================= -->
    <section class="card">
      <h2>登记来访 & 发放临时码</h2>
      <form on:submit|preventDefault={submitVisit}>
        <div class="grid">
          <label>访客姓名 <input bind:value={form.visitor_name} required placeholder="张三" /></label>
          <label>来访公司 <input bind:value={form.company} required placeholder="ACME 科技" /></label>
          <label>设备数量
            <select bind:value={form.device_limit}>
              {#each [1, 2, 3, 4, 5] as n}<option value={n}>{n} 台</option>{/each}
            </select>
          </label>
          <label>生效时间 <input type="datetime-local" bind:value={form.valid_from} required /></label>
          <label>失效时间 <input type="datetime-local" bind:value={form.valid_to} required /></label>
          <label class="check"><input type="checkbox" bind:checked={form.for_self} /> 给我自己开通（受越权规则约束）</label>
        </div>
        <fieldset>
          <legend>可用区域</legend>
          {#each zones as z}
            <label class="check">
              <input type="checkbox" value={z.id} bind:group={form.zone_ids} />
              {z.name}{z.is_admin ? '（管理区）' : ''}
            </label>
          {/each}
        </fieldset>
        <button type="submit" class="primary">登记并发码</button>
      </form>
    </section>

    <section>
      <h2>来访记录</h2>
      {#if visits.length === 0}<p class="muted">暂无来访记录</p>{/if}
      {#each visits as v (v.id)}
        <div class="card visit">
          <div class="visit-head">
            <strong>{v.visitor_name}</strong> · {v.company}
            <span class="badge visit-{v.status}">{VISIT_TEXT[v.status] || v.status}</span>
            {#if v.visitor_user_id}<span class="badge self">自用</span>{/if}
            <span class="actions">
              {#if v.status === 'booked'}
                <button on:click={() => run(() => api.checkin(v.id), '已签到，权限已开通')}>签到</button>
              {/if}
              {#if v.status === 'checked_in'}
                <button on:click={() => run(() => api.checkout(v.id), '已离场，权限回收中')}>离场</button>
              {/if}
              {#if v.status === 'booked' || v.status === 'checked_in'}
                <button on:click={() => startReschedule(v)}>改期</button>
                <button on:click={() => run(() => api.reissue(v.id), '已重新发码，旧码失效')}>重新发码</button>
                <button class="danger" on:click={() => run(() => api.cancel(v.id), '会议已取消，权限已回收')}>取消</button>
              {/if}
            </span>
          </div>

          {#if rescheduling === v.id}
            <div class="reschedule">
              新窗口：<input type="datetime-local" bind:value={rescheduleFrom} /> 至
              <input type="datetime-local" bind:value={rescheduleTo} />
              <button class="primary" on:click={() => confirmReschedule(v)}>确认改期</button>
              <button on:click={() => (rescheduling = null)}>取消</button>
            </div>
          {/if}

          {#each v.auths as a (a.id)}
            <div class="auth-card">
              <div class="auth-line">
                <span class="code">
                  {#if revealed[a.id]}
                    <code>{revealed[a.id]}</code>
                  {:else}
                    <code>VST-••••-{a.code_hint}</code>
                  {/if}
                </span>
                <button class="link" on:click={() => toggleReveal(a)}>{revealed[a.id] ? '隐藏' : '显示完整码'}</button>
                <span class="badge st-{a.status}">{STATUS_TEXT[a.status] || a.status}</span>
                <span class="badge net">网络：{NET_TEXT[a.network_state] || a.network_state}</span>
                {#if a.status === 'revoke_failed'}
                  <button class="danger" on:click={() => run(() => api.retryRevoke(a.id), '已重试回收')}>重试回收</button>
                {/if}
              </div>
              <div class="limits">
                区域：{a.zones.join('、')} ｜ 设备：{a.device_count}/{a.device_limit} 台 ｜
                有效期：{fmt(a.valid_from)} ~ {fmt(a.valid_to)}
                {#if a.revoke_reason}｜ 回收原因：{a.revoke_reason}{/if}
              </div>
            </div>
          {/each}
        </div>
      {/each}
    </section>

    <section class="card">
      <h2>通知（开通 / 回收结果）</h2>
      {#if notifications.length === 0}<p class="muted">暂无通知</p>{/if}
      <ul class="notif">
        {#each notifications.slice(0, 10) as n (n.id)}
          <li><span class="muted">{fmt(n.created_at)}</span> {n.message}</li>
        {/each}
      </ul>
    </section>
  {:else}
    <!-- ================= 网管视图 ================= -->
    <section class="card">
      <h2>网络设备模拟</h2>
      <div class="netrow">
        <label class="check">
          <input type="checkbox" checked={netState.fail_revoke}
            on:change={(e) => run(() => api.netMock(e.target.checked), e.target.checked ? '已模拟设备故障（回收将失败）' : '设备恢复正常')} />
          模拟设备故障（回收失败）
        </label>
        <button on:click={() => run(() => api.sweep(), '已执行一轮到期扫描')}>立即扫描到期</button>
        <span>时间偏移演示：</span>
        <input class="offset" type="number" step="1" bind:value={offsetHours} /> 小时
        <button on:click={applyOffset}>应用</button>
        <button on:click={resetTime}>恢复正常时间</button>
      </div>
      <div>
        设备当前下发权限：
        {#if netState.applied.length === 0}<span class="muted">（空）</span>{/if}
        {#each netState.applied as a}
          <span class="badge net">#{a.auth_id} {a.code}</span>
        {/each}
      </div>
    </section>

    <section class="card">
      <h2>⚠️ 异常登录（{anomalies.length}）</h2>
      {#if anomalies.length === 0}<p class="muted">暂无异常</p>{/if}
      {#if anomalies.length > 0}
        <table>
          <thead><tr><th>时间</th><th>临时码</th><th>设备 MAC</th><th>结果</th><th>详情</th></tr></thead>
          <tbody>
            {#each anomalies as e (e.id)}
              <tr class="anomaly">
                <td>{fmt(e.created_at)}</td><td><code>{e.code_used}</code></td><td>{e.mac}</td>
                <td>{RESULT_TEXT[e.result] || e.result}</td><td>{e.detail || ''}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section class="card">
      <h2>设备使用情况（{devices.length}）</h2>
      {#if devices.length === 0}<p class="muted">暂无设备接入</p>{/if}
      {#if devices.length > 0}
        <table>
          <thead><tr><th>授权 #</th><th>MAC</th><th>首次接入</th><th>最近在线</th></tr></thead>
          <tbody>
            {#each devices as d (d.id)}
              <tr><td>#{d.auth_id}</td><td>{d.mac}</td><td>{fmt(d.first_seen)}</td><td>{fmt(d.last_seen)}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section class="card">
      <h2>全部登录事件（最近 {events.length} 条）</h2>
      {#if events.length > 0}
        <table>
          <thead><tr><th>时间</th><th>临时码</th><th>MAC</th><th>区域</th><th>结果</th></tr></thead>
          <tbody>
            {#each events as e (e.id)}
              <tr class:anomaly={e.flagged}>
                <td>{fmt(e.created_at)}</td><td><code>{e.code_used}</code></td><td>{e.mac}</td>
                <td>{e.zone_id ?? '-'}</td><td>{RESULT_TEXT[e.result] || e.result}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</main>

<style>
  :global(body) { margin: 0; font-family: -apple-system, 'PingFang SC', 'Microsoft YaHei', sans-serif; background: #f3f5f7; color: #222; }
  main { max-width: 980px; margin: 0 auto; padding: 16px; }
  header { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; }
  h1 { font-size: 20px; } h2 { font-size: 16px; margin-top: 0; }
  .user-switch button { margin-left: 6px; padding: 6px 12px; border: 1px solid #bbb; background: #fff; border-radius: 6px; cursor: pointer; }
  .user-switch button.active { background: #2563eb; color: #fff; border-color: #2563eb; }
  .card { background: #fff; border-radius: 10px; padding: 16px; margin: 12px 0; box-shadow: 0 1px 3px rgba(0,0,0,.08); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 10px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  label.check { flex-direction: row; align-items: center; gap: 6px; }
  input, select { padding: 6px 8px; border: 1px solid #ccc; border-radius: 6px; font-size: 14px; }
  fieldset { margin: 10px 0; border: 1px solid #ddd; border-radius: 8px; display: flex; gap: 14px; flex-wrap: wrap; }
  legend { font-size: 13px; color: #555; }
  button { padding: 6px 12px; border: 1px solid #bbb; background: #fff; border-radius: 6px; cursor: pointer; font-size: 13px; }
  button.primary { background: #2563eb; color: #fff; border-color: #2563eb; }
  button.danger { background: #dc2626; color: #fff; border-color: #dc2626; }
  button.link { border: none; color: #2563eb; background: none; padding: 2px 4px; }
  .banner { padding: 10px 14px; border-radius: 8px; margin: 10px 0; }
  .banner.error { background: #fee2e2; color: #991b1b; }
  .banner.ok { background: #dcfce7; color: #166534; }
  .visit-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .actions { margin-left: auto; display: flex; gap: 6px; }
  .reschedule { margin: 8px 0; display: flex; gap: 6px; align-items: center; flex-wrap: wrap; font-size: 13px; }
  .auth-card { border: 1px solid #e5e7eb; border-radius: 8px; padding: 10px 12px; margin-top: 8px; background: #fafafa; }
  .auth-line { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .code code { font-size: 15px; background: #eef2ff; padding: 2px 8px; border-radius: 6px; }
  .limits { font-size: 12px; color: #555; margin-top: 6px; }
  .badge { font-size: 12px; padding: 2px 8px; border-radius: 999px; background: #e5e7eb; }
  .badge.st-active { background: #dcfce7; color: #166534; }
  .badge.st-pending_checkin { background: #fef9c3; color: #854d0e; }
  .badge.st-expired, .badge.st-revoked { background: #e5e7eb; color: #4b5563; }
  .badge.st-superseded { background: #ffedd5; color: #9a3412; }
  .badge.st-revoke_failed { background: #fee2e2; color: #991b1b; font-weight: 600; }
  .badge.net { background: #e0f2fe; color: #075985; }
  .badge.self { background: #f3e8ff; color: #6b21a8; }
  .badge.visit-checked_in { background: #dcfce7; color: #166534; }
  .badge.visit-booked { background: #fef9c3; color: #854d0e; }
  .muted { color: #888; font-size: 13px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid #eee; }
  tr.anomaly td { background: #fef2f2; }
  .notif { margin: 0; padding-left: 18px; font-size: 13px; }
  .netrow { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 8px; }
  .offset { width: 70px; }
</style>
