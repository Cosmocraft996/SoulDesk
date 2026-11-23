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
    : readFileSync(new URL(`../src/${name}${name.endsWith('.ts') ? '' : name === 'panel-layout' ? '.ts' : '.tsx'}`, import.meta.url), 'utf8');
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

async function fixture({ legacy = false, active = null, locked = false, query = '?panel=1', positions = new Map(), outside, shortcutRegistered = true, appTheme } = {}) {
  dom.reconfigure({ url: `http://fixture.local/${query}` });
  const state = {
    appTheme,
    packages: [{ id: 'clock', name: '时钟', version: '1.0.0', defaultSize: { width: 300, height: 180 }, minSize: { width: 100, height: 80 } }],
    instances: ['a', 'b', 'hidden', 'desktop'].map(id => ({ id, packageId: 'clock', location: id === 'desktop' ? 'desktop' : 'panel', visible: id !== 'hidden', pinned: false, width: null, height: null })),
    panelGroups: { groups: [{ id: 'work', name: '工作', autoColor: palette.presets[0].color, color: null }, { id: 'life', name: '生活', autoColor: palette.presets[1].color, color: null }], active, membership: { a: 'work', hidden: 'work' }, ungroupedColor: null },
    panelLayouts: { a: { x: 30, y: 75, width: 320, height: 200 } },
    panel: { ratio: .4, tint: .5, slideInMs: 100, slideOutMs: 100, desktopDoubleClick: true },
    panelOpen: true,
    panelLocked: locked,
    panelShortcutRegistered: shortcutRegistered,
    diagnostics: { shortcut: 'Ctrl+Alt+Space' }, updates: {},
  };
  const notificationPreferences = {
    windows: true, island: true, maxVisible: 9, opacity: 100, volume: 100,
    controls: { hidden: false, muted: false },
    shortcuts: { visibility: { value: 'Alt+H', registered: true }, mute: { value: 'Alt+M', registered: true } },
    marketTones: { up: '', down: '' }, marketToneDefaults: { up: '', down: '' }, marketToneGroups: [],
  };
  if (legacy) delete state.panelGroups;
  const f = { state, mounted: new Set(), mounts: [], calls: [], fail: false, gate: undefined, outside, listeners: new Map(),
    async invoke(command, args) {
      f.calls.push([command, args]);
      if (command === 'snapshot') return { ...structuredClone(state), panelScroll: positions.get(state.panelGroups?.active ?? null) };
      if (command === 'notification_settings') {
        const { type, percent } = args.command;
        if (type === 'opacity' || type === 'volume') notificationPreferences[type] = percent;
        return structuredClone(notificationPreferences);
      }
      if (command === 'panel_view' && args.update.event === 'scroll') { positions.set(args.update.group, args.update.position); return; }
      if (command === 'create_instance' && f.fail) throw Error('测试添加失败');
      if (command === 'set_shortcut') {
        if (f.fail) throw Error('快捷键已被占用');
        if (typeof args.enabled === 'boolean') state.panelShortcutEnabled = args.enabled;
        if (args.value) state.diagnostics.shortcut = args.value;
        state.panelShortcutRegistered = state.panelShortcutEnabled !== false;
      }
      if (command === 'drag_panel_edge') return f.edgeDragged ?? false;
      if (command === 'window_action' && args.action === 'set-app-theme') {
        if (f.fail) throw Error('测试主题保存失败');
        state.appTheme = args.theme;
        return;
      }
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

test('锁定从宿主状态恢复，保存失败时保持原状态，锁定时挂件用于置底', async () => {
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
    const collapse = button('置底组件面板');
    assert.equal(collapse.disabled, false);
    await click(collapse);
    assert.ok(first.calls.some(([command, args]) => command === 'window_action' && args.action === 'edge-activate'));
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
  const css = readFileSync(new URL('../src/app-theme.css', import.meta.url), 'utf8');
  const cssDom = new JSDOM(`<style>${css}</style>`);
  const rules = [...cssDom.window.document.styleSheets[0].cssRules];
  const themes = rules.filter(rule => /^\[data-app-theme="[a-z]+"\]$/.test(rule.selectorText));
  const groupRule = rules.find(rule => rule.selectorText === '.panel-group-tab, .panel-color-preview');
  const hexToRgb = hex => hex.trim().slice(1).match(/../g).map(value => parseInt(value, 16));
  try {
    assert.equal(themes.length, 4);
    await click(button('管理分组'));
    for (const color of ['#FFFFFF', '#000000', '#0000FF', '#00FF00']) {
      await click(textButton(groupRow('工作'), '配色')); await input(button('十六进制颜色'), color); await submit(document.querySelector('.panel-color-editor'));
      assert.equal(colorOf('工作'), color);
      for (const theme of themes) {
        const text = luminance(hexToRgb(theme.style.getPropertyValue('--app-text')));
        const card = hexToRgb(theme.style.getPropertyValue('--app-card'));
        for (const variable of ['--group-bg', '--group-hover', '--group-active']) {
          const weight = Number(groupRule.style.getPropertyValue(variable).match(/var\(--group-color\) (\d+)%/)[1]) / 100;
          const background = luminance(hexToRgb(color).map((value, i) => card[i] * (1 - weight) + value * weight));
          assert.ok((Math.max(text, background) + .05) / (Math.min(text, background) + .05) >= 4.5, `${theme.selectorText} ${color} 的 ${variable} 可读`);
        }
      }
    }
  } finally { cssDom.window.close(); await f.close(); }
});

test('应用主题显示已保存选项，成功后切换，失败保留当前主题', async () => {
  const f = await fixture({ query: '?tab=checks', appTheme: 'pine' });
  const choice = id => document.querySelector(`input[name="app-theme"][value="${id}"]`);
  try {
    assert.equal(document.documentElement.dataset.appTheme, 'pine');
    assert.equal(choice('pine').checked, true);
    assert.equal(document.querySelectorAll('input[name="app-theme"]').length, 4);
    await click(choice('oat'));
    assert.equal(f.state.appTheme, 'oat');
    assert.equal(document.documentElement.dataset.appTheme, 'oat');
    assert.equal(choice('oat').checked, true);
    f.fail = true;
    await click(choice('iris'));
    assert.equal(document.documentElement.dataset.appTheme, 'oat');
    assert.equal(choice('oat').checked, true);
    assert.match(document.querySelector('.notice').textContent, /测试主题保存失败/);
  } finally { await f.close(); }
});

test('侧边面板随宿主事件切换主题并保留组件会话与自定义分组色', async () => {
  const f = await fixture({ active: 'work' });
  try {
    assert.equal(document.documentElement.dataset.appTheme, 'ink');
    const widget = document.querySelector('[data-instance="a"]');
    assert.ok(widget);
    const groupColor = colorOf('工作');
    f.state.appTheme = 'iris';
    await act(async () => { for (const notify of f.listeners.get('host:changed')) await notify(); });
    assert.equal(document.documentElement.dataset.appTheme, 'iris');
    assert.equal(document.querySelector('[data-instance="a"]'), widget);
    assert.deepEqual(f.mounts, ['a']);
    assert.equal(colorOf('工作'), groupColor);
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

test('组件库允许网页文件拖放，仅将导入区内的单个 ZIP 字节送给宿主', async () => {
  const f = await fixture({ query: '' });
  const zone = button('导入组件');
  const picker = textButton(zone, '导入本地 ZIP');
  const zip = new File(['测试 ZIP 内容'], '计时器.ZIP');
  const drag = async (node, type, files = [zip], relatedTarget = null) => {
    const event = new dom.window.MouseEvent(type, { bubbles: true, cancelable: true, relatedTarget });
    const transfer = { types: ['Files'], files, dropEffect: 'none' };
    Object.defineProperty(event, 'dataTransfer', { value: transfer });
    await act(async () => node.dispatchEvent(event));
    return { prevented: event.defaultPrevented, effect: transfer.dropEffect };
  };
  const imports = () => f.calls.filter(([command]) => command === 'import_package');
  const originalInvoke = f.invoke;
  let finish;
  const pending = new Promise(resolve => { finish = resolve; });
  f.invoke = async (command, args) => {
    if (command !== 'import_package') return originalInvoke(command, args);
    f.calls.push([command, args]);
    if (!args) return null;
    if (f.fail) throw Error('测试 ZIP 校验失败');
    await pending;
    const pkg = { ...f.state.packages[0], id: 'timer', name: '计时器' };
    f.state.packages.push(pkg);
    return pkg;
  };
  try {
    assert.deepEqual(await drag(zone, 'dragover'), { prevented: true, effect: 'copy' });
    assert.equal(zone.hasAttribute('data-drag-over'), true);
    await drag(zone, 'dragleave', [zip], picker);
    assert.equal(zone.hasAttribute('data-drag-over'), true);
    await drag(zone, 'dragleave');
    assert.equal(zone.hasAttribute('data-drag-over'), false);
    const outside = document.querySelector('.topbar');
    assert.deepEqual(await drag(outside, 'dragover'), { prevented: true, effect: 'none' });
    assert.equal((await drag(outside, 'drop')).prevented, true);
    assert.equal(imports().length, 0);

    for (const files of [[new File([], '说明.txt')], [zip, zip]]) {
      await drag(zone, 'drop', files);
      assert.match(document.querySelector('.notice').textContent, /一次拖入一个组件 ZIP 文件/);
    }
    await drag(zone, 'drop', [{ name: '过大.zip', size: 20 * 1024 * 1024 + 1 }]);
    assert.match(document.querySelector('.notice').textContent, /超过 20 MiB/);
    assert.equal(imports().length, 0);

    await drag(picker, 'drop');
    assert.equal(picker.disabled, true);
    assert.deepEqual(await drag(zone, 'dragover'), { prevented: true, effect: 'none' });
    await drag(zone, 'drop');
    assert.equal(imports().length, 1);
    assert.equal(new TextDecoder().decode(imports()[0][1]), '测试 ZIP 内容');
    assert.equal(zone.hasAttribute('data-drag-over'), false);
    await act(async () => finish());
    assert.equal(document.querySelector('.notice').textContent, '已导入“计时器”');
    assert.ok([...document.querySelectorAll('.package h2')].some(title => title.textContent === '计时器'));

    f.fail = true;
    await drag(zone, 'drop');
    assert.match(document.querySelector('.notice').textContent, /测试 ZIP 校验失败/);
    await click(picker);
    assert.deepEqual(imports().at(-1), ['import_package', undefined]);
    assert.equal(document.querySelector('.notice'), null);

    await click([...document.querySelectorAll('nav button')].find(node => node.textContent.endsWith('设置')));
    assert.equal((await drag(document.querySelector('.topbar'), 'drop')).prevented, true);
    assert.equal(imports().length, 3);
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
    assert.equal(button('面板背景遮罩').value, '50');
    assert.equal(button('滑入时长').value, '100');
    assert.equal(button('滑出时长').value, '100');
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
    assert.equal(f.state.panel.tint, .5);
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

test('灵动通知滑杆可设置全透明和零音量，松开各自保存', async () => {
  const f = await fixture({ query: '?tab=checks' });
  try {
    const opacity = document.querySelector('#island-opacity');
    const volume = document.querySelector('#island-volume');
    for (const slider of [opacity, volume]) {
      assert.equal(slider.type, 'range');
      assert.equal(slider.min, '0');
      assert.equal(slider.max, '100');
    }
    const changes = () => f.calls.filter(([name, args]) => name === 'notification_settings' && args.command.type !== 'get').map(([, args]) => args.command);
    const commit = slider => act(async () => slider.dispatchEvent(new Event('change', { bubbles: true })));
    await input(opacity, '100');
    assert.deepEqual(changes(), []);
    await commit(opacity);
    assert.equal(opacity.value, '100');
    assert.equal(volume.value, '100');
    await input(volume, '0');
    await commit(volume);
    assert.deepEqual(changes(), [{ type: 'opacity', percent: 0 }, { type: 'volume', percent: 0 }]);
    assert.equal(opacity.value, '100');
    assert.equal(volume.value, '0');
  } finally { await f.close(); }
});

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

test('面板快捷键未注册时可从管理页打开设置并重试原键', async () => {
  const f = await fixture({ query: '', shortcutRegistered: false });
  try {
    const toggle = document.querySelector('[data-panel-toggle]');
    assert.equal(toggle.querySelector('kbd'), null);
    assert.match(document.querySelector('[role="alert"]').textContent, /Ctrl\+Alt\+Space 未能注册/);
    await click(textButton(document, '快捷键设置'));
    const field = document.querySelector('#shortcut');
    assert.equal(field.value, 'Ctrl+Alt+Space');
    const save = textButton(field.closest('.setting-row'), '保存');
    f.fail = true;
    await click(save);
    assert.match(document.querySelector('[role="alert"]').textContent, /未能注册/);
    assert.equal(toggle.querySelector('kbd'), null);
    f.fail = false;
    await click(save);
    assert.deepEqual(f.calls.filter(([command]) => command === 'set_shortcut').map(([, args]) => args), [
      { value: 'Ctrl+Alt+Space' }, { value: 'Ctrl+Alt+Space' },
    ]);
    assert.equal(document.querySelector('[role="alert"]'), null);
    assert.equal(toggle.querySelector('kbd').textContent, 'Ctrl+Alt+Space');
  } finally { await f.close(); }
});

test('面板快捷键录入完整组合，取消还原已保存的键位，冲突后仍可修改', async () => {
  const f = await fixture({ query: '?tab=checks' });
  const field = document.querySelector('#shortcut');
  const save = textButton(field.closest('.setting-row'), '保存');
  const shortcutCalls = () => f.calls.filter(([name]) => name === 'set_shortcut').map(([, args]) => args);
  const press = async (key, code, options = {}, type = 'keydown') => {
    const event = new dom.window.KeyboardEvent(type, { key, code, bubbles: true, cancelable: true, ...options });
    await act(async () => field.dispatchEvent(event));
    return event.defaultPrevented;
  };
  try {
    assert.equal(field.readOnly, true);
    await act(async () => field.focus());
    await press('u', 'KeyU');
    await press('F10', 'F10');
    assert.equal(field.value, 'Ctrl+Alt+Space');
    assert.equal(save.disabled, true);
    await press('Control', 'ControlLeft', { ctrlKey: true });
    assert.equal(field.value, 'Ctrl+…');
    assert.equal(save.disabled, true);
    await press('Alt', 'AltLeft', { ctrlKey: true, altKey: true });
    assert.equal(field.value, 'Ctrl+Alt+…');
    await press('Alt', 'AltLeft', { ctrlKey: true }, 'keyup');
    assert.equal(field.value, 'Ctrl+…');
    await press('Control', 'ControlLeft', {}, 'keyup');
    await press('Process', 'KeyU', { isComposing: true });
    assert.equal(field.value, 'Ctrl+Alt+Space');
    // 键盘布局返回的字母与物理键位不同时，使用用户实际输入的字母。
    assert.equal(await press('A', 'KeyQ', { ctrlKey: true, shiftKey: true }), true);
    assert.equal(field.value, 'Ctrl+Shift+A');
    assert.equal(save.disabled, false);
    assert.deepEqual(shortcutCalls(), []);
    assert.equal(await press('Tab', 'Tab', { shiftKey: true }), false);
    await press('Escape', 'Escape');
    assert.equal(field.value, 'Ctrl+Alt+Space');
    assert.equal(save.disabled, true);
    assert.notEqual(document.activeElement, field);

    await act(async () => field.focus());
    await press('1', 'Numpad1', { altKey: true });
    assert.equal(field.value, 'Alt+Numpad1');
    f.fail = true;
    await click(save);
    assert.equal(f.state.diagnostics.shortcut, 'Ctrl+Alt+Space');
    assert.equal(field.value, 'Alt+Numpad1');
    assert.match(document.querySelector('.notice').textContent, /快捷键已被占用/);

    f.fail = false;
    await act(async () => field.focus());
    await press('Delete', 'NumpadDecimal', { ctrlKey: true });
    assert.equal(field.value, 'Ctrl+Delete');
    await press('ArrowUp', 'Numpad8', { ctrlKey: true, altKey: true });
    await press('F10', 'F10', { repeat: true });
    assert.equal(field.value, 'Ctrl+Alt+ArrowUp');
    await click(save);
    assert.deepEqual(shortcutCalls(), [{ value: 'Alt+Numpad1' }, { value: 'Ctrl+Alt+ArrowUp' }]);
    assert.equal(f.state.diagnostics.shortcut, 'Ctrl+Alt+ArrowUp');
    assert.equal(document.querySelector('[data-panel-toggle] kbd').textContent, 'Ctrl+Alt+ArrowUp');
  } finally { await f.close(); }
});

test('展开后按住挂件仍发起移动，拖完不会误收起，轻点可收起', async () => {
  const f = await fixture();
  try {
    const toggle = button('收起组件面板');
    const activateCalls = () => f.calls.filter(([command, args]) => command === 'window_action' && args.action === 'edge-activate');
    const pressAndRelease = () => act(async () => {
      edgePointer(toggle, 'pointerdown', { buttons: 1 });
      toggle.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, detail: 1 }));
    });
    f.edgeDragged = true;
    await pressAndRelease();
    assert.equal(f.calls.filter(([command]) => command === 'drag_panel_edge').length, 1);
    assert.equal(activateCalls().length, 0);
    f.edgeDragged = false;
    await pressAndRelease();
    assert.equal(activateCalls().length, 1);
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

test('面板在最靠下的组件之后额外留出滚动区可视高度的50%', async t => {
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
    assert.equal(parseFloat(canvas.style.height) - bottom, 12 + 200);
    viewportHeight = 600;
    await act(async () => resize());
    assert.equal(parseFloat(canvas.style.height) - bottom, 12 + 300);
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
    await f.pointer(handle, 'pointerdown', 10, 10);
    await f.pointer(handle, 'pointermove', 180, 95);
    await f.pointer(handle, 'pointercancel', 180, 95);
    assert.equal(f.article.style.width, '300px', '下一次取消拖动时恢复刚保存的尺寸');
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

function groupStrip(width = 280, tabWidths = [76, 76, 76]) {
  const strip = document.querySelector('.panel-group-tabs');
  const rect = (left, width) => ({ left, right: left + width, width, top: 0, bottom: 36 });
  strip.getBoundingClientRect = () => rect(0, width);
  strip.style.columnGap = '4px';
  let scroll = 0;
  Object.defineProperty(strip, 'scrollLeft', {
    get: () => scroll,
    set: value => { scroll = Math.max(0, Math.min(value, tabWidths.reduce((sum, item) => sum + item + 4, 0) - width)); },
  });
  for (const [index, node] of [...strip.children].entries()) {
    node.setPointerCapture = () => {};
    Object.defineProperties(node, {
      offsetWidth: { value: tabWidths[index] },
      offsetLeft: { get: () => [...strip.children].slice(0, [...strip.children].indexOf(node)).reduce((left, tab) => left + tab.offsetWidth + 4, 0) },
    });
    node.getBoundingClientRect = () => {
      const shift = parseFloat(node.style.transform.slice('translateX('.length)) || 0;
      return rect(node.offsetLeft - scroll + shift, node.offsetWidth);
    };
  }
  return strip;
}

const groupPointer = (node, type, x, y = 15) => act(async () => node.dispatchEvent(new dom.window.PointerEvent(type, {
  bubbles: true, isPrimary: true, pointerId: 1, button: 0, clientX: x, clientY: y,
})));
const pointerClick = node => act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })));
const tabOrder = () => [...document.querySelector('.panel-group-tabs').children].map(node => node.title);
const floatingTab = () => document.querySelector('.panel-group-floating');

function mockGroupHitTest(t, strip) {
  const previous = document.elementFromPoint;
  const contains = (element, x, y) => {
    const { left, right, top, bottom } = element.getBoundingClientRect();
    return x >= left && x < right && y >= top && y < bottom;
  };
  document.elementFromPoint = (x, y) => {
    if (!contains(strip, x, y)) return document.body;
    const group = [...strip.children].find(node => contains(node, x, y));
    return group?.firstElementChild ?? strip;
  };
  t.after(() => {
    if (previous) document.elementFromPoint = previous;
    else delete document.elementFromPoint;
  });
}

test('拖动组件到顶部标签移动分组，保留当前组，保存失败时回到原位置', async t => {
  const f = await fixture({ active: 'work' });
  try {
    mockGroupHitTest(t, groupStrip());
    const card = () => document.querySelector('[data-panel-instance="a"]');
    const handle = card().querySelector('.drag-handle');
    handle.setPointerCapture = () => {};
    const start = () => groupPointer(handle, 'pointerdown', 44, 145);
    const move = (x, y) => groupPointer(handle, 'pointermove', x, y);
    const drop = (x, y) => groupPointer(handle, 'pointerup', x, y);
    const target = () => document.querySelector('[data-drop-target]');
    const changes = () => f.calls.filter(([name]) => name === 'manage_panel_group').map(([, args]) => args.change);
    const life = tab('生活');

    await start();
    await move(90, 230);
    assert.notEqual(card().style.top, '75px');
    await move(195, 15);
    assert.equal(target(), life);
    await move(78, 15);
    assert.equal(target(), null, '标签之间的空隙不构成目标');
    await drop(110, 15);
    assert.equal(card().style.top, '75px');
    assert.deepEqual(changes(), []);

    for (const cancel of ['pointercancel', 'lostpointercapture', 'Escape']) {
      await start();
      await move(195, 15);
      assert.equal(target(), life);
      if (cancel === 'Escape') {
        await act(async () => window.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: cancel })));
      } else {
        await groupPointer(handle, cancel, 195);
      }
      await drop(195, 15);
      assert.equal(target(), null);
      assert.deepEqual(changes(), []);
    }

    f.fail = true;
    await start();
    await move(90, 230);
    await move(195, 15);
    await drop(195, 15);
    assert.equal(f.state.panelGroups.membership.a, 'work');
    assert.equal(card().style.top, '75px');
    assert.match(document.querySelector('.panel-notice').textContent, /测试保存失败/);

    f.fail = false;
    await start();
    await move(195, 15);
    await drop(195, 15);
    assert.equal(target(), null);
    assert.equal(f.state.panelGroups.membership.a, 'life');
    assert.equal(tab('工作').getAttribute('aria-selected'), 'true');
    assert.equal(tab('生活').querySelector('small').textContent, '1');
    assert.equal(card(), null);
    assert.match(document.querySelector('.panel-notice').textContent, /已移到“生活”/);

    await click(life);
    const newHandle = card().querySelector('.drag-handle');
    newHandle.setPointerCapture = () => {};
    await groupPointer(newHandle, 'pointerdown', 44, 145);
    await groupPointer(newHandle, 'pointermove', 50);
    assert.equal(target(), tab('未分组'));
    await groupPointer(newHandle, 'pointerup', 50);
    assert.equal(f.state.panelGroups.membership.a, undefined);
    assert.equal(tab('生活').getAttribute('aria-selected'), 'true');
    assert.equal(f.calls.some(([name]) => name === 'save_panel_layout'), false);
    assert.deepEqual(changes(), [
      { action: 'move', instance: 'a', group: 'life' },
      { action: 'move', instance: 'a', group: 'life' },
      { action: 'select', group: 'life' },
      { action: 'move', instance: 'a', group: null },
    ]);
  } finally { await f.close(); }
});

test('组件拖到分组栏边缘可滚动到隐藏的标签，缩放不触发移动分组', async t => {
  const request = window.requestAnimationFrame, cancel = window.cancelAnimationFrame;
  const frames = new Map();
  let nextFrame = 0;
  window.requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame; };
  window.cancelAnimationFrame = id => { frames.delete(id); };
  t.after(() => Object.assign(window, { requestAnimationFrame: request, cancelAnimationFrame: cancel }));
  const f = await fixture({ active: 'work' });
  try {
    const strip = groupStrip(160);
    mockGroupHitTest(t, strip);
    const resize = document.querySelector('.resize-handle');
    resize.setPointerCapture = () => {};
    await groupPointer(resize, 'pointerdown', 125, 280);
    await groupPointer(resize, 'pointermove', 65, 15);
    assert.equal(document.querySelector('[data-drop-target]'), null);
    await groupPointer(resize, 'pointerup', 65, 15);
    assert.equal(f.calls.some(([name]) => name === 'save_panel_layout'), true);
    assert.equal(f.calls.some(([name]) => name === 'manage_panel_group'), false);

    const handle = document.querySelector('.drag-handle');
    handle.setPointerCapture = () => {};
    await groupPointer(handle, 'pointerdown', 44, 145);
    await groupPointer(handle, 'pointermove', 154);
    for (let i = 0; i < 20 && frames.size; i++) {
      const [id, draw] = frames.entries().next().value;
      frames.delete(id);
      await act(async () => draw());
    }
    assert.equal(strip.scrollLeft, 80);
    assert.equal(document.querySelector('[data-drop-target]'), tab('生活'));
    await groupPointer(handle, 'pointerup', 154);
    assert.equal(f.state.panelGroups.membership.a, 'life');
    assert.equal(frames.size, 0);
  } finally { await f.close(); }
});

test('分组标签浮起跟随指针，邻居滑开让位，保存失败退回原顺序，成功后保留当前组与组件', async () => {
  const f = await fixture({ active: 'work' });
  try {
    groupStrip(280, [76, 102, 58]);
    const original = ['未分组', '工作', '生活'];
    const life = tab('生活');
    const work = tab('工作');
    const drag = async () => { await groupPointer(life, 'pointerdown', 195); await groupPointer(life, 'pointermove', 95, 23); };
    await drag();
    assert.equal(floatingTab().style.transform, 'translate3d(86px, 8px, 0)');
    assert.equal(floatingTab().style.width, '58px');
    assert.equal(floatingTab().getAttribute('aria-hidden'), 'true');
    assert.equal(work.style.transform, 'translateX(62px)');
    assert.equal(tab('未分组').style.transform, '');
    await groupPointer(life, 'pointermove', 130);
    assert.equal(work.style.transform, '', '返回时以相邻标签原位置判断，避免让位后落点来回跳动');
    await groupPointer(life, 'pointermove', 95);
    assert.deepEqual(tabOrder(), original);
    assert.equal(f.calls.some(([name]) => name === 'manage_panel_group'), false);
    let release;
    f.gate = new Promise(resolve => { release = resolve; });
    f.fail = true;
    await groupPointer(life, 'pointerup', 95);
    assert.equal(life.disabled, true);
    assert.deepEqual(tabOrder(), original);
    assert.equal(floatingTab().style.transform, 'translate3d(80px, 0px, 0)');
    assert.equal(floatingTab().dataset.settling, 'true');
    assert.equal(work.style.transform, 'translateX(62px)');
    await act(async () => release());
    assert.match(document.querySelector('.panel-notice').textContent, /测试保存失败/);
    assert.deepEqual(tabOrder(), original);
    assert.equal(life.dataset.dragging, undefined);
    assert.equal(floatingTab(), null);
    assert.equal(work.style.transform, '');

    f.fail = false;
    await drag();
    await groupPointer(life, 'pointerup', 95);
    await pointerClick(life);
    assert.deepEqual(tabOrder(), ['未分组', '生活', '工作']);
    assert.equal(floatingTab(), null);
    assert.equal(work.style.transform, '');
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
      assert.equal(floatingTab(), null);
      assert.equal(work.style.transform, '');
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
    assert.equal(tab('生活').style.transform, 'translateX(-80px)');
    assert.equal(floatingTab().style.transform, 'translate3d(122px, 0px, 0)');
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

test('官方广场、组件库和已添加实例都能组合分类与搜索', async () => {
  const f = await fixture({ query: '?management=1' });
  const clock = { ...f.state.packages[0], id: 'local.souldesk.widgetjs.clock', name: '时钟' };
  const imported = { ...clock, id: 'local.fixture.note', name: '本地便签' };
  f.state.packages = [clock, imported];
  f.state.instances = f.state.instances.map(instance => ({
    ...instance, packageId: instance.id === 'desktop' ? imported.id : clock.id,
  }));
  const entries = [
    { id: clock.id, name: clock.name, category: '桌面组件', summary: '显示当地时间', versions: [] },
    { id: imported.id, name: imported.name, category: '效率', summary: '记录临时内容', versions: [] },
  ];
  const base = f.invoke;
  f.invoke = async (command, args) => command === 'official_catalog'
    ? { online: !args?.cachedOnly, catalog: { revision: 6, widgets: entries } }
    : base(command, args);
  const select = label => click(textButton(button('组件分类'), label));
  const names = () => [...document.querySelectorAll('.library-grid h2')].map(node => node.textContent);
  const navigate = label => click([...document.querySelectorAll('nav button')].find(node => node.textContent.endsWith(label)));
  try {
    await act(async () => { for (const notify of f.listeners.get('host:changed')) await notify(); });
    assert.deepEqual(f.calls.filter(([command]) => command === 'official_catalog'),
      [['official_catalog', { cachedOnly: true }]]);
    await select('时间');
    assert.deepEqual(names(), ['时钟']);
    await input(button('搜索组件'), '便签');
    assert.deepEqual(names(), []);
    await select('其他');
    assert.deepEqual(names(), ['本地便签']);

    await navigate('官方广场');
    await select('时间');
    await input(button('搜索组件'), '当地');
    assert.deepEqual(names(), ['时钟']);
    await select('效率');
    assert.deepEqual(names(), []);
    await click(textButton(document, '清除筛选'));
    assert.deepEqual(names(), ['时钟', '本地便签']);

    await navigate('组件库');
    assert.equal(textButton(button('组件分类'), '其他'), undefined);
    await select('效率');
    await input(button('搜索组件'), '临时内容');
    assert.deepEqual(names(), ['本地便签']);
    const instanceNames = () => [...document.querySelectorAll('.instance-row h2')].map(node => node.textContent);
    await click(textButton(document, '管理实例'));
    assert.deepEqual(instanceNames(), ['本地便签']);
    await click(textButton(document, '显示全部'));
    await select('时间');
    assert.deepEqual(instanceNames(), ['时钟', '时钟', '时钟']);
    assert.match(document.querySelector('.widget-result-count').textContent, /3 个实例/);
    await input(button('搜索组件'), clock.id);
    assert.equal(instanceNames().length, 3);
    await input(button('搜索组件'), '临时内容');
    assert.deepEqual(instanceNames(), []);
    await select('效率');
    assert.deepEqual(instanceNames(), ['本地便签']);
    assert.ok(textButton(document.querySelector('.instance-row'), '移入面板'));
  } finally { await f.close(); }
});

test('组件库全部更新先确认兼容版本和新增权限，逐项更新并报告部分失败', async () => {
  const f = await fixture({ query: '?management=1' });
  const installed = ['便签', '时钟', '已是新版', '需升级宿主', '私有组件'].map((name, i) => ({
    ...f.state.packages[0], name, id: `local.fixture.widget-${i}`, digest: String(i).repeat(64),
    version: i === 2 ? '2.0.0' : '1.0.0', capabilities: [],
  }));
  f.state.packages = installed;
  const published = { version: '2.0.0', minHostVersion: '0.5.0', capabilities: ['storage.instance'],
    releaseNotes: '更新', status: 'published', sha256: 'b'.repeat(64) };
  const widgets = [...installed.slice(0, 4), { id: 'local.fixture.uninstalled', name: '未安装' }]
    .map(({ id, name }, i) => ({ id, name, category: i === 1 ? '时间' : '效率', summary: '说明',
      versions: i === 3 ? [{ ...published, minHostVersion: '99.0.0' }]
        : [{ ...published, version: '3.0.0', status: 'withdrawn' }, published] }));
  let online = false;
  let rejectFirst;
  const base = f.invoke;
  f.invoke = async (command, args) => {
    if (command === 'official_catalog') {
      return { online: online && !args?.cachedOnly, catalog: { revision: 6, widgets }, error: online ? null : '网络失败' };
    }
    if (command === 'install_official') {
      f.calls.push([command, args]);
      if (args.packageId === installed[0].id) return new Promise((_, reject) => { rejectFirst = reject; });
      installed[1].version = published.version;
      return;
    }
    return base(command, args);
  };
  const open = () => click(textButton(document, '全部更新'));
  const dialog = () => button('全部更新组件');
  const calls = () => f.calls.filter(([command]) => command === 'install_official').map(([, args]) => args);
  try {
    await act(async () => { for (const notify of f.listeners.get('host:changed')) await notify(); });
    await open();
    assert.match(dialog().textContent, /无法在线检查组件更新/);
    assert.equal(textButton(dialog(), '确认全部更新'), undefined);
    await click(textButton(dialog(), '关闭'));

    // 即使组件库正在筛选，批量范围仍覆盖全部符合条件的已安装组件。
    await input(button('搜索组件'), '便签');
    online = true;
    await open();
    assert.deepEqual([...dialog().querySelectorAll('li h3')].map(node => node.textContent), ['便签', '时钟']);
    assert.match(dialog().textContent, /v1\.0\.0 → v2\.0\.0 · 新增权限 · 读取和保存/);
    assert.deepEqual(calls(), []);
    await submit(dialog().querySelector('form'));
    assert.equal(calls().length, 1);
    assert.match(dialog().textContent, /正在下载和试加载/);
    await act(async () => rejectFirst(Error('试加载失败，原版已保留')));
    assert.deepEqual(calls(), installed.slice(0, 2).map(pkg => ({
      packageId: pkg.id, expectedSha256: published.sha256, expectedLocalDigest: pkg.digest,
    })));
    assert.match(dialog().textContent, /已更新 1 个，失败 1 个/);
    assert.match(dialog().textContent, /试加载失败，原版已保留/);
    assert.equal(installed[0].version, '1.0.0');
    assert.equal(installed[1].version, '2.0.0');
    await click(textButton(dialog(), '关闭'));
    await open();
    assert.deepEqual([...dialog().querySelectorAll('li h3')].map(node => node.textContent), ['便签']);
  } finally { await f.close(); }
});

test('官方广场选兼容版本，更新确认显示新增权限，试加载失败保留原登记', async () => {
  const f = await fixture({ query: '?management=1' });
  const id = 'official.souldesk.test';
  const digest = 'c'.repeat(64);
  const local = { ...f.state.packages[0], id, version: '0.1.0', installed: true, digest: 'a'.repeat(64), capabilities: [] };
  f.state.packages.push(local);
  const compatible = { version: '0.1.1', minHostVersion: '0.5.0', capabilities: ['storage.instance'],
    releaseNotes: '启用便签保存', status: 'published', sha256: digest };
  const official = { id, name: '测试组件', category: '效率', summary: '测试说明', versions: [
    { ...compatible, version: '0.5.0', minHostVersion: '99.0.0' },
    { ...compatible, version: '0.4.0', status: 'withdrawn' }, compatible,
  ] };
  const base = f.invoke.bind(f);
  f.invoke = async (command, args) => {
    if (command === 'official_catalog') return { online: true, catalog: { revision: 4, widgets: [official] } };
    if (command === 'install_official') {
      f.calls.push([command, args]);
      throw Error('试加载超时，原版已保留');
    }
    return base(command, args);
  };
  try {
    await act(async () => { for (const notify of f.listeners.get('host:changed')) await notify(); });
    await click([...document.querySelectorAll('nav button')].find(node => node.textContent.endsWith('官方广场')));
    const card = document.querySelector('.gallery-package');
    assert.match(card.textContent, /v0\.1\.1/);
    assert.doesNotMatch(card.textContent, /v0\.5\.0|v0\.4\.0/);
    await click(textButton(card, '更新'));
    const dialog = document.querySelector('dialog[open]');
    assert.match(dialog.textContent, /新增权限 · 读取和保存当前实例的设置与存储/);
    await submit(dialog.querySelector('form'));
    assert.deepEqual(f.calls.find(([name]) => name === 'install_official')[1],
      { packageId: id, expectedSha256: digest, expectedLocalDigest: local.digest });
    assert.match(dialog.querySelector('[role=alert]').textContent, /试加载超时/);
    assert.equal(local.version, '0.1.0');
  } finally { await f.close(); }
});

test('目录故障保留可信列表和已装组件状态，已撤回的包可离线卸载', async () => {
  const f = await fixture({ query: '?management=1' });
  const id = 'official.souldesk.test';
  const local = { ...f.state.packages[0], id, version: '0.1.0', installed: true, digest: 'a'.repeat(64) };
  f.state.packages.push(local);
  const version = { version: '0.1.0', minHostVersion: '0.5.0', capabilities: [],
    releaseNotes: '首次发布', status: 'published', sha256: 'b'.repeat(64) };
  const entry = { id, name: '测试组件', category: '效率', summary: '测试说明', versions: [version] };
  const base = f.invoke.bind(f);
  f.invoke = async (command, args) => {
    if (command === 'unregister_package') {
      f.state.packages = f.state.packages.filter(item => item.id !== args.packageId);
    }
    if (command === 'official_catalog') {
      return { online: false, error: '官方目录验签失败', catalog: { revision: 4, widgets: [entry] } };
    }
    return base(command, args);
  };
  try {
    await act(async () => { for (const notify of f.listeners.get('host:changed')) await notify(); });
    await click([...document.querySelectorAll('nav button')].find(node => node.textContent.endsWith('官方广场')));
    assert.match(document.querySelector('[role=status]').textContent, /上次验签成功/);
    assert.match(document.querySelector('.gallery-package').textContent, /同版本.*冲突/);
    version.status = 'withdrawn';
    await click(textButton(document, '刷新目录'));
    assert.match(document.querySelector('.gallery-package').textContent, /暂无可安装版本/);
    await click([...document.querySelectorAll('nav button')].find(node => node.textContent.endsWith('组件库')));
    const localCard = [...document.querySelectorAll('.library-grid .package')]
      .find(card => card.querySelector('[data-kind]')?.dataset.kind === id);
    await click(textButton(localCard, '卸载'));
    assert.ok(f.calls.some(([name, args]) => name === 'unregister_package' && args.packageId === id));
    assert.equal(f.state.packages.some(item => item.id === id), false);
  } finally { await f.close(); }
});


test('官方广场首次安装须确认，完成后由用户决定添加到面板', async () => {
  const f = await fixture({ query: '?management=1' });
  const id = 'official.souldesk.test';
  const release = { version: '0.1.0', minHostVersion: '0.5.2', capabilities: [],
    releaseNotes: '首次发布', status: 'published', sha256: 'a'.repeat(64) };
  const widget = { id, name: '测试组件', category: '效率', summary: '测试说明', versions: [release] };
  const base = f.invoke.bind(f);
  f.invoke = async (command, args) => {
    if (command === 'official_catalog') return { online: true, catalog: { revision: 4, widgets: [widget] } };
    if (command === 'install_official') {
      f.state.packages.push({ ...f.state.packages[0], id, version: release.version, installed: true, digest: release.sha256 });
    }
    if (command === 'create_instance') {
      f.state.instances.push({ ...f.state.instances[0], id: 'gallery-instance', packageId: id, location: 'panel' });
    }
    return base(command, args);
  };
  try {
    await click([...document.querySelectorAll('nav button')].find(node => node.textContent.endsWith('官方广场')));
    const card = () => document.querySelector('.gallery-package');
    await click(textButton(card(), '安装'));
    assert.equal(f.calls.some(([name]) => name === 'install_official'), false);
    const dialog = document.querySelector('dialog[open]');
    assert.match(dialog.textContent, /测试组件.*0\.1\.0.*0\.5\.2.*无需额外权限/);
    await submit(dialog.querySelector('form'));
    assert.deepEqual(f.calls.find(([name]) => name === 'install_official')[1],
      { packageId: id, expectedSha256: release.sha256 });
    assert.match(card().textContent, /0 个实例/);
    assert.equal(f.calls.some(([name]) => name === 'create_instance'), false);
    await click(textButton(card(), '添加到面板'));
    assert.deepEqual(f.calls.find(([name]) => name === 'create_instance')[1], { packageId: id });
    assert.ok(textButton(card(), '管理实例'));
    assert.equal(textButton(card(), '卸载'), undefined);
  } finally { await f.close(); }
});
