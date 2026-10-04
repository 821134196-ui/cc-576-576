<script>
  // 访客授权卡片：写清权限限制；数据库里只有后 4 位，完整码默认隐藏。
  // freshCode 仅在“刚刚发码”的响应里存在一次，可点眼睛临时查看。
  export let grant;
  export let zones = [];
  export let freshCode = null;
  export let now = Date.now();
  export let showActions = true;

  import { STATUS_TEXT, STATUS_CLASS, fmtTime, fmtMinutes } from './util.js';

  let revealed = false;

  function zoneName(id) {
    return zones.find((z) => z.id === id)?.name || id;
  }

  let remaining = null;
  $: remaining = grant.status === 'active' && grant.expiresAt != null
    ? Math.max(0, grant.expiresAt - now)
    : null;
  let remainingText = '';
  $: remainingText = remaining == null ? '' : fmtMinutes(Math.ceil(remaining / 60000));

  export let events = { revoke: () => {}, retry: () => {}, activate: () => {}, refresh: () => {} };
</script>

<div class="card grant-card" class:warn-card={grant.effectiveState === 'revoke_pending' || grant.effectiveState === 'provision_failed'}>
  <div class="head">
    <div>
      <span class="badge {STATUS_CLASS[grant.effectiveState] || ''}">
        {STATUS_TEXT[grant.effectiveState] || grant.effectiveState}
      </span>
      {#if grant.effectiveState === 'revoke_pending'}
        <span class="badge st-danger">回收失败 · 需人工处理</span>
      {/if}
    </div>
    <span class="muted small mono">编号 {grant.id.slice(0, 8)}</span>
  </div>

  <!-- 临时码：默认只显示后 4 位 -->
  <div class="code-box">
    {#if freshCode}
      <div>
        <div class="small muted">临时上网码（仅显示这一次，请当面转交访客）</div>
        {#if revealed}
          <span class="code-full mono">{freshCode}</span>
        {:else}
          <span class="code-mask mono">•••• •••• ••{freshCode.slice(-4)}</span>
        {/if}
      </div>
      <button class="ghost sm" on:click={() => (revealed = !revealed)}>
        {revealed ? '隐藏' : '显示完整码'}
      </button>
    {:else}
      <div>
        <div class="small muted">临时上网码</div>
        <span class="code-mask mono">•••• •••• ••{grant.codeLast4}</span>
        <span class="small muted">（完整码不保存，仅发卡时展示一次）</span>
      </div>
    {/if}
  </div>

  <!-- 权限限制，逐条写清 -->
  <dl class="kv">
    <dt>来访公司</dt><dd>{grant.companyName}</dd>
    <dt>可用区域</dt>
    <dd>
      {#each grant.zones as zid}
        <span class="tag" class:admin-zone={zones.find((z) => z.id === zid)?.isAdmin}>{zoneName(zid)}</span>
      {/each}
    </dd>
    <dt>设备数量</dt>
    <dd>限 <b>{grant.deviceLimit}</b> 台 · 已登记 {grant.deviceCount} 台（按设备 MAC 去重，超限拦截）</dd>
    <dt>有效时间</dt>
    <dd>
      时长 <b>{fmtMinutes(grant.durationMinutes)}</b>，自签到激活起算
      {#if grant.validFrom != null}
        <br />生效 {fmtTime(grant.validFrom)} ～ 到期 <b>{fmtTime(grant.expiresAt)}</b>
        {#if remaining != null}
          <span class="small muted">（剩余约 {remainingText}，到期时刻立即失效）</span>
        {/if}
      {:else}
        <span class="small muted">（访客签到后才生效）</span>
      {/if}
    </dd>
    <dt>使用者</dt>
    <dd>
      {#if grant.boundUserId}
        已绑定使用者 <span class="mono">{grant.boundUserId}</span>，他人用此码将被拦截
      {:else}
        未绑定，首次联网时绑定；禁止多人共用
      {/if}
    </dd>
    <dt>设备侧规则</dt>
    <dd>
      {#if grant.aclApplied}
        <span style="color: var(--ok)">● 已下发到网络设备</span>
      {:else}
        <span class="muted">○ 设备上无生效规则（未开通或已回收）</span>
      {/if}
    </dd>
    {#if grant.revokeReason}
      <dt>回收原因</dt><dd class="small">{grant.revokeReason}</dd>
    {/if}
  </dl>

  {#if grant.provisionError}
    <div class="err-box" style="margin-top:8px">
      设备侧操作失败：{grant.provisionError}
      {#if grant.effectiveState === 'revoke_pending'}
        —— 权限尚未真正停用，已列为待处理，不能按“已回收”对待。
      {/if}
    </div>
  {/if}

  {#if showActions}
    <div class="divider"></div>
    <div class="row">
      {#if grant.effectiveState === 'provision_failed'}
        <button class="primary sm" on:click={() => events.activate(grant.id)}>重试开通</button>
      {/if}
      {#if grant.effectiveState === 'revoke_pending'}
        <button class="sm" disabled title="仅网络管理员可在设备恢复后重试">等待网管处理</button>
      {/if}
      {#if ['active', 'pending', 'provision_failed'].includes(grant.effectiveState)}
        <button class="danger sm" on:click={() => events.revoke(grant.id, 'meeting_canceled')}>会议取消</button>
        <button class="danger sm" on:click={() => events.revoke(grant.id, 'reschedule')}>来访改期</button>
      {/if}
      <button class="ghost sm" on:click={() => events.refresh(grant.id)}>刷新状态</button>
    </div>
  {/if}
</div>

<style>
  .grant-card { display: flex; flex-direction: column; gap: 10px; }
  .head { display: flex; justify-content: space-between; align-items: center; gap: 8px; flex-wrap: wrap; }
  .warn-card { border-color: rgba(239,95,107,.45); box-shadow: 0 0 0 1px rgba(239,95,107,.15); }
</style>
