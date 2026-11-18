// 离线执行正式React页面事件；组件会话和宿主调用使用夹具，不替代桌面验收。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import ts from 'typescript';

const dom = new JSDOM('<div id="root"></div>', { url: 'http://fixture.local/?panel=1' });
for (const key of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLDialogElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'location']) {
  Object.defineProperty(globalThis, key, { value: dom.window[key], configurable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class { observe() {} disconnect() {} };
globalThis.IntersectionObserver = class {
  constructor(callback) { this.callback = callback; this.nodes = new Set(); }
  observe(node) {
    this.nodes.add(node);
    queueMicrotask(() => {
      const id = node.dataset.panelInstance;
      if (this.nodes.has(node) && !globalThis.panelFixture?.outside?.has(id)) this.enter(node);
    });
  }
  enter(node) { this.callback([{ target: node, isIntersecting: true }]); }
  unobserve(node) { this.nodes.delete(node); }
  disconnect() { this.nodes.clear(); }
};
Object.defineProperty(dom.window.HTMLElement.prototype, 'clientWidth', { get: () => 600 });
dom.window.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
dom.window.HTMLDialogElement.prototype.close = function () { this.open = false; };
const { createElement: h, act } = await import('react');
const { createRoot } = await import('react-dom/client');
const dataUrl = source => 'data:text/javascript;base64,' + Buffer.from(source).toString('base64');
const palette = JSON.parse(readFileSync(new URL('../src/panel-colors.json', import.meta.url), 'utf8'));
const cache = new Map();
function moduleUrl(name) {
  if (cache.has(name)) return cache.get(name);
  if (name.endsWith('.png')) return dataUrl('export default ' + JSON.stringify(new URL('../src/' + name, import.meta.url).href) + ';');
  if (name.endsWith('.json')) return dataUrl('export default ' + readFileSync(new URL('../src/' + name, import.meta.url), 'utf8') + ';');
  const source = name === 'host' ? 'export const invoke=(...args)=>globalThis.panelFixture.invoke(...args);'
    : name === 'WidgetFrame' ? `import {useEffect,createElement} from 'react';export function WidgetFrame({instance}){useEffect(()=>{const f=globalThis.panelFixture;f.mounted.add(instance);f.mounts.push(instance);return()=>f.mounted.delete(instance);},[instance]);return createElement('div',{'data-instance':instance});}`
    : name === 'NetworkSettings' ? 'export const NetworkSettings=()=>null;'
    : name === 'UpdateSettings' ? 'export const UpdateSettings=()=>null;'
    : readFileSync(new URL(`../src/${name}${name === 'panel-layout' ? '.ts' : '.tsx'}`, import.meta.url), 'utf8');
  let compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX } }).outputText;
  compiled = compiled.replace(/from\s*(['"])([^'"]+)\1/g, (_all, _quote, specifier) => {
    const url = specifier.startsWith('./') ? moduleUrl(specifier.slice(2))
      : specifier === '@tauri-apps/api/core' ? dataUrl('export const isTauri=()=>true;')
      : specifier === '@tauri-apps/api/event' ? dataUrl('export const listen=async(name,callback)=>{const listeners=globalThis.panelFixture?.listeners;if(!listeners)return()=>{};const set=listeners.get(name)??new Set();listeners.set(name,set);set.add(callback);return()=>set.delete(callback);};') : import.meta.resolve(specifier);
    return 'from ' + JSON.stringify(url);
  });
  const url = dataUrl(compiled); cache.set(name, url); return url;
}
const { App } = await import(moduleUrl('App'));
const { PanelCanvas } = await import(moduleUrl('PanelCanvas'));
const { WidgetSurface } = await import(moduleUrl('WidgetSurface'));
const highlighted = node => [...node.querySelectorAll('.snap-highlight.active')].map(n => n.dataset.edge).sort();
const click = async node => { assert.ok(node, '目标按钮存在'); await act(async () => node.click()); };
const input = async (node, value) => { await act(async () => { Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set.call(node, value); node.dispatchEvent(new dom.window.Event('input', { bubbles: true })); }); };
const submit = async node => { await act(async () => node.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }))); };
const button = label => document.querySelector(`[aria-label="${label}"]`);
const textButton = (scope, text) => [...scope.querySelectorAll('button')].find(node => node.textContent === text);
const tab = name => [...document.querySelectorAll('[role="tab"]')].find(node => node.title === name);

