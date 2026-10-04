import { writable } from 'svelte/store';

export const toasts = writable([]);
let seq = 0;

export function pushToast(message, kind = 'info', ttl = 4200) {
  const id = ++seq;
  toasts.update((list) => [...list, { id, message, kind }]);
  setTimeout(() => {
    toasts.update((list) => list.filter((t) => t.id !== id));
  }, ttl);
}

export const ok = (m) => pushToast(m, 'ok');
export const err = (m) => pushToast(m, 'err', 6500);
