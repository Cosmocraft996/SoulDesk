import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createSSRApp, nextTick } from 'vue';
import { renderToString } from 'vue/server-renderer';
import { createServer } from 'vite';

const root = 'components/widgetjs/clock-source/';
const variants = ['clock', 'flip-clock', 'glitch-clock', 'mickey-clock', 'text-clock'];
function defaults(variant) {
  const manifest = JSON.parse(readFileSync(`components/widgetjs/${variant}/manifest.json`));
  return Object.fromEntries(Object.entries(manifest.settingsSchema.properties)
    .filter(([, schema]) => Object.hasOwn(schema, 'default'))
    .map(([key, schema]) => [key, structuredClone(schema.default)]));
}
async function withClock(run) {
  const vite = await createServer({ configFile: resolve(root + 'vite.config.mjs'), server: { middlewareMode: true, hmr: false, watch: null }, appType: 'custom', logLevel: 'error', optimizeDeps: { noDiscovery: true, include: [] } });
  // 原Options API表盘在created启动计时；SSR没有DOM卸载流程，测试环境负责回收。
  const originalInterval = globalThis.setInterval, timers = new Set();
  globalThis.setInterval = (...args) => { const timer = originalInterval(...args); timers.add(timer); return timer; };
  try {
    const { default: App } = await vite.ssrLoadModule('/App.vue');
    const shared = await vite.ssrLoadModule('/souldesk.ts');
    await run({ vite, App, ...shared });
  } finally {
    globalThis.setInterval = originalInterval;
    for (const timer of timers) clearInterval(timer);
    await vite.close();
  }
}
function resetSettings(shared, variant) {
  for (const key of Object.keys(shared.settings)) delete shared.settings[key];
  Object.assign(shared.settings, defaults(variant));
}
async function renderComponent(Component, shared, props = {}) {
  let state;
  const Subject = { ...Component, setup(props, context) { state = Component.setup(props, context); return state; } };
  const app = createSSRApp(Subject, props);
  app.component('component-frame', shared.ComponentFrame);
  return { html: await renderToString(app), state };
}
async function atTime(value, run) {
  const NativeDate = globalThis.Date, fixed = NativeDate.parse(value);
  globalThis.Date = class extends NativeDate {
    constructor(...args) { super(...(args.length ? args : [fixed])); }
    static now() { return fixed; }
  };
  try { return await run(); } finally { globalThis.Date = NativeDate; }
}

test('五款原Vue时钟分支实际渲染，独立主题生效且离线渲染不调用宿主', async () => withClock(async shared => {
  const patterns = { clock: /clock-container/, 'flip-clock': /flipper/, 'glitch-clock': /glitch_container/, 'mickey-clock': /mickey-clock/, 'text-clock': /class="time"/ };
  for (const variant of variants) {
    resetSettings(shared, variant);
    shared.settings.background = '#123456';
    shared.settings.radius = 17;
    const host = { preview: true, request() { throw Error('离线渲染不应请求宿主'); }, ready() { throw Error('SSR没有挂载视图'); } };
    const { html } = await renderComponent(shared.App, shared, { host, variant });
    assert.match(html, /id="clock-root"/);
    assert.match(html, /--widget-background-color:#123456/);
    assert.match(html, /--widget-border-radius:17px/);
    assert.match(html, patterns[variant], variant);
    assert.doesNotMatch(html, /<component-frame[\s>]/);
  }
}));

test('模拟同一真实瞬间：表盘时区变更及午夜回绕执行原计算，不混用本地时间', async () => withClock(async shared => {
  const { default: Clock } = await shared.vite.ssrLoadModule('/widgets/clock/ClockWidgetView.vue');
  resetSettings(shared, 'clock');
  shared.settings.timezone = 'Asia/Shanghai';
  let state;
  await atTime('2026-11-16T15:59:59Z', async () => {
    ({ state } = await renderComponent(Clock, shared));
    assert.equal(state.minuteDeg.value, 354);
    assert.equal(state.hourDeg.value, 359.5);
    assert.equal(state.secondDeg.value, 354);
    shared.settings.timezone = 'UTC';
    await nextTick(); state.updateTime();
    assert.equal(state.hourDeg.value, 119.5);
  });
  shared.settings.timezone = 'Asia/Shanghai';
  await atTime('2026-11-16T16:00:00Z', async () => {
    state.updateTime();
    assert.equal(state.hourDeg.value, 0);
    assert.equal(state.minuteDeg.value, 0);
    assert.equal(state.secondDeg.value % 360, 0);
  });
}));

test('时钟真实保存动作拒绝非法时区，宿主失败保留草稿，成功后才替换配置并关闭', async () => withClock(async shared => {
  resetSettings(shared, 'clock');
  let fail = true, writes = 0, closed = 0;
  const host = { preview: false, async request(method, args) { assert.equal(method, 'settings.update'); writes++; if (fail) throw Error('测试只读库'); return structuredClone(args); } };
  const { state } = await renderComponent(shared.App, shared, { host, variant: 'clock' });
  state.dialog.value = { showModal() {}, close() { closed++; } };
  state.openSettings();
  state.draft.value.timezone = 'invalid/timezone';
  await state.save(new Event('submit'));
  assert.equal(writes, 0);
  assert.equal(closed, 0);
  assert.notEqual(shared.settings.timezone, 'invalid/timezone');
  state.draft.value.timezone = 'UTC';
  state.draft.value.background = '#abcdef';
  state.draft.value.radius = '19';
  await state.save(new Event('submit'));
  assert.equal(writes, 1);
  assert.equal(closed, 0);
  assert.match(state.saveError.value, /测试只读库/);
  assert.equal(state.draft.value.background, '#abcdef');
  assert.notEqual(shared.settings.background, '#abcdef');
  fail = false; await state.save(new Event('submit'));
  assert.equal(writes, 2);
  assert.equal(closed, 1);
  assert.equal(shared.settings.background, '#abcdef');
  assert.equal(shared.settings.radius, 19);
  assert.equal(shared.settings.timezone, 'UTC');
  assert.equal(state.saveError.value, '');
}));
