<script>
  import { onMount, onDestroy } from 'svelte';
  import { api } from './lib/api.js';
  import { ok as toastOk, err as toastErr } from './lib/toast.js';
  import { REASON_TEXT, fmtTime } from './lib/util.js';

  export let meta = { zones: [] };

  let tab = 'anomalies';
  let anomalies = [];
  let events = [];
  let devices = [];
  let pending = [];
  let notifications = [];
  let now = Date.now();

  async function loadAll() {
    try {
      const [a, e, d, p, n] = await Promise.all([
        api('/api/admin/anomalies'),
        api('/api/admin/events'),
        api('/api/admin/devices'),
        api('/api/admin/pending-revokes'),
        api('/api/notifications')
      ]);
      anomalies = a.anomalies;
      events = e.events;
      devices = d.devices;
      pending = p.pendingRevokes;
      notifications = n.notifications;
    } catch (e) {
      toastErr(e.message);
    }
  }

  async function sweep() {
    try {
      const d = await api('/api/admin/sweep', { method: 'POST' });
      d.retired.length
        ? toastOk(`已回收 ${d.retired.length} 条到期授权`)
        : toastOk('没有需要回收的到期授权');
      await loadAll();
    } catch (e) {
      toastErr(e.message);
    }
  }

  async function retryRevoke(id) {
    try {
      const d = await api(`/api/grants/${id}/retry-revoke`, { method: 'POST' });
      if (d.grant.status === 'revoke_pending') {
        toastErr('设备仍未恢复，依旧待处理');
      } else {
        toastOk('设备侧回收成功，授权已终结');
      }
      await loadAll();
    } catch (e) {
      toastErr(e.message);
    }
  }

  async function setFault(kind, armed) {
    try {
      await api('/api/sim/faults', { method: 'POST', body: { kind, armed } });
      toastOk(`模拟设备：${kind} 故障已${armed ? '注入' : '解除'}`);
      faults[kind] = armed;
      faults = faults;
    } catch (e) {
      toastErr(e.message);
    }
  }
  let faults = { provision: false, revoke: false };

  // 模拟设备发起网络认证
  let authForm = { code: '', mac: '', userId: '', zoneId: '' };
  let lastDecision = null;
  async function simulateAuth() {
    try {
      const d = await api('/api/network/auth', {
        method: 'POST',
        withRole: false,
        body: {
          code: authForm.code.trim(),
          mac: authForm.mac.trim(),
          userId: authForm.userId || null,
          zoneId: authForm.zoneId || null
        }
      });
      lastDecision = d;
      await loadAll();
    } catch (e) {
      toastErr(e.message);
    }
  }

  function zoneName(id) {
    return meta.zones.find((z) => z.id === id)?.name || id || '—';
  }

  const timer = setInterval(loadAll, 5000);
  const tick = setInterval(() => (now = Date.now()), 1000);
  onMount(loadAll);
  onDestroy(() => { clearInterval(timer); clearInterval(tick); });
</script>

