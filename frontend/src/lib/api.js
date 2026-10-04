// 简单的 API 封装：通过 X-Role 头模拟当前登录角色（前台 / 网络管理员）。

let currentRole = 'front_desk';

export function getRole() {
  return currentRole;
}

export function setRole(r) {
  currentRole = r;
}

export class ApiError extends Error {
  constructor(status, code, message) {
    super(message || code);
    this.status = status;
    this.code = code;
  }
}

export async function api(path, { method = 'GET', body, withRole = true } = {}) {
  const headers = { 'Content-Type': 'application/json' };
  if (withRole) headers['X-Role'] = currentRole;
  const resp = await fetch(path, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined
  });
  const text = await resp.text();
  const data = text ? JSON.parse(text) : {};
  if (!resp.ok) {
    throw new ApiError(resp.status, data.error || 'error', data.message);
  }
  return data;
}
