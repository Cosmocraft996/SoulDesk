// 只供显式服务器组件隔离验收使用；数据为固定夹具，不是设备指标。
import { createServer } from 'node:http';
import { writeFileSync } from 'node:fs';
export async function startServerFixture(report) {
  const saved = new Map(), evidence = { fixture: true, listening: false, closed: false, requests: [] };
  const record = () => writeFileSync(report + '.http-fixture.json', JSON.stringify(evidence, null, 2));
  let base;
  const server = createServer(async (request, response) => {
    const path = new URL(request.url, 'http://127.0.0.1').pathname, match = /^\/([a-f0-9-]{36})(\/servers|\/failure)?$/.exec(path);
    let body = '', data, kind = 'invalid';
    const reply = (status, value) => { response.writeHead(status, { 'content-type': 'application/json' }); response.end(JSON.stringify(value)); };
    try {
      for await (const bytes of request) { body += bytes.toString(); if (body.length > 4 * 1024 * 1024) throw Error('夹具请求超限'); }
      if (!match || request.headers.token !== 'souldesk-fixture-token') { reply(403, {}); return; }
      data = body ? JSON.parse(body) : null;
      const endpoint = base + '/' + match[1];
      if (match[2] === '/failure') { kind = 'failure'; reply(503, { fixture: true }); }
      else if (match[2] === '/servers' && request.method === 'POST' && Array.isArray(data)) { kind = 'push'; saved.set(match[1], structuredClone(data)); reply(200, { fixture: true, accepted: true }); }
      else if (match[2] === '/servers' && request.method === 'GET') {
        kind = 'pull'; reply(200, [...(saved.get(match[1]) || []), { id: 'fixture-remote-' + match[1], name: '远端夹具服务器', ip: '127.0.0.2', platform: 'Other', createTime: '2026-01-01T00:00:00Z', api: { url: endpoint, token: 'souldesk-fixture-token', interval: 1 } }]);
      } else if (!match[2] && request.method === 'POST' && data?.url === endpoint && data?.token === 'souldesk-fixture-token') {
        if (data.forwardUrl && data.forwardUrl !== endpoint) throw Error('夹具不转发外部目标');
        kind = data.currentLoad ? 'stats' : 'info';
        reply(200, { fixture: true, cpu: { cores: 4, processors: 1 }, osInfo: { distro: 'Ubuntu', release: 'fixture', arch: 'x64' }, currentLoad: { currentLoad: 36.5 }, mem: { total: 8589934592, active: 4294967296, available: 4294967296 }, fsSize: [{ used: 80530636800, size: 107374182400, use: 75 }] });
      } else { reply(400, { fixture: true }); }
    } catch { if (!response.headersSent) reply(400, { fixture: true }); else response.end(); }
    finally { evidence.requests.push({ at: Date.now(), instance: match?.[1], kind, method: request.method, forwarded: Boolean(data?.forwardUrl), status: response.statusCode }); record(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  base = 'http://127.0.0.1:' + server.address().port; evidence.listening = true; evidence.url = base; record();
  return { url: base, async close() { await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())); evidence.listening = false; evidence.closed = true; record(); } };
}