async function fixture({ legacy = false, active = null, locked = false, query = '?panel=1', positions = new Map(), outside } = {}) {
  dom.reconfigure({ url: `http://fixture.local/${query}` });
  const state = {
    packages: [{ id: 'clock', name: '时钟', version: '1.0.0', defaultSize: { width: 300, height: 180 }, minSize: { width: 100, height: 80 } }],
    instances: ['a', 'b', 'hidden', 'desktop'].map(id => ({ id, packageId: 'clock', location: id === 'desktop' ? 'desktop' : 'panel', visible: id !== 'hidden', pinned: false, width: null, height: null })),
    panelGroups: { groups: [{ id: 'work', name: '工作', autoColor: palette.presets[0].color, color: null }, { id: 'life', name: '生活', autoColor: palette.presets[1].color, color: null }], active, membership: { a: 'work', hidden: 'work' }, ungroupedColor: null },
    panelLayouts: { a: { x: 30, y: 75, width: 320, height: 200 } },
    panel: { ratio: .4, tint: .12, slideInMs: 250, slideOutMs: 250, desktopDoubleClick: true },
    panelOpen: true,
    panelLocked: locked,
    diagnostics: { shortcut: 'Ctrl+Alt+Space' }, updates: {},
  };
  if (legacy) delete state.panelGroups;
  const f = { state, mounted: new Set(), mounts: [], calls: [], fail: false, gate: undefined, outside, listeners: new Map(),
    async invoke(command, args) {
      f.calls.push([command, args]);
      if (command === 'snapshot') return { ...structuredClone(state), panelScroll: positions.get(state.panelGroups?.active ?? null) };
      if (command === 'panel_view' && args.update.event === 'scroll') { positions.set(args.update.group, args.update.position); return; }
      if (command === 'create_instance' && f.fail) throw Error('测试添加失败');
      if (command === 'set_shortcut' && typeof args.enabled === 'boolean') state.panelShortcutEnabled = args.enabled;
      if (command === 'drag_panel_edge') return f.edgeDragged ?? false;
      if (command === 'window_action' && ['enable-widget-backdrop', 'disable-widget-backdrop'].includes(args.action)) {
        if (f.fail) throw Error('测试衬底保存失败');
        state.widgetBackdrop = args.action === 'enable-widget-backdrop';
        return;
      }
      if (command === 'set_panel') {
        if (f.panelGate) await f.panelGate;
        if (f.failPanel) throw Error('测试偏好保存失败');
        Object.assign(state.panel, args);
        return;
      }
      if (command === 'window_action' && ['lock-panel', 'unlock-panel'].includes(args.action)) {
        if (f.fail) throw Error('测试锁定状态保存失败');
        state.panelLocked = args.action === 'lock-panel';
        return;
      }
      if (command === 'window_action' && args.action === 'resize-panel') {
        if (f.gate) await f.gate;
        if (f.fail) throw Error('测试宽度保存失败');
        state.panel.ratio = .55;
        return;
      }
      if (command !== 'manage_panel_group') return;
      if (f.gate) await f.gate;
      if (f.fail) throw Error('测试保存失败');
      const change = args.change, g = state.panelGroups;
      switch (change.action) {
        case 'select': g.active = change.group; break;
        case 'create': g.groups.push({ id: 'new', name: change.name.trim(), autoColor: palette.presets[2].color, color: null }); g.active = 'new'; break;
        case 'rename': g.groups.find(g => g.id === change.id).name = change.name.trim(); break;
        case 'color': if (change.group === null) g.ungroupedColor = change.color; else g.groups.find(g => g.id === change.group).color = change.color; break;
        case 'reorder': g.groups = change.ids.map(id => g.groups.find(g => g.id === id)); break;
        case 'move': if (change.group) g.membership[change.instance] = change.group; else delete g.membership[change.instance]; break;
        case 'remove': g.groups = g.groups.filter(g => g.id !== change.id); for (const [id, group] of Object.entries(g.membership)) if (group === change.id) delete g.membership[id]; if (g.active === change.id) g.active = null; break;
      }
    },
  };
  globalThis.panelFixture = f;
  const root = createRoot(document.getElementById('root'));
  await act(async () => root.render(h(App)));
  f.close = () => act(async () => root.unmount());
  return f;
}
test.after(() => dom.window.close());

test('按组跨窗口记住两轴滚动，屏外组件滚近才加载，显式恢复可以加载提前指定的目标', async () => {
  const positions = new Map();
  const outside = new Set(['a']);
  const scroller = () => document.querySelector('.panel-scroll');
  const scroll = (left, top) => act(async () => {
    const node = scroller();
    node.scrollLeft = left; node.scrollTop = top;
    node.dispatchEvent(new Event('scroll'));
  });
  const first = await fixture({ query: '?panel=1&view=first', positions, outside });
  try {
    assert.deepEqual([...first.mounted], ['b']);
    await scroll(17, 340);
    await click(tab('工作'));
    assert.equal(scroller().scrollTop, 0);
    assert.ok(document.querySelector('[data-panel-instance="a"] .frame-message'));
    assert.equal(first.mounted.has('a'), false);
    outside.delete('a');
    await scroll(24, 270);
    await act(async () => { for (const notify of first.listeners.get('host:changed')) await notify(); });
    assert.equal(first.mounted.has('a'), true);
    outside.add('a');
    await act(async () => { for (const notify of first.listeners.get('host:changed')) await notify(); });
    assert.equal(first.mounted.has('a'), true);
    await click(tab('未分组'));
    assert.equal(scroller().scrollTop, 340);
    assert.equal(scroller().scrollLeft, 17);
  } finally { await first.close(); }
  const second = await fixture({ query: '?panel=1&view=second', active: 'work', positions, outside });
  try {
    assert.equal(scroller().scrollTop, 270);
    assert.equal(scroller().scrollLeft, 24);
    assert.equal(second.mounted.has('a'), false);
    second.state.panelTarget = 'a';
    await act(async () => { for (const notify of second.listeners.get('host:changed')) await notify(); });
    assert.equal(second.mounted.has('a'), true);
  } finally { await second.close(); }
});

test('锁定从宿主状态恢复，保存失败时保持原状态，锁定时仍可手动收起', async () => {
  const first = await fixture();
  try {
    const lock = button('锁定面板');
    assert.equal(lock.closest('.panel-tools').lastElementChild, lock);
    assert.equal(lock.title, '锁定面板');
    assert.equal(lock.getAttribute('aria-pressed'), 'false');
    const unlockedPath = lock.querySelector('path').getAttribute('d');
    assert.equal(lock.querySelector('svg').getAttribute('fill'), 'none');
    await click(lock);
    assert.equal(first.state.panelLocked, true);
    assert.equal(lock.title, '解锁面板');
    assert.equal(lock.getAttribute('aria-pressed'), 'true');
    assert.notEqual(lock.querySelector('path').getAttribute('d'), unlockedPath);
    assert.equal(lock.querySelector('svg').getAttribute('fill'), 'currentColor');
    const collapse = button('收起组件面板');
    assert.equal(collapse.disabled, false);
    await click(collapse);
    assert.ok(first.calls.some(([command, args]) => command === 'window_action' && args.action === 'collapse'));
  } finally { await first.close(); }

  const reopened = await fixture({ locked: first.state.panelLocked });
  try {
    const lock = button('锁定面板');
    assert.equal(lock.getAttribute('aria-pressed'), 'true');
    reopened.fail = true;
    await click(lock);
    assert.equal(lock.getAttribute('aria-pressed'), 'true');
    assert.match(document.querySelector('.panel-notice').textContent, /锁定状态保存失败/);
    reopened.fail = false;
    await click(lock);
    assert.equal(reopened.state.panelLocked, false);
    assert.equal(lock.getAttribute('aria-pressed'), 'false');
    assert.equal(lock.querySelector('svg').getAttribute('fill'), 'none');
  } finally { await reopened.close(); }
});

