<script>
  import { onMount, onDestroy } from 'svelte';
  import { api, ApiError } from './lib/api.js';
  import { ok as toastOk, err as toastErr } from './lib/toast.js';
  import GrantCard from './lib/GrantCard.svelte';
  import { fmtTime } from './lib/util.js';

  export let meta = { companies: [], zones: [] };

  let visitors = [];
  let selectedId = null;
  let detail = null; // { visitor, grants }
  let loading = false;
  let now = Date.now();

  // 新建访客
  let nv = { name: '', companyId: '' };
  // 发码表单
  let form = { zoneIds: [], deviceLimit: 2, durationMinutes: 240 };
  let fresh = {}; // grantId -> 完整码（仅本会话内存）
  let formError = '';

  const MAX_MIN = 8 * 60; // 与后端前台上限保持一致

  async function loadVisitors() {
    const d = await api('/api/visitors');
    visitors = d.visitors;
  }

  async function selectVisitor(id) {
    selectedId = id;
    fresh = {};
    await refreshDetail();
  }

  async function refreshDetail() {
    if (!selectedId) return;
    loading = true;
    try {
      detail = await api(`/api/visitors/${selectedId}`);
    } catch (e) {
      toastErr(e.message);
    } finally {
      loading = false;
    }
  }

  async function createVisitor() {
    if (!nv.name.trim() || !nv.companyId) {
      toastErr('请填写访客姓名并选择来访公司');
      return;
    }
    try {
      const d = await api('/api/visitors', {
        method: 'POST',
        body: { name: nv.name.trim(), companyId: nv.companyId }
      });
      toastOk(`已登记访客：${d.visitor.name}`);
      nv = { name: '', companyId: '' };
      await loadVisitors();
      await selectVisitor(d.visitor.id);
    } catch (e) {
      toastErr(e.message);
    }
  }

  function toggleZone(id) {
    form.zoneIds = form.zoneIds.includes(id)
      ? form.zoneIds.filter((z) => z !== id)
      : [...form.zoneIds, id];
  }

  async function issue() {
    formError = '';
    if (form.zoneIds.length === 0) {
      formError = '请至少选择一个可用区域';
      return;
    }
    if (form.durationMinutes > MAX_MIN) {
      formError = `前台最多开通 ${MAX_MIN / 60} 小时，不能开通长期权限`;
      return;
    }
    try {
      const d = await api(`/api/visitors/${selectedId}/grants`, {
        method: 'POST',
        body: {
          zoneIds: form.zoneIds,
          deviceLimit: Number(form.deviceLimit),
          durationMinutes: Number(form.durationMinutes)
        }
      });
      fresh[d.grant.id] = d.code;
      toastOk(d.grant.status === 'active' ? '临时码已开通生效' : '临时码已生成，访客签到后生效');
      form = { zoneIds: [], deviceLimit: 2, durationMinutes: 240 };
      await refreshDetail();
    } catch (e) {
      // 越权 / 非法参数必须原样呈现给前台
      formError = e instanceof ApiError && e.status === 403
        ? `被阻止：${e.message}`
        : e.message;
    }
  }

  async function checkIn() {
    try {
      detail = await api(`/api/visitors/${selectedId}/check-in`, { method: 'POST' });
      const failed = detail.grants.some((g) => g.status === 'provision_failed');
      failed
        ? toastErr('签到完成，但有授权开通失败，请在卡片上重试或联系网管')
        : toastOk('签到成功，待生效授权已开通');
    } catch (e) {
      toastErr(e.message);
    }
  }

  async function checkOut() {
    try {
      detail = await api(`/api/visitors/${selectedId}/check-out`, { method: 'POST' });
      const pending = detail.grants.some((g) => g.status === 'revoke_pending');
      pending
        ? toastErr('已离场，但设备侧回收失败，状态为“待处理”，请勿当作已停用')
        : toastOk('已离场，临时权限回收成功');
      await refreshDetail();
    } catch (e) {
      toastErr(e.message);
    }
  }

  async function revoke(grantId, reason) {
    try {
      const d = await api(`/api/grants/${grantId}/revoke`, {
        method: 'POST',
        body: { reason }
      });
      if (d.grant.status === 'revoke_pending') {
        toastErr('回收失败：网络设备无响应，已列为待处理');
      } else {
        toastOk(reason === 'meeting_canceled' ? '会议已取消，旧码已失效' : '已按改期回收，旧码已失效');
      }
      await refreshDetail();
    } catch (e) {
      toastErr(e.message);
    }
  }

  async function activate(grantId) {
    try {
      await api(`/api/grants/${grantId}/activate`, { method: 'POST' });
      toastOk('已重试开通');
      await refreshDetail();
    } catch (e) {
      toastErr(e.message);
    }
  }

  const timer = setInterval(() => (now = Date.now()), 1000);
  onDestroy(() => clearInterval(timer));

  onMount(loadVisitors);

  function companyName(id) {
    return meta.companies.find((c) => c.id === id)?.name || id;
  }
  $: if (detail) now = detail.now || now;
</script>

