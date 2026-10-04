let userId = 1

export function setUser(id) {
  userId = id
}

async function req(method, url, body) {
  const res = await fetch(url, {
    method,
    headers: {
      'Content-Type': 'application/json',
      'X-User-Id': String(userId)
    },
    body: body === undefined ? undefined : JSON.stringify(body)
  })
  const data = await res.json().catch(() => ({}))
  if (!res.ok) throw new Error(data.error || `请求失败 (${res.status})`)
  return data
}

export const api = {
  me: () => req('GET', '/api/me'),
  zones: () => req('GET', '/api/zones'),
  listVisits: () => req('GET', '/api/visits'),
  createVisit: (v) => req('POST', '/api/visits', v),
  checkin: (id) => req('POST', `/api/visits/${id}/checkin`, {}),
  checkout: (id) => req('POST', `/api/visits/${id}/checkout`, {}),
  cancel: (id) => req('POST', `/api/visits/${id}/cancel`, {}),
  reschedule: (id, from, to) => req('POST', `/api/visits/${id}/reschedule`, { valid_from: from, valid_to: to }),
  reissue: (id) => req('POST', `/api/visits/${id}/reissue`, {}),
  reveal: (id) => req('POST', `/api/auths/${id}/reveal`, {}),
  retryRevoke: (id) => req('POST', `/api/auths/${id}/retry-revoke`, {}),
  notifications: () => req('GET', '/api/notifications'),
  events: () => req('GET', '/api/admin/events'),
  anomalies: () => req('GET', '/api/admin/anomalies'),
  devices: () => req('GET', '/api/admin/devices'),
  netMock: (fail) => req('POST', '/api/admin/network/mock', { fail_revoke: fail }),
  netState: () => req('GET', '/api/admin/network/state'),
  setTime: (payload) => req('POST', '/api/admin/time', payload),
  sweep: () => req('POST', '/api/admin/sweep', {})
}