test('删除确认和移动分组对话框可取消，实例内容保留', async () => {
  const f = await fixture();
  try {
    await click(button('删除实例'));
    assert.ok(document.querySelector('.remove-instance-dialog[open]'));
    await click(document.querySelector('[data-action="cancel-remove"]'));
    assert.equal(document.querySelector('.remove-instance-dialog'), null);
    assert.equal(f.calls.some(([command, args]) => command === 'window_action' && args.action === 'remove'), false);
    await click(button('移动到分组'));
    assert.ok(document.querySelector('dialog[open]'));
    await click(textButton(document.querySelector('dialog[open]'), '取消'));
    assert.equal(document.querySelector('dialog[open]'), null);
  } finally { await f.close(); }
});

const groupRow = name => [...document.querySelectorAll('.panel-group-row')].find(row => row.querySelector('.panel-group-name')?.textContent.startsWith(name));
const colorOf = name => tab(name).style.getPropertyValue('--group-color');
test('预设配色保存失败保留草稿，成功更新标签；恢复自动、排序和改名保持原自动色', async () => {
  const f = await fixture();
  try {
    assert.equal(colorOf('工作'), palette.presets[0].color);
    assert.equal(colorOf('未分组'), palette.neutral);
    await click(button('管理分组')); await click(textButton(groupRow('工作'), '配色'));
    assert.equal(document.querySelectorAll('.panel-color-swatch').length, 12);
    await click(button('使用紫色'));
    f.fail = true; await submit(document.querySelector('.panel-color-editor'));
    assert.equal(colorOf('工作'), palette.presets[0].color);
    assert.equal(button('十六进制颜色').value, '#BF5AF2');
    assert.match(document.querySelector('dialog [role="alert"]').textContent, /测试保存失败/);
    f.fail = false; await submit(document.querySelector('.panel-color-editor'));
    assert.equal(colorOf('工作'), '#BF5AF2');
    assert.equal(document.querySelector('.panel-color-editor'), null);
    await click(button('前移分组 生活')); assert.equal(colorOf('工作'), '#BF5AF2');
    await click(textButton(groupRow('工作'), '改名')); await input(button('新的分组名称'), '办公'); await submit(document.querySelector('.panel-group-rename'));
    assert.equal(colorOf('办公'), '#BF5AF2');
    await click(textButton(groupRow('办公'), '配色')); await click(textButton(document.querySelector('.panel-color-editor'), '自动配色')); await submit(document.querySelector('.panel-color-editor'));
    assert.equal(colorOf('办公'), palette.presets[0].color);
    assert.equal(f.mounts.filter(id => id === 'b').length, 1, '配色不重建组件');
  } finally { await f.close(); }
});

test('未分组可自定义和恢复灰色，取消不保存，无效色值不能提交', async () => {
  const f = await fixture();
  try {
    await click(button('管理分组')); await click(textButton(groupRow('未分组'), '配色'));
    await input(button('十六进制颜色'), '#123');
    assert.equal(textButton(document.querySelector('.panel-color-editor'), '保存颜色').disabled, true);
    await submit(document.querySelector('.panel-color-editor'));
    assert.equal(f.calls.filter(([command, args]) => command === 'manage_panel_group' && args.change.action === 'color').length, 0);
    await input(button('十六进制颜色'), '#ABC123'); await click(textButton(document.querySelector('.panel-color-editor'), '取消'));
    assert.equal(colorOf('未分组'), palette.neutral);
    await click(textButton(groupRow('未分组'), '配色')); await input(button('十六进制颜色'), '#ABC123'); await submit(document.querySelector('.panel-color-editor'));
    assert.equal(colorOf('未分组'), '#ABC123');
    await click(textButton(groupRow('未分组'), '配色')); await click(textButton(document.querySelector('.panel-color-editor'), '自动配色')); await submit(document.querySelector('.panel-color-editor'));
    assert.equal(colorOf('未分组'), palette.neutral);
    assert.equal(f.state.panelGroups.ungroupedColor, null);
  } finally { await f.close(); }
});

test('黑白和饱和自定义色的标签文字与底色保持足够对比', async () => {
  const f = await fixture({ active: 'work' });
  const luminance = rgb => rgb.map(value => { const v = value / 255; return v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4; }).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
  try {
    await click(button('管理分组'));
    for (const color of ['#FFFFFF', '#000000', '#0000FF', '#00FF00']) {
      await click(textButton(groupRow('工作'), '配色')); await input(button('十六进制颜色'), color); await submit(document.querySelector('.panel-color-editor'));
      assert.equal(colorOf('工作'), color);
      for (const variable of ['--group-bg', '--group-hover', '--group-active']) {
        const rgb = tab('工作').style.getPropertyValue(variable).match(/\d+/g).map(Number);
        assert.ok((luminance([245, 245, 247]) + .05) / (luminance(rgb) + .05) >= 4.5, `${color} 的 ${variable} 可读`);
      }
    }
  } finally { await f.close(); }
});

test('组件库默认添加到面板，只有宿主成功后才提示成功，隐藏全部时禁止添加', async () => {
  const f = await fixture({ query: '' });
  const add = () => textButton(document, '＋ 添加到面板');
  try {
    assert.match(document.querySelector('.heading').textContent, /侧边面板当前分组/);
    await click(add());
    assert.deepEqual(f.calls.find(([command]) => command === 'create_instance'), ['create_instance', { packageId: 'clock' }]);
    assert.equal(document.querySelector('[role="status"]').textContent, '已添加到侧边面板');
    f.fail = true;
    await click(add());
    assert.match(document.querySelector('[role="status"]').textContent, /操作失败：.*测试添加失败/);
    f.state.hiddenAll = true;
    await click(textButton(document, '隐藏全部'));
    assert.equal(add().disabled, true);
  } finally { await f.close(); }
});

