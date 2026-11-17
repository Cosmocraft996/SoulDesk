import test from 'node:test';
import assert from 'node:assert/strict';
import { verifySmall, verifyMedium } from './verify.js';
import { formatDecimal } from '../metal/metal-model.js';

// 只模拟验收器的恢复分支，不替代真实网络、WebView或桌面验收。
async function withPage(elements, run) {
  const values = { document: { getElementById: id => elements[id] }, location: { search: '?verify=widgetjs-coins' } };
  const previous = Object.fromEntries(Object.keys(values).map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  for (const [key, value] of Object.entries(values)) Object.defineProperty(globalThis, key, { configurable: true, value });
  try { return await run(); }
  finally {
    for (const key of Object.keys(values)) if (previous[key]) Object.defineProperty(globalThis, key, previous[key]); else delete globalThis[key];
  }
}

function hostState(settings, prior) {
  let stored = { verification: structuredClone(prior) };
  const requests = [];
  return {
    host: {
      preview: false, instanceId: '11111111-1111-4111-8111-111111111111',
      async request(name, value) {
        requests.push(name);
        if (name === 'storage.get') return structuredClone(stored);
        if (name === 'settings.get') return structuredClone(settings);
        if (name === 'storage.set') { stored = structuredClone(value); return; }
        throw Error('恢复验收不应修改配置：' + name);
      },
    },
    result: () => stored.verification,
    requests,
  };
}

function smallPage(ready = () => true) {
  const price = '1234.5', settings = { coin: 'ETH-USD', refreshMs: 30000, radius: 20 };
  const quote = { instrument: settings.coin, price, timestamp: Date.now(), reference: '1200', basis: 'open24h' };
  return {
    settings,
    model: { state: { loading: false, quote, error: '', points: [{ price }] } },
    elements: {
      price: { textContent: '$' + formatDecimal(price), title: '原始指数价：' + price },
      identity: { textContent: 'OKX · ETH-USD · 指数价 Index · USD' },
      time: { textContent: '2026/11/15 20:00:00' },
      chart: { width: 150, height: 60, getContext: () => ({ getImageData: () => ({ data: Uint8Array.of(0, 0, 0, ready() ? 255 : 0) }) }) },
      settings: { click() { throw Error('恢复时不得重新执行首次BTC切换测试'); } },
    },
  };
}

test('小组件已有保存基准时，即使临时验收标记失败，也按ETH恢复而不重跑BTC切换', async () => {
  const page = smallPage(), state = hostState(page.settings, { runtimeLoads: 2, testSettings: page.settings, initialUiVerified: true, uiVerified: false });
  await withPage(page.elements, () => verifySmall({ host: state.host, model: page.model, getState: () => ({ settings: page.settings, disposed: false }), isOld: () => false }));
  assert.equal(state.result().uiVerified, true);
  assert.equal(state.result().runtimeLoads, 3);
  assert.equal(state.result().latestQuote.instrument, 'ETH-USD');
  assert.equal(state.requests.includes('settings.update'), false);
});

test('小组件恢复等待画布就绪，不能将初始空画布直接写成最终失败', async () => {
  let ready = false;
  const page = smallPage(() => ready), state = hostState(page.settings, { runtimeLoads: 1, testSettings: page.settings, initialUiVerified: true, uiVerified: true });
  const timer = setTimeout(() => { ready = true; }, 60);
  try {
    await withPage(page.elements, () => verifySmall({ host: state.host, model: page.model, getState: () => ({ settings: page.settings, disposed: false }), isOld: () => false }));
    assert.equal(state.result().uiVerified, true);
    assert.equal(state.result().latestQuote.chart.nonEmpty, true);
    assert.equal(state.result().runtimeLoads, 2);
  } finally { clearTimeout(timer); }
});

test('中组件已有保存基准时，临时验收失败不触发再次修改设置', async () => {
  const settings = { refreshMs: 3000, radius: 20 }, state = hostState(settings, { runtimeLoads: 2, testSettings: settings, initialUiVerified: true, uiVerified: false });
  const quote = { price: '100', timestamp: Date.now(), receivedAt: Date.now(), epoch: 1 };
  const quotes = new Map([['BTC', quote]]), rows = new Map();
  for (let index = 0; index < 14; index++) rows.set(index ? 'TEST' + index : 'BTC', {
    price: { textContent: index ? '—' : '$100.00', title: index ? '' : '原始网页报价：100' },
    meta: { textContent: index ? '等待来源' : '' }, row: { classList: { contains: () => false } },
  });
  const model = { state: { snapshot: { connection: 'connected', quotes, epoch: 1 }, error: '' } };
  await withPage({ settings: { click() { throw Error('恢复时不得重新修改主题'); } } }, () => verifyMedium({ host: state.host, model, display: { quotes }, rows, getState: () => ({ settings, disposed: false }), quoteIsOld: () => false }));
  assert.equal(state.result().uiVerified, true);
  assert.equal(state.result().runtimeLoads, 3);
  assert.equal(state.requests.includes('settings.update'), false);
});

test('首次验证未通过时，不能仅因存在设置基准而绕过BTC切换检查', async () => {
  const page = smallPage(), state = hostState(page.settings, { runtimeLoads: 1, testSettings: page.settings, initialUiVerified: false, uiVerified: false });
  await withPage(page.elements, () => verifySmall({ host: state.host, model: page.model, getState: () => ({ settings: page.settings, disposed: false }), isOld: () => false }));
  assert.equal(state.result().uiVerified, false);
  assert.equal(state.result().phase, 'failed');
  assert.match(state.result().failure, /不得重新执行首次BTC切换测试/);
});
