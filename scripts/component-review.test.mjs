import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { request } from 'node:http';
import { JSDOM } from 'jsdom';
import { createReviewServer, loadReviewCatalog } from './component-review.mjs';

async function fixture(t) {
  const parent = resolve('artifacts/component-review-checks'); mkdirSync(parent, { recursive: true });
  const stateDir = mkdtempSync(join(parent, 'run-')), server = createReviewServer({ stateDir });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  t.after(() => new Promise(resolve => { server.closeIdleConnections(); server.close(resolve); }));
  const api = async (path, options) => { const response = await fetch(base + path, options); return { status: response.status, value: await response.json() }; };
  const put = (state, extra = {}) => api('/api/state', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(state), ...extra });
  return { base, api, put, stateDir };
}

test('64 项清单与来源完整，历史源码和本次验收版本保持身份', () => {
  const catalog = loadReviewCatalog();
  assert.equal(catalog.items.length, 64); assert.equal(new Set(catalog.items.map(item => item.id)).size, 64);
  assert.equal(catalog.items.filter(item => item.cloudDeferred).length, 7);
  assert.ok(catalog.items.every(item => /^https:\/\//.test(item.projectUrl) && existsSync(item.sourcePath)));
  assert.match(catalog.items.find(item => item.slug === 'time-progress').pinnedUrl, /714802e6/);
  assert.equal(catalog.build.label, '01aeb817');
});

test('草稿保存、过期写入和非法请求不会混淆；提交保存完整本机快照', async t => {
  const { api, put, stateDir } = await fixture(t);
  const initial = (await api('/api/state')).value;
  const id = 'local.souldesk.widgetjs.coin-medium';
  initial.entries[id] = { decision: 'keep', result: 'partial', notes: '重开后保留\n价格｜曲线', testedVersion: '0.1.1' };
  const saved = await put(initial); assert.equal(saved.status, 200); assert.equal(saved.value.revision, 1);
  assert.equal((await put(initial)).status, 409);
  const invalid = structuredClone(saved.value); invalid.entries[id].decision = 'delete-now';
  assert.equal((await put(invalid)).status, 400);
  assert.equal((await put(saved.value, { headers: { 'Content-Type': 'application/json', Origin: 'https://outside.invalid' } })).status, 403);
  assert.equal((await api('/api/state')).value.entries[id].notes, initial.entries[id].notes);
  assert.equal((await api('/api/source?id=../../package.json')).status, 404);
  const submitted = await api('/api/submit', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(saved.value) });
  assert.equal(submitted.status, 201); assert.equal(submitted.value.revision, 1);
  const report = JSON.parse(readFileSync(submitted.value.files[0], 'utf8'));
  assert.equal(report.items.length, 64); assert.equal(report.items.find(item => item.id === id).notes, initial.entries[id].notes);
  assert.ok(report.items.every(item => item.projectUrl));
  const markdown = readFileSync(submitted.value.files[1], 'utf8'); assert.equal(markdown.split('\n').filter(line => /^\| \d{2} \|/.test(line)).length, 64);
  assert.ok(existsSync(join(stateDir, 'latest-submission.json')));
  const second = createReviewServer({ stateDir });
  await new Promise(resolve => second.listen(0, '127.0.0.1', resolve));
  try { const restored = await (await fetch(`http://127.0.0.1:${second.address().port}/api/state`)).json(); assert.equal(restored.entries[id].decision, 'keep'); }
  finally { await new Promise(resolve => { second.closeIdleConnections(); second.close(resolve); }); }
});

test('网络分片不会破坏中文备注', async t => {
  const { base, api } = await fixture(t); const state = (await api('/api/state')).value;
  state.entries['local.souldesk.widgetjs.coin-medium'].notes = '中文分片验证';
  const bytes = Buffer.from(JSON.stringify(state)), split = bytes.indexOf(Buffer.from('中文')) + 1;
  const response = await new Promise((resolve, reject) => {
    const req = request(base + '/api/state', { method: 'PUT', headers: { 'Content-Type': 'application/json' } }, res => { const parts = []; res.on('data', part => parts.push(part)); res.on('end', () => resolve({ status: res.statusCode, body: JSON.parse(Buffer.concat(parts)) })); });
    req.on('error', reject); req.write(bytes.subarray(0, split)); setTimeout(() => req.end(bytes.subarray(split)), 10);
  });
  assert.equal(response.status, 200); assert.equal(response.body.entries['local.souldesk.widgetjs.coin-medium'].notes, '中文分片验证');
});

test('网页可选择、记录、搜索、恢复并提交；异常保存不冒充成功', async t => {
  const { base, api } = await fixture(t);
  const html = readFileSync('docs/component-review/index.html', 'utf8'), script = readFileSync('docs/component-review/app.js', 'utf8');
  const until = async condition => { for (let i = 0; i < 160; i++) { if (condition()) return; await new Promise(resolve => setTimeout(resolve, 10)); } throw Error('界面条件未满足'); };
  const open = async () => {
    const dom = new JSDOM(html, { url: base, runScripts: 'outside-only', pretendToBeVisual: true });
    dom.window.fetch = (path, options) => fetch(base + path, options);
    dom.window.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
    dom.window.HTMLDialogElement.prototype.close = function () { this.open = false; };
    dom.window.eval(script); await until(() => dom.window.document.querySelectorAll('.record').length === 64); return dom;
  };
  let dom = await open(); t.after(() => dom.window.close());
  const d = dom.window.document, row = d.querySelector('.record'), id = row.dataset.id;
  const input = (element, value) => { if (element.type === 'radio') element.checked = true; else element.value = value; element.dispatchEvent(new dom.window.Event('input', { bubbles: true })); };
  input(row.querySelector('[value="keep"]')); input(row.querySelector('[data-field="result"]'), 'failed');
  input(row.querySelector('textarea'), '<script>不应执行</script>\n重启后需要检查');
  assert.equal(d.querySelector('#keep-count').textContent, '1'); assert.ok(row.querySelector('details').open);
  await until(() => d.querySelector('#save-status').textContent.includes('草稿已保存到本机'));
  input(d.querySelector('#search'), 'AU99.99'); assert.equal(d.querySelectorAll('.record').length, 1);
  assert.ok(d.querySelector('.record').textContent.includes('黄金价格'));
  dom.window.close(); dom = await open();
  const restored = dom.window.document.querySelector(`[data-id="${id}"]`);
  assert.ok(restored.querySelector('[value="keep"]').checked); assert.match(restored.querySelector('textarea').value, /<script>/);
  dom.window.document.querySelector('#submit').click(); assert.ok(dom.window.document.querySelector('#submit-dialog').open);
  dom.window.document.querySelector('#confirm-submit').click();
  await until(() => dom.window.document.querySelector('#toast').textContent.includes('已提交到项目目录'));
  assert.equal((await api('/api/state')).value.entries[id].result, 'failed');
  const realFetch = dom.window.fetch;
  dom.window.fetch = (path, options) => options?.method === 'PUT' ? Promise.resolve({ ok: false, status: 500, json: async () => ({ error: '磁盘写入失败' }) }) : realFetch(path, options);
  const note = restored.querySelector('textarea'); note.value = '模拟保存失败后仍保留的输入'; note.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await until(() => dom.window.document.querySelector('#save-status').textContent.includes('保存失败'));
  assert.ok(JSON.parse(dom.window.localStorage.getItem('souldesk-component-review-v1')).pending);
  assert.equal(note.value, '模拟保存失败后仍保留的输入');
});