test('左边沿主键拖动等待宿主保存，失败撤掉拖动反馈，成功更新比例且不重建组件', async () => {
  const f = await fixture();
  const pointer = (node, which) => act(async () => node.dispatchEvent(new dom.window.MouseEvent('pointerdown', { bubbles: true, cancelable: true, button: which })));
  try {
    await pointer(button('调整面板宽度'), 2);
    assert.equal(f.calls.some(([command, args]) => command === 'window_action' && args.action === 'resize-panel'), false);
    let release; f.gate = new Promise(resolve => release = resolve); f.fail = true;
    await pointer(button('调整面板宽度'), 0);
    assert.ok(document.querySelector('.resizing-width .panel-width-output'));
    assert.equal(button('调整面板宽度').disabled, true);
    await act(async () => release()); f.gate = undefined;
    assert.equal(document.querySelector('.resizing-width'), null);
    assert.match(document.querySelector('[role="status"]').textContent, /测试宽度保存失败/);
    assert.equal(button('调整面板宽度').getAttribute('aria-valuenow'), '40');
    f.fail = false; await pointer(button('调整面板宽度'), 0);
    assert.equal(button('调整面板宽度').getAttribute('aria-valuenow'), '55');
    assert.equal(f.mounts.filter(id => id === 'b').length, 1);
    await click(button('面板设置'));
    assert.equal(button('面板宽度比例'), null);
    assert.match(document.querySelector('.panel-width-hint').textContent, /55%/);
    await input(button('面板背景遮罩'), '25');
    assert.deepEqual(f.calls.filter(([command]) => command === 'set_panel').at(-1)[1], { tint: .25 });
    assert.equal(document.querySelector('.panel-shell').style.getPropertyValue('--panel-tint'), '0.25');
    assert.equal(f.state.panel.ratio, .55);
  } finally { await f.close(); }
});

test('左边沿可用左右方向键微调，只提交宽度，不覆盖其他偏好', async () => {
  const f = await fixture();
  try {
    await act(async () => button('调整面板宽度').dispatchEvent(new dom.window.KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'ArrowLeft' })));
    assert.ok(Math.abs(f.state.panel.ratio - .42) < 1e-9);
    assert.deepEqual(Object.keys(f.calls.find(([command]) => command === 'set_panel')[1]), ['ratio']);
    assert.equal(f.state.panel.tint, .12);
    await act(async () => button('调整面板宽度').dispatchEvent(new dom.window.KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'ArrowRight' })));
    assert.ok(Math.abs(f.state.panel.ratio - .4) < 1e-9);
  } finally { await f.close(); }
});

test('八款挂件选中即保存，左侧停靠时右边沿向右加宽', async () => {
  const f = await fixture();
  try {
    await click(button('面板设置'));
    const form = document.querySelector('.panel-preferences');
    const options = [...form.querySelectorAll('input[name="panel-edge-style"]')];
    assert.deepEqual(options.map(option => option.value), ['classic', 'frost', 'glow', 'paw', 'rocket', 'basketball', 'jordan', 'shinobu']);
    assert.equal(options[0].checked, true);
    assert.equal(form.querySelectorAll('.edge-style-preview img').length, 5);
    await click(options.at(-1));
    assert.equal(f.state.panel.edgeStyle, 'shinobu');
    assert.equal(document.querySelector('.edge [data-edge-style]').dataset.edgeStyle, 'shinobu');
    const visual = { style: 'shinobu', position: { side: 'left', offset: .2 } };
    await act(async () => { for (const listener of f.listeners.get('host:panel-edge')) listener({ payload: visual }); });
    const shell = document.querySelector('.panel-shell');
    assert.equal(shell.dataset.edgeSide, 'left');
    assert.equal(shell.style.getPropertyValue('--edge-offset'), '0.2');
    assert.equal(document.querySelector('.edge [data-edge-style]').dataset.edgeStyle, 'shinobu');
    await act(async () => button('调整面板宽度').dispatchEvent(new dom.window.KeyboardEvent('keydown', { bubbles: true, key: 'ArrowRight' })));
    assert.ok(Math.abs(f.state.panel.ratio - .42) < 1e-9);
  } finally { await f.close(); }
});

test('面板偏好连续改动立即预览，只保存修改字段，失败恢复已保存值', async () => {
  const f = await fixture({ query: '?tab=checks' });
  try {
    assert.equal(document.querySelector('.panel-preferences button'), null);
    let release;
    f.panelGate = new Promise(resolve => { release = resolve; });
    await input(button('面板背景遮罩'), '30');
    await input(button('面板背景遮罩'), '60');
    await input(button('滑入时长'), '80');
    assert.equal(button('面板背景遮罩').value, '60');
    assert.equal(button('滑入时长').value, '80');
    const saves = () => f.calls.filter(([command]) => command === 'set_panel').map(([, args]) => args);
    assert.deepEqual(saves(), [{ tint: .30 }]);
    await act(async () => release());
    assert.deepEqual(saves(), [{ tint: .30 }, { tint: .60, slideInMs: 80 }]);
    assert.equal(f.state.panel.tint, .60);
    assert.equal(f.state.panel.slideInMs, 80);
    f.failPanel = true;
    await input(button('滑入时长'), '90');
    assert.equal(button('滑入时长').value, '80');
    assert.match(document.querySelector('.notice').textContent, /测试偏好保存失败/);
  } finally { await f.close(); }
});

function edgePointer(node, type, { pointerType = 'mouse', buttons = 0 } = {}) {
  const event = new MouseEvent(type, { bubbles: true, button: 0, buttons });
  Object.defineProperties(event, { pointerType: { value: pointerType }, isPrimary: { value: true }, pointerId: { value: 1 } });
  node.dispatchEvent(event);
}

