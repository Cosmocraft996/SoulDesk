// 执行真实打包入口及其动态模块；假宿主仅允许预览握手，不运行外部网站、剪贴板或模型。
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { pathToFileURL } from 'node:url';
import { JSDOM } from 'jsdom';
import { widgetFiles } from './build-widgetjs.mjs';

const cases = [
  ['ai-assistant', '', '网页 AI 助手'],
  ['ai-mindmap', '', '其他说明'],
  ['ai-mermaid', '', '图表类型'],
  ['ai-translate', '', '第二语言'],
  ['clipboard', '', '打开历史记录'],
  ['clipboard-search', '', '打开搜索条'],
  ['clipboard-search', 'clipboard-ai-mindmap', '其他说明'],
  ['clipboard-search', 'clipboard-ai-translate', '第二语言'],
  ['ai-mindmap', '', '其他说明', 'graph'],
  ['ai-mermaid', '', '图表类型', 'graph'],
  ['clipboard-search', 'clipboard-ai-mindmap', '其他说明', 'graph'],
  ['clipboard-search', 'clipboard-ai-translate', '第二语言', 'graph'],
];

if (process.argv.includes('--entry-fixture')) {
  const [, name, page, graph] = process.argv.slice(2), files = widgetFiles(name);
  const root = mkdtempSync(join(tmpdir(), 'souldesk-entry-'));
  writeFileSync(join(root, 'package.json'), '{"type":"module"}');
  for (const [path, bytes] of Object.entries(files)) { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), bytes); }
  const dom = new JSDOM('<!doctype html><html><head></head><body><div id="app"></div></body></html>', { url: `http://widget.fixture/dist/index.html?hostOrigin=http://host.fixture&page=${page}`, pretendToBeVisual: true });
  for (const key of ['window', 'document', 'Element', 'HTMLElement', 'SVGElement', 'Node', 'navigator', 'MutationObserver', 'Event', 'CustomEvent', 'MouseEvent', 'FocusEvent', 'HTMLInputElement', 'HTMLTextAreaElement', 'DOMParser', 'XMLSerializer']) Object.defineProperty(globalThis, key, { value: key === 'window' ? dom.window : key === 'document' ? dom.window.document : dom.window[key], configurable: true });
  globalThis.location = dom.window.location;
  globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
  globalThis.requestAnimationFrame = callback => setTimeout(callback, 0); globalThis.cancelAnimationFrame = clearTimeout;
  dom.window.SVGElement.prototype.getBBox = function () { return { x: 0, y: 0, width: Math.max(40, (this.textContent?.length ?? 0) * 8), height: 24 }; };
  dom.window.SVGElement.prototype.getComputedTextLength = function () { return (this.textContent?.length ?? 0) * 8; };
  Object.defineProperty(dom.window.HTMLElement.prototype, 'scrollWidth', { get() { return Math.max(40, (this.textContent?.length ?? 0) * 8); } });
  Object.defineProperty(dom.window.HTMLElement.prototype, 'scrollHeight', { get() { return 24; } });
  const nativeTimeout = globalThis.setTimeout, nativeInterval = globalThis.setInterval, timers = new Set(), intervals = new Set();
  globalThis.setTimeout = (...args) => { const timer = nativeTimeout(...args); timers.add(timer); return timer; };
  globalThis.setInterval = (...args) => { const timer = nativeInterval(...args); intervals.add(timer); return timer; };
  let network = 0; globalThis.fetch = async () => { network++; throw Error('离线入口禁止外部网络'); };
  const result = name === 'ai-mermaid' ? 'graph TD\nA[已保存开始]-->B[结束]' : page.includes('translate') ? '已保存翻译' : '# 已保存主题\n## 分支甲';
  function response(request) {
    if (request.method === 'ai.status') return { revision: 0, configured: false, enabled: false, hasKey: false, job: null };
    if (request.method === 'ai.toolInfo') return { content: '已保存输入' };
    if (request.method === 'storage.get' || request.method === 'ai.document.get') return { version: 1, content: '已保存输入', result, theme: 'light', history: [] };
    return {};
  }
  let ready;
  const mounted = new Promise(resolve => { ready = resolve; });
  const calls = [], port = { postMessage(request) { calls.push(request.method); queueMicrotask(() => { port.onmessage({ data: { protocol: 1, session: 'fixture', id: request.id, result: response(request) } }); if (request.method === 'runtime.ready') ready(); }); } };
  const parent = { postMessage() { queueMicrotask(() => dom.window.dispatchEvent(new dom.window.MessageEvent('message', { source: parent, origin: 'http://host.fixture', data: { type: 'souldesk:init', protocol: 1, session: 'fixture', instanceId: 'fixture', preview: !graph }, ports: [port] }))); } };
  Object.defineProperty(dom.window, 'parent', { value: parent }); globalThis.parent = parent;
  try {
    await import(pathToFileURL(join(root, 'dist/app.js')).href);
    let timeout;
    try { await Promise.race([mounted, new Promise((_, reject) => { timeout = setTimeout(() => reject(Error('组件没有报告就绪：' + document.body.textContent)), 5000); })]); }
    finally { clearTimeout(timeout); }
    await new Promise(resolve => setTimeout(resolve, 25));
    const expected = cases.find(item => item[0] === name && item[1] === page)[2];
    assert.ok(document.body.textContent.includes(expected), `${name}/${page}没有实际挂载：${document.body.textContent}`);
    if (graph) {
      const translated = page.includes('translate'), selector = name === 'ai-mermaid' ? '.mermaid-content svg' : 'svg.markmap g.markmap-node';
      const end = Date.now() + 8000;
      while (!(translated ? document.body.textContent.includes(result) : document.querySelector(selector)) && Date.now() < end) await new Promise(resolve => setTimeout(resolve, 25));
      assert.ok(translated ? document.body.textContent.includes(result) : document.querySelector(selector), '已保存内容没有恢复成真实图形/翻译：' + document.body.textContent);
      assert.ok(calls.includes(page ? 'ai.document.get' : 'storage.get'));
      if (page) assert.ok(!calls.some(method => method === 'storage.get' || method === 'storage.set'));
      assert.ok(!calls.some(method => ['ai.begin', 'ai.configure', 'ai.clear'].includes(method)));
    } else assert.deepEqual(calls, ['runtime.ready']);
    assert.equal(network, 0); console.log(JSON.stringify({ name, page, graph: !!graph, ready: true, network }));
  } finally {
    dom.window.dispatchEvent(new dom.window.Event('pagehide')); dom.window.close();
    globalThis.setTimeout = nativeTimeout; globalThis.setInterval = nativeInterval;
    for (const timer of timers) clearTimeout(timer); for (const timer of intervals) clearInterval(timer);
  }
} else {
  for (const [name, page, , graph = ''] of cases) test(`独立包真实入口可离线挂载：${name}/${page || '控制卡'}${graph ? '/已保存内容' : ''}`, () => {
    const result = spawnSync(process.execPath, [import.meta.filename, '--entry-fixture', name, page, graph], { encoding: 'utf8', timeout: 25000, windowsHide: true });
    assert.equal(result.status, 0, result.error?.message ?? `${result.stdout}\n${result.stderr}`);
  });
}