<div class="grid fd-grid">
  <!-- 左：访客列表与登记 -->
  <div class="col">
    <div class="card">
      <h2>登记访客</h2>
      <div class="row">
        <div class="field">
          <label>访客姓名</label>
          <input placeholder="如：王小明" bind:value={nv.name} />
        </div>
        <div class="field">
          <label>来访公司</label>
          <select bind:value={nv.companyId}>
            <option value="">请选择</option>
            {#each meta.companies as c}
              <option value={c.id}>{c.name}</option>
            {/each}
          </select>
        </div>
      </div>
      <div style="margin-top:10px"><button class="primary" on:click={createVisitor}>登记</button></div>
    </div>

    <div class="card">
      <h2>今日访客</h2>
      <div class="col" style="gap:6px">
        {#each visitors as v}
          <button class="visitor-btn" class:sel={v.id === selectedId} on:click={() => selectVisitor(v.id)}>
            <span>{v.name}</span>
            <span class="muted small">{companyName(v.companyId)}</span>
            {#if v.checkedIn}
              <span class="badge st-active">已签到</span>
            {:else if v.checkOutAt}
              <span class="badge st-done">已离场</span>
            {:else}
              <span class="badge st-pending">未签到</span>
            {/if}
          </button>
        {/each}
        {#if visitors.length === 0}
          <p class="muted small">还没有访客，先在上方登记。</p>
        {/if}
      </div>
    </div>
  </div>

  <!-- 中：访客详情与操作 -->
  <div class="col">
    {#if !detail}
      <div class="card muted">选择左侧访客查看授权、办理签到/离场。</div>
    {:else}
      <div class="card">
        <div class="row" style="justify-content:space-between;align-items:center">
          <div>
            <h2 style="margin:0">{detail.visitor.name}
              <span class="muted small">{companyName(detail.visitor.companyId)}</span>
            </h2>
            <div class="small muted" style="margin-top:4px">
              签到：{detail.visitor.checkInAt ? fmtTime(detail.visitor.checkInAt) : '—'} ·
              离场：{detail.visitor.checkOutAt ? fmtTime(detail.visitor.checkOutAt) : '—'}
            </div>
          </div>
          <div class="row">
            {#if !detail.visitor.checkedIn && !detail.visitor.checkOutAt}
              <button class="primary" on:click={checkIn}>办理签到</button>
            {/if}
            {#if detail.visitor.checkedIn}
              <button class="danger" on:click={checkOut}>办理离场</button>
            {/if}
            <button class="ghost sm" on:click={refreshDetail}>刷新</button>
          </div>
        </div>
      </div>

      <!-- 发码表单 -->
      <div class="card">
        <h2>开通限时上网权限</h2>
        {#if !detail.visitor.checkedIn}
          <div class="warn-box">访客尚未签到：可先制码，状态为“待签到生效”，签到后才会下发到网络设备。</div>
        {/if}
        <div style="margin-top:10px">
          <label>可用区域（管理员核心机房区不对前台开放）</label>
          <div class="row">
            {#each meta.zones as z}
              <button
                class="zone-btn"
                class:on={form.zoneIds.includes(z.id)}
                class:admin={z.isAdmin}
                disabled={z.isAdmin}
                title={z.isAdmin ? '管理员区域，前台不可授权' : ''}
                on:click={() => toggleZone(z.id)}
              >
                {z.name}{z.isAdmin ? ' 🔒' : ''}
              </button>
            {/each}
          </div>
        </div>
        <div class="row" style="margin-top:10px">
          <div class="field">
            <label>设备数量上限（1–10）</label>
            <input type="number" min="1" max="10" bind:value={form.deviceLimit} />
          </div>
          <div class="field">
            <label>有效时间（分钟，前台最长 8 小时）</label>
            <input type="number" min="1" max={MAX_MIN} bind:value={form.durationMinutes} />
          </div>
        </div>
        {#if formError}
          <div class="err-box" style="margin-top:10px">{formError}</div>
        {/if}
        <div style="margin-top:10px">
          <button class="primary" on:click={issue}>发码并开通</button>
        </div>
      </div>
    {/if}
  </div>

  <!-- 右：授权卡片 -->
  <div class="col">
    {#if detail}
      <h3 style="padding:0 4px">访客授权卡片（{detail.grants.length}）</h3>
      {#each detail.grants as g (g.id)}
        <GrantCard
          grant={g}
          zones={meta.zones}
          freshCode={fresh[g.id] || null}
          {now}
          events={{
            revoke,
            activate,
            refresh: refreshDetail
          }}
        />
      {/each}
      {#if detail.grants.length === 0}
        <div class="card muted small">还没有授权记录。</div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .fd-grid { grid-template-columns: 280px minmax(360px, 1fr) minmax(330px, 420px); align-items: start; }
  .visitor-btn {
    display: grid; grid-template-columns: 1fr auto auto; gap: 8px; align-items: center;
    text-align: left; background: transparent; border: 1px solid transparent;
  }
  .visitor-btn.sel { background: var(--panel-2); border-color: var(--line); }
  .zone-btn { border-radius: 999px; padding: 5px 13px; font-size: 12.5px; }
  .zone-btn.on { background: rgba(79,140,255,.18); border-color: var(--accent); color: #cfe0ff; }
  .zone-btn.admin { opacity: .55; }
</style>