test('收起态挂件单击优先，停留后展开，按住经过和快速移出不展开', async () => {
  const f = await fixture({ query: '?edge=1&style=rocket&side=left' });
  try {
    const toggle = button('展开组件面板');
    assert.equal(toggle.dataset.edgeSide, 'left');
    assert.equal(toggle.querySelector('[data-edge-style]').dataset.edgeStyle, 'rocket');
    const opens = () => f.calls.filter(([command, args]) => command === 'window_action' && args.action.startsWith('edge-')).map(([, args]) => args.action);
    await act(async () => {
      edgePointer(toggle, 'pointerover', { buttons: 1 });
      edgePointer(toggle, 'pointerout');
      edgePointer(toggle, 'pointerover', { pointerType: 'touch' });
      edgePointer(toggle, 'pointerout');
      edgePointer(toggle, 'pointerover');
      edgePointer(toggle, 'pointerout');
      await new Promise(resolve => setTimeout(resolve, 220));
    });
    assert.deepEqual(opens(), []);
    await act(async () => {
      edgePointer(toggle, 'pointerover');
      edgePointer(toggle, 'pointerdown', { buttons: 1 });
      toggle.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, detail: 1 }));
      await new Promise(resolve => setTimeout(resolve, 220));
    });
    assert.deepEqual(opens(), ['edge-activate']);
    await act(async () => {
      edgePointer(toggle, 'pointerout');
      edgePointer(toggle, 'pointerover');
      assert.deepEqual(opens(), ['edge-activate']);
      await new Promise(resolve => setTimeout(resolve, 220));
    });
    assert.deepEqual(opens(), ['edge-activate', 'edge-hover']);
  } finally { await f.close(); }
});

test('管理页主键点击只把普通区域交给外部收起，面板按钮走显式切换', async () => {
  const f = await fixture({ query: '?tab=checks' });
  try {
    const toggle = document.querySelector('[data-panel-toggle]');
    await act(async () => {
      edgePointer(toggle.querySelector('kbd'), 'pointerdown', { buttons: 1 });
      toggle.click();
      edgePointer(document.querySelector('.management-content h1'), 'pointerdown', { buttons: 1 });
    });
    assert.deepEqual(f.calls.filter(([command]) => command === 'window_action').map(([, args]) => args.action), ['toggle', 'outside-panel']);
    const checkbox = [...document.querySelectorAll('input[type="checkbox"]')].find(input => input.labels[0]?.textContent === '启用面板快捷键');
    await click(checkbox);
    assert.equal(f.state.panelShortcutEnabled, false);
    assert.equal(toggle.querySelector('kbd'), null);
    await click(checkbox);
    assert.equal(toggle.querySelector('kbd').textContent, 'Ctrl+Alt+Space');
  } finally { await f.close(); }
});

test('展开后按住挂件仍发起移动，拖完不会误收起，轻点可收起', async () => {
  const f = await fixture();
  try {
    const toggle = button('收起组件面板');
    const collapseCalls = () => f.calls.filter(([command, args]) => command === 'window_action' && args.action === 'collapse');
    const pressAndRelease = () => act(async () => {
      edgePointer(toggle, 'pointerdown', { buttons: 1 });
      toggle.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, detail: 1 }));
    });
    f.edgeDragged = true;
    await pressAndRelease();
    assert.equal(f.calls.filter(([command]) => command === 'drag_panel_edge').length, 1);
    assert.equal(collapseCalls().length, 0);
    f.edgeDragged = false;
    await pressAndRelease();
    assert.equal(collapseCalls().length, 1);
  } finally { await f.close(); }
});

test('全局宿主衬底默认关闭，切换立即保存，保存失败时沿用已保存状态', async () => {
  const f = await fixture({ query: '?tab=checks' });
  try {
    const checkbox = document.querySelector('.settings-panel input[type="checkbox"]');
    assert.equal(checkbox.labels[0].textContent, '显示宿主衬底');
    assert.equal(checkbox.checked, false);
    await click(checkbox);
    assert.equal(f.state.widgetBackdrop, true);
    assert.equal(checkbox.checked, true);
    f.fail = true;
    await click(checkbox);
    assert.equal(checkbox.checked, true);
    assert.match(document.querySelector('.notice').textContent, /测试衬底保存失败/);
    f.fail = false;
    await click(checkbox);
    assert.equal(f.state.widgetBackdrop, false);
    assert.equal(checkbox.checked, false);
  } finally { await f.close(); }
});

test('桌面和面板已展示的组件随全局偏好刷新，无需重建组件会话', async () => {
  for (const [query, selector, instance] of [['?instance=desktop', '.desktop-shell', 'desktop'], ['?panel=1', '.panel-shell', 'b']]) {
    const f = await fixture({ query });
    try {
      const shell = document.querySelector(selector);
      assert.equal(shell.hasAttribute('data-widget-backdrop'), false);
      const rendered = shell.querySelector(`[data-instance="${instance}"]`);
      assert.ok(rendered);
      for (const enabled of [true, false]) {
        f.state.widgetBackdrop = enabled;
        await act(async () => { for (const callback of f.listeners.get('host:changed')) callback(); });
        assert.equal(shell.hasAttribute('data-widget-backdrop'), enabled);
        assert.equal(shell.querySelector(`[data-instance="${instance}"]`), rendered);
      }
    } finally { await f.close(); }
  }
});

test('桌面外壳接收边线事件，清空及切换实例时移除高亮和旧监听', async () => {
  const f = { listeners: new Map(), errors: [], invoke: async () => {} };
  globalThis.panelFixture = f;
  const root = createRoot(document.getElementById('root'));
  const render = instance => act(async () => root.render(h(WidgetSurface, { instance, items: [], onDrag() {}, onResize() {}, onError: message => f.errors.push(message) }, '组件内容')));
  const emit = payload => act(async () => { for (const callback of f.listeners.get('host:snap-edges')) callback({ payload }); });
  try {
    await render('desktop-a');
    await emit(['right', 'bottom', 'invalid']);
    assert.deepEqual(highlighted(document), ['bottom', 'right']);
    assert.equal(document.querySelector('.snap-highlights').getAttribute('aria-hidden'), 'true');
    await emit([]); assert.deepEqual(highlighted(document), []);
    await emit(['left']); await render('desktop-b');
    assert.deepEqual(highlighted(document), []);
    assert.equal(f.listeners.get('host:snap-edges').size, 1);
    assert.deepEqual(f.errors, []);
  } finally { await act(async () => root.unmount()); }
  assert.equal(f.listeners.get('host:snap-edges').size, 0);
});