<div class="grid" style="gap:14px">
  <div class="card row" style="justify-content:space-between;align-items:center">
    <div class="tabs">
      <button class:on={tab === 'anomalies'} on:click={() => (tab = 'anomalies')}>
        异常登录 {#if anomalies.length}<span class="badge st-danger" style="margin-left:4px">{anomalies.length}</span>{/if}
      </button>
      <button class:on={tab === 'devices'} on:click={() => (tab = 'devices')}>设备使用情况</button>
      <button class:on={tab === 'pending'} on:click={() => (tab = 'pending')}>
        回收待处理 {#if pending.length}<span class="badge st-danger" style="margin-left:4px">{pending.length}</span>{/if}
      </button>
      <button class:on={tab === 'events'} on:click={() => (tab = 'events')}>全部认证记录</button>
      <button class:on={tab === 'sim'} on:click={() => (tab = 'sim')}>模拟与通知</button>
    </div>
    <div class="row">
      <button on:click={sweep}>立即扫描到期授权</button>
      <button class="ghost" on:click={loadAll}>刷新</button>
    </div>
  </div>

  {#if tab === 'anomalies'}
    <div class="card">
      <h2>异常登录</h2>
      <table>
        <thead>
          <tr><th>时间</th><th>访客 / 公司</th><th>临时码</th><th>原因</th><th>MAC</th><th>使用者</th><th>区域</th></tr>
        </thead>
        <tbody>
          {#each anomalies as a}
            <tr>
              <td class="small">{fmtTime(a.ts)}</td>
              <td>{a.visitorName || '—'}<div class="muted small">{a.companyName || ''}</div></td>
              <td class="mono">{a.codeLast4 ? `****${a.codeLast4}` : '—'}</td>
              <td><span class="badge st-danger">{REASON_TEXT[a.reason] || a.reason}</span></td>
              <td class="mono small">{a.mac || '—'}</td>
              <td class="mono small">{a.userId || '—'}</td>
              <td class="small">{zoneName(a.zoneId)}</td>
            </tr>
          {/each}
          {#if anomalies.length === 0}
            <tr><td colspan="7" class="muted" style="text-align:center;padding:20px">暂无异常登录</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {/if}

  {#if tab === 'devices'}
    <div class="card">
      <h2>设备使用情况</h2>
      <table>
        <thead>
          <tr><th>访客</th><th>临时码</th><th>MAC</th><th>设备名/使用者</th><th>首次/最近在线</th><th>授权状态</th><th>限额</th></tr>
        </thead>
        <tbody>
          {#each devices as d}
            <tr>
              <td>{d.visitorName}</td>
              <td class="mono">****{d.codeLast4}</td>
              <td class="mono">{d.mac}</td>
              <td class="small">{d.deviceName || '—'}<div class="muted">{d.userId || ''}</div></td>
              <td class="small">{fmtTime(d.firstSeen)}<div class="muted">{fmtTime(d.lastSeen)}</div></td>
              <td>
                <span class="badge" class:st-active={d.grantStatus === 'active'}
                  class:st-done={d.grantStatus !== 'active'}>{d.grantStatus}</span>
              </td>
              <td>{d.deviceLimit} 台</td>
            </tr>
          {/each}
          {#if devices.length === 0}
            <tr><td colspan="7" class="muted" style="text-align:center;padding:20px">还没有设备联网记录</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {/if}

  {#if tab === 'pending'}
    <div class="card">
      <h2>回收失败 · 待处理</h2>
      <p class="muted small" style="margin-top:0">
        这些授权在设备侧的回收调用失败，<b style="color:var(--danger)">不能视为已停用</b>。
        待网络设备恢复后点“重试回收”，系统会把规则真正撤下并落入目标状态。
      </p>
      <table>
        <thead>
          <tr><th>访客</th><th>临时码</th><th>目标状态</th><th>原因</th><th>失败详情</th><th></th></tr>
        </thead>
        <tbody>
          {#each pending as g}
            <tr>
              <td>{g.visitorName}</td>
              <td class="mono">****{g.codeLast4}</td>
              <td><span class="badge st-warn">{g.intendedStatus}</span></td>
              <td class="small">{g.revokeReason}</td>
              <td class="small err-text">{g.provisionError}</td>
              <td><button class="primary sm" on:click={() => retryRevoke(g.id)}>重试回收</button></td>
            </tr>
          {/each}
          {#if pending.length === 0}
            <tr><td colspan="6" class="muted" style="text-align:center;padding:20px">没有待处理项</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {/if}

  {#if tab === 'events'}
    <div class="card">
      <h2>全部认证记录</h2>
      <table>
        <thead>
          <tr><th>时间</th><th>结果</th><th>访客</th><th>原因</th><th>MAC</th><th>区域</th></tr>
        </thead>
        <tbody>
          {#each events as e}
            <tr>
              <td class="small">{fmtTime(e.ts)}</td>
              <td>
                {#if e.result === 'allowed'}
                  <span class="badge st-active">放行</span>
                {:else}
                  <span class="badge" class:st-danger={e.anomaly} class:st-pending={!e.anomaly}>拒绝</span>
                {/if}
              </td>
              <td class="small">{e.visitorName || '—'}</td>
              <td class="small">{REASON_TEXT[e.reason] || e.reason}</td>
              <td class="mono small">{e.mac || '—'}</td>
              <td class="small">{zoneName(e.zoneId)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  {#if tab === 'sim'}
    <div class="grid" style="grid-template-columns:1fr 1fr">
      <div class="col">
        <div class="card">
          <h2>设备联网认证模拟</h2>
          <p class="muted small">模拟访客设备（AC/RADIUS）用临时码联网。同一使用者换第 N 台设备、他人用同码、越区、到期、离场后再用，都会被拦截并记录异常。</p>
          <div class="row">
            <div class="field"><label>临时码</label><input class="mono" bind:value={authForm.code} placeholder="发卡时展示的完整码" /></div>
            <div class="field"><label>设备 MAC</label><input class="mono" bind:value={authForm.mac} placeholder="AA:BB:CC:00:00:01" /></div>
          </div>
          <div class="row" style="margin-top:8px">
            <div class="field"><label>使用者标识（用于多人共用检测）</label><input bind:value={authForm.userId} placeholder="如 user-001" /></div>
            <div class="field">
              <label>接入区域</label>
              <select bind:value={authForm.zoneId}>
                <option value="">不指定</option>
                {#each meta.zones as z}<option value={z.id}>{z.name}</option>{/each}
              </select>
            </div>
          </div>
          <div style="margin-top:10px"><button class="primary" on:click={simulateAuth}>发起认证</button></div>
          {#if lastDecision}
            <div style="margin-top:10px">
              {#if lastDecision.allowed}
                <div class="ok-box">认证通过，已放行（reason: {lastDecision.reason}）</div>
              {:else}
                <div class="err-box">
                  已拦截：{REASON_TEXT[lastDecision.reason] || lastDecision.reason}
                  {#if lastDecision.anomaly}<b> · 已记录为异常登录</b>{/if}
                </div>
              {/if}
            </div>
          {/if}
        </div>

        <div class="card">
          <h2>网络设备故障模拟</h2>
          <p class="muted small">本地模拟 AC/网关。注入故障后，新的开通或回收调用会超时失败，
            用于验证“回收失败必须显示待处理、不能当作已停用”。</p>
          <div class="col">
            <div class="row" style="justify-content:space-between;align-items:center">
              <span>规则下发（provision）</span>
              <button class={faults.provision ? 'danger' : 'primary'}
                on:click={() => setFault('provision', !faults.provision)}>
                {faults.provision ? '故障中 · 点击解除' : '模拟下发故障'}
              </button>
            </div>
            <div class="row" style="justify-content:space-between;align-items:center">
              <span>规则回收（revoke）</span>
              <button class={faults.revoke ? 'danger' : 'primary'}
                on:click={() => setFault('revoke', !faults.revoke)}>
                {faults.revoke ? '故障中 · 点击解除' : '模拟回收故障'}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <h2>通知中心（模拟）</h2>
        <div class="col" style="gap:6px;max-height:360px;overflow:auto">
          {#each notifications as n}
            <div class="noti">
              <span class="tag">{n.channel}</span>
              <span class="tag">{n.kind}</span>
              <div class="small" style="margin-top:3px">{n.message}</div>
              <div class="muted" style="font-size:11px">{fmtTime(n.ts)}</div>
            </div>
          {/each}
          {#if notifications.length === 0}
            <p class="muted small">暂无通知</p>
          {/if}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .err-text { color: #ff9aa2; max-width: 320px; }
  .noti { border-bottom: 1px solid rgba(43,53,82,.5); padding: 6px 2px; }
</style>
