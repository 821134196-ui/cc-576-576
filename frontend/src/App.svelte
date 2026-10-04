<script>
  import { onMount } from 'svelte';
  import FrontDesk from './FrontDesk.svelte';
  import Admin from './Admin.svelte';
  import { api, getRole, setRole } from './lib/api.js';
  import { toasts } from './lib/toast.js';

  let role = getRole();
  let meta = { companies: [], zones: [] };

  function switchRole(r) {
    role = r;
    setRole(r);
  }

  onMount(async () => {
    const d = await api('/api/meta', { withRole: false });
    meta = { companies: d.companies, zones: d.zones };
  });
</script>

<header class="topbar">
  <h1>共享办公 · 访客网络临时码系统</h1>
  <div class="tabs">
    <button class:on={role === 'front_desk'} on:click={() => switchRole('front_desk')}>前台</button>
    <button class:on={role === 'network_admin'} on:click={() => switchRole('network_admin')}>
      网络管理员
    </button>
  </div>
  <span class="pill-role {role === 'front_desk' ? 'pill-front' : 'pill-admin'}" style="margin-left:auto">
    当前身份：{role === 'front_desk' ? '前台（最长 8 小时、不可授权管理员区域）' : '网络管理员'}
  </span>
</header>

<main class="page">
  {#if role === 'front_desk'}
    <FrontDesk {meta} />
  {:else}
    <Admin {meta} />
  {/if}
</main>

<div class="toast">
  {#each $toasts as t (t.id)}
    <div class="t {t.kind}">{t.message}</div>
  {/each}
</div>