test('组件工具栏直接提供分组和删除，窄卡片的更多菜单按 Esc 返回入口', async () => {
  const f = await fixture();
  try {
    const surface = document.querySelector('.widget-surface');
    const more = surface.querySelector('[aria-label="更多功能"]');
    const actions = [...surface.querySelectorAll('.surface-toolbar button')];
    assert.deepEqual(actions.map(node => node.ariaLabel), ['放回桌面', '隐藏组件', '移动到分组', '删除实例', '更多功能']);
    assert.ok(actions.slice(-3, -1).every(node => !node.hasAttribute('data-overflow')));
    assert.ok(more.hasAttribute('data-compact-only'));
    await click(more);
    const menu = surface.querySelector('.surface-menu');
    assert.equal(more.getAttribute('aria-expanded'), 'true');
    assert.deepEqual([...menu.querySelectorAll('button')].map(node => node.ariaLabel), ['放回桌面', '隐藏组件']);
    const hide = menu.querySelector('[aria-label="隐藏组件"]');
    await act(async () => {
      hide.focus();
      hide.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    assert.equal(more.getAttribute('aria-expanded'), 'false');
    assert.equal(surface.querySelector('.surface-menu'), null);
    assert.equal(document.activeElement, more);
  } finally { await f.close(); }
});

test('鼠标离开桌面或面板上的组件时收起更多菜单和工具栏', async () => {
  for (const query of ['?instance=desktop', '?panel=1']) {
    const f = await fixture({ query });
    try {
      const surface = document.querySelector('.widget-surface');
      const more = surface.querySelector('[aria-label="更多功能"]');
      const leave = (relatedTarget, pointerType = 'mouse') => act(async () => {
        const event = new MouseEvent('pointerout', { bubbles: true, relatedTarget });
        Object.defineProperty(event, 'pointerType', { value: pointerType });
        more.dispatchEvent(event);
      });
      await act(async () => { more.focus(); more.click(); });
      const menu = surface.querySelector('.surface-menu');
      await leave(menu.querySelector('button'));
      assert.equal(more.getAttribute('aria-expanded'), 'true');
      await leave(document.body, 'touch');
      assert.equal(more.getAttribute('aria-expanded'), 'true');
      await leave(document.body);
      assert.equal(more.getAttribute('aria-expanded'), 'false');
      assert.equal(surface.querySelector('.surface-menu'), null);
      assert.equal(surface.classList.contains('menu-open'), false);
      assert.equal(surface.contains(document.activeElement), false);
    } finally { await f.close(); }
  }
});

async function canvasFixture() {
  const saved = { a: { x: 30, y: 75, width: 200, height: 180 }, b: { x: 330, y: 300, width: 200, height: 180 } };
  const f = { mounted: new Set(), mounts: [], calls: [], errors: [], fail: false,
    async invoke(command, args) {
      f.calls.push([command, args]);
      if (command === 'save_panel_layout' && f.fail) throw Error('测试布局保存失败');
    },
  };
  globalThis.panelFixture = f;
  const root = createRoot(document.getElementById('root'));
  await act(async () => root.render(h(PanelCanvas, { group: null, saved,
    instances: ['a', 'b'].map(id => ({ id, packageId: 'clock', width: 200, height: 180 })),
    packages: [{ id: 'clock', defaultSize: { width: 200, height: 180 }, minSize: { width: 100, height: 80 } }],
    version: () => '1', items: () => [], onError: message => f.errors.push(message),
  })));
  f.article = document.querySelector('.panel-widget');
  for (const node of document.querySelectorAll('.drag-handle,.resize-handle')) {
    node.setPointerCapture = id => { node.captured = id; };
    node.hasPointerCapture = id => node.captured === id;
  }
  f.pointer = async (node, type, x, y) => act(async () => {
    const event = new dom.window.MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y });
    Object.defineProperty(event, 'pointerId', { value: 1 });
    node.dispatchEvent(event);
  });
  f.close = () => act(async () => root.unmount());
  return f;
}

test('面板在最靠下的组件之后额外留出滚动区可视高度的30%', async t => {
  let viewportHeight = 400;
  let resize;
  const originalObserver = globalThis.ResizeObserver;
  t.after(() => { globalThis.ResizeObserver = originalObserver; });
  globalThis.ResizeObserver = class {
    constructor(callback) { resize = callback; }
    observe(node) { Object.defineProperty(node, 'clientHeight', { get: () => viewportHeight }); }
    disconnect() {}
  };
  const f = await canvasFixture();
  try {
    const canvas = document.querySelector('.panel-canvas');
    const last = canvas.querySelectorAll('.panel-widget')[1];
    const bottom = parseFloat(last.style.top) + parseFloat(last.style.height);
    assert.equal(parseFloat(canvas.style.height) - bottom, 12 + 120);
    viewportHeight = 600;
    await act(async () => resize());
    assert.equal(parseFloat(canvas.style.height) - bottom, 12 + 180);
    assert.equal(parseFloat(last.style.top) + parseFloat(last.style.height), bottom);
  } finally { await f.close(); }
});

test('面板指针拖动实时吸附、脱离并保存最终吸附位置', async () => {
  const f = await canvasFixture();
  try {
    const handle = f.article.querySelector('.drag-handle');
    await f.pointer(handle, 'pointerdown', 10, 10);
    assert.deepEqual(highlighted(document), []);
    await f.pointer(handle, 'pointermove', 104, 49);
    assert.equal(f.article.style.left, '130px'); assert.equal(f.article.style.top, '120px');
    assert.deepEqual(highlighted(f.article), ['bottom', 'right']);
    assert.deepEqual(highlighted(document.querySelectorAll('.panel-widget')[1]), ['left', 'top']);
    await f.pointer(handle, 'pointermove', 98, 40);
    assert.equal(f.article.style.left, '118px'); assert.equal(f.article.style.top, '105px');
    assert.deepEqual(highlighted(document), []);
    await f.pointer(handle, 'pointermove', -14, 30);
    assert.deepEqual(highlighted(f.article), ['left']);
    assert.deepEqual(highlighted(document.querySelectorAll('.panel-widget')[1]), []);
    await f.pointer(handle, 'pointermove', 104, 49);
    await f.pointer(handle, 'pointerup', 104, 49);
    assert.deepEqual(highlighted(document), []);
    assert.deepEqual(f.calls.find(([name]) => name === 'save_panel_layout')[1], { instance: 'a', layout: { x: 130, y: 120, width: 200, height: 180 } });
  } finally { await f.close(); }
});

test('面板缩放保存吸附尺寸；取消或保存失败恢复布局', async () => {
  const f = await canvasFixture();
  try {
    const handle = f.article.querySelector('.resize-handle');
    await f.pointer(handle, 'pointerdown', 10, 10);
    await f.pointer(handle, 'pointermove', 104, 49);
    assert.equal(f.article.style.left, '30px'); assert.equal(f.article.style.top, '75px');
    assert.equal(f.article.style.width, '300px'); assert.equal(f.article.style.height, '225px');
    assert.deepEqual(highlighted(f.article), ['bottom', 'right']);
    assert.deepEqual(highlighted(document.querySelectorAll('.panel-widget')[1]), ['left', 'top']);
    await f.pointer(handle, 'pointercancel', 104, 49);
    assert.deepEqual(highlighted(document), []);
    assert.equal(f.article.style.width, '200px');
    assert.ok(!f.calls.some(([name]) => name === 'save_panel_layout'));
    f.fail = true;
    await f.pointer(handle, 'pointerdown', 10, 10);
    await f.pointer(handle, 'pointermove', 104, 49);
    await f.pointer(handle, 'pointerup', 104, 49);
    assert.equal(f.article.style.width, '200px'); assert.equal(f.article.style.height, '180px');
    assert.match(f.errors[0], /布局保存失败/);
    assert.deepEqual(highlighted(document), []);
    f.fail = false;
    await f.pointer(handle, 'pointerdown', 10, 10);
    await f.pointer(handle, 'pointermove', 104, 49);
    await f.pointer(handle, 'pointerup', 104, 49);
    assert.deepEqual(f.calls.filter(([name]) => name === 'save_panel_layout').at(-1)[1].layout, { x: 30, y: 75, width: 300, height: 225 });
    assert.equal(f.article.style.width, '300px'); assert.equal(f.article.style.height, '225px');
  } finally { await f.close(); }
});

test('旧数据默认未分组；新数据只挂载当前组的可见组件，切换恢复原布局', async () => {
  let f = await fixture({ legacy: true });
  try { assert.deepEqual([...f.mounted].sort(), ['a', 'b']); assert.equal(tab('未分组').getAttribute('aria-selected'), 'true'); }
  finally { await f.close(); }
  f = await fixture();
  try {
    assert.deepEqual([...f.mounted], ['b']);
    assert.equal(tab('工作').querySelector('small').textContent, '2');
    await click(tab('工作')); assert.deepEqual([...f.mounted], ['a']);
    assert.equal(document.querySelector('.panel-widget').style.top, '75px');
    await click(tab('生活')); assert.equal(f.mounted.size, 0);
    assert.match(document.querySelector('.panel-empty').textContent, /没有可见组件/);
    await click(tab('工作')); assert.deepEqual([...f.mounted], ['a']);
    assert.equal(document.querySelector('.panel-widget').style.width, '320px');
  } finally { await f.close(); }
});

test('新建分组先等待保存，失败保留草稿与当前组，成功后显示新标签', async () => {
  const f = await fixture();
  try {
    await click(button('管理分组'));
    const field = document.getElementById('panel-group-name'); await input(field, '研究');
    let release; f.gate = new Promise(resolve => release = resolve); f.fail = true;
    await submit(document.querySelector('.panel-group-create'));
    assert.equal(tab('未分组').disabled, true); assert.equal(f.state.panelGroups.groups.length, 2);
    await act(async () => release()); f.gate = undefined;
    assert.match(document.querySelector('dialog [role="alert"]').textContent, /测试保存失败/);
    assert.equal(field.value, '研究'); assert.equal(tab('未分组').getAttribute('aria-selected'), 'true');
    f.fail = false; await submit(document.querySelector('.panel-group-create'));
    assert.equal(field.value, ''); assert.equal(tab('研究').getAttribute('aria-selected'), 'true');
  } finally { await f.close(); }
});

function groupStrip(width = 280) {
  const strip = document.querySelector('.panel-group-tabs');
  const rect = (left, width) => ({ left, right: left + width, width, top: 0, bottom: 36 });
  strip.getBoundingClientRect = () => rect(0, width);
  let scroll = 0;
  Object.defineProperty(strip, 'scrollLeft', {
    get: () => scroll,
    set: value => { scroll = Math.max(0, Math.min(value, strip.children.length * 80 - width)); },
  });
  for (const node of strip.children) {
    node.setPointerCapture = () => {};
    node.getBoundingClientRect = () => rect([...strip.children].indexOf(node) * 80 - scroll, 76);
  }
  return strip;
}

const groupPointer = (node, type, x, y = 15) => act(async () => node.dispatchEvent(new dom.window.PointerEvent(type, {
  bubbles: true, isPrimary: true, pointerId: 1, button: 0, clientX: x, clientY: y,
})));
const pointerClick = node => act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })));
const tabOrder = () => [...document.querySelector('.panel-group-tabs').children].map(node => node.title);

test('拖动顶部标签先预览插入位置，保存失败保留顺序，成功后保留当前组与组件', async () => {
  const f = await fixture({ active: 'work' });
  try {
    groupStrip();
    const original = ['未分组', '工作', '生活'];
    const life = tab('生活');
    const drag = async () => { await groupPointer(life, 'pointerdown', 180); await groupPointer(life, 'pointermove', 95); };
    await drag();
    assert.equal(tab('工作').dataset.dropSide, 'before');
    assert.deepEqual(tabOrder(), original);
    assert.equal(f.calls.some(([name]) => name === 'manage_panel_group'), false);
    let release;
    f.gate = new Promise(resolve => { release = resolve; });
    f.fail = true;
    await groupPointer(life, 'pointerup', 95);
    assert.equal(life.disabled, true);
    assert.deepEqual(tabOrder(), original);
    await act(async () => release());
    assert.match(document.querySelector('.panel-notice').textContent, /测试保存失败/);
    assert.deepEqual(tabOrder(), original);
    assert.equal(life.dataset.dragging, undefined);
    assert.equal(tab('工作').dataset.dropSide, undefined);

    f.fail = false;
    await drag();
    await groupPointer(life, 'pointerup', 95);
    await pointerClick(life);
    assert.deepEqual(tabOrder(), ['未分组', '生活', '工作']);
    assert.deepEqual(f.calls.filter(([name]) => name === 'manage_panel_group').map(([, args]) => args.change), [
      { action: 'reorder', ids: ['life', 'work'] }, { action: 'reorder', ids: ['life', 'work'] },
    ]);
    assert.equal(tab('工作').getAttribute('aria-selected'), 'true');
    assert.deepEqual(f.mounts, ['a']);
    assert.equal(document.querySelector('.panel-widget').style.top, '75px');
  } finally { await f.close(); }
});

test('分组栏支持轻点切换，固定标签、原位松手和取消拖动不提交排序', async () => {
  const f = await fixture();
  try {
    groupStrip();
    const fixed = tab('未分组'), work = tab('工作'), life = tab('生活');
    await groupPointer(fixed, 'pointerdown', 35);
    await groupPointer(fixed, 'pointermove', 200);
    await groupPointer(fixed, 'pointerup', 200);
    assert.equal(fixed.dataset.sortable, undefined);
    await groupPointer(work, 'pointerdown', 98);
    await groupPointer(work, 'pointermove', 110);
    await groupPointer(work, 'pointerup', 110);
    await pointerClick(work);
    assert.equal(tab('未分组').getAttribute('aria-selected'), 'true');

    for (const cancel of ['outside', 'pointercancel', 'Escape']) {
      await groupPointer(life, 'pointerdown', 180);
      await groupPointer(life, 'pointermove', 95);
      if (cancel === 'Escape') await act(async () => life.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: cancel, bubbles: true })));
      await groupPointer(life, cancel === 'pointercancel' ? cancel : 'pointerup', 95, cancel === 'outside' ? 50 : 15);
      await pointerClick(life);
      assert.equal(life.dataset.dragging, undefined);
      assert.equal(tab('未分组').getAttribute('aria-selected'), 'true');
    }
    await groupPointer(life, 'pointerdown', 180);
    await groupPointer(life, 'pointermove', 182);
    await groupPointer(life, 'pointerup', 182);
    await pointerClick(life);
    assert.deepEqual(f.calls.filter(([name]) => name === 'manage_panel_group').map(([, args]) => args.change), [{ action: 'select', group: 'life' }]);
  } finally { await f.close(); }
});

test('拖动到分组栏边缘时可以把标签排到滚动区外的分组之后', async t => {
  const request = window.requestAnimationFrame, cancel = window.cancelAnimationFrame;
  const frames = new Map();
  let nextFrame = 0;
  window.requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame; };
  window.cancelAnimationFrame = id => { frames.delete(id); };
  t.after(() => Object.assign(window, { requestAnimationFrame: request, cancelAnimationFrame: cancel }));
  const f = await fixture();
  try {
    const strip = groupStrip(160);
    const work = tab('工作');
    await groupPointer(work, 'pointerdown', 112);
    await groupPointer(work, 'pointermove', 154);
    for (let i = 0; i < 20 && frames.size; i++) {
      const [id, draw] = frames.entries().next().value;
      frames.delete(id);
      await act(async () => draw());
    }
    assert.equal(strip.scrollLeft, 80);
    assert.equal(tab('生活').dataset.dropSide, 'after');
    await groupPointer(work, 'pointerup', 154);
    assert.deepEqual(tabOrder(), ['未分组', '生活', '工作']);
    assert.equal(frames.size, 0);
  } finally { await f.close(); }
});

test('管理支持排序改名；取消删除保持原组，确认只删除分组并保留组件', async () => {
  const f = await fixture({ active: 'work' });
  try {
    await click(button('管理分组')); await click(button('前移分组 生活'));
    assert.deepEqual(f.state.panelGroups.groups.map(g => g.id), ['life', 'work']);
    const row = [...document.querySelectorAll('.panel-group-row')].find(row => row.textContent.startsWith('工作'));
    await click(textButton(row, '改名')); await input(button('新的分组名称'), '办公');
    await submit(document.querySelector('.panel-group-rename')); assert.ok(tab('办公'));
    assert.equal(f.mounts.filter(id => id === 'a').length, 1, '排序和改名不重建组件');
    const updated = [...document.querySelectorAll('.panel-group-row')].find(row => row.textContent.startsWith('办公'));
    await click(textButton(updated, '删除'));
    await click(textButton(document.querySelector('.panel-group-remove'), '取消')); assert.ok(tab('办公'));
    await click(textButton(updated, '删除')); await click(textButton(document.querySelector('.panel-group-remove'), '确认删除分组'));
    assert.equal(tab('办公'), undefined); assert.equal(f.state.instances.length, 4);
    assert.deepEqual([...f.mounted].sort(), ['a', 'b']);
    assert.equal(f.calls.some(([command, args]) => command === 'window_action' && args.action === 'remove'), false);
  } finally { await f.close(); }
});

test('从组件工具栏移动分组，失败保留选择，成功后可在目标组找到同一实例', async () => {
  const f = await fixture();
  try {
    await click(button('移动到分组'));
    const select = document.getElementById('panel-group-destination');
    await act(async () => { select.value = 'work'; select.dispatchEvent(new dom.window.Event('change', { bubbles: true })); });
    f.fail = true; await submit(document.querySelector('.panel-group-move'));
    assert.equal(select.value, 'work'); assert.deepEqual([...f.mounted], ['b']);
    f.fail = false; await submit(document.querySelector('.panel-group-move'));
    assert.equal(document.querySelector('dialog'), null); assert.equal(f.mounted.size, 0);
    await click(tab('工作')); assert.deepEqual([...f.mounted].sort(), ['a', 'b']);
    assert.equal(f.state.instances.length, 4);
  } finally { await f.close(); }
});
