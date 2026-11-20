import { readFileSync } from 'node:fs';
import test from 'node:test';
import assert from 'node:assert/strict';
import { framePermissions } from '../src/widget-permissions.ts';
const root = new URL('../src-tauri/', import.meta.url);
const capabilities = ['host', 'trial'].map(name => {
  try { return JSON.parse(readFileSync(new URL(`capabilities/${name}.json`, root), 'utf8')); }
  catch { return null; }
});
test('隐藏试加载容器拥有桥接与失败报告权限，不拥有系统或实例管理权限', () => {
  const trial = capabilities[1];
  assert.ok(trial?.windows.includes('trial-*'));
  assert.deepEqual([...trial.permissions].sort(), ['allow-open-session', 'allow-close-session', 'allow-widget-request', 'allow-trial-failed'].sort());
});
test('G1/G2宿主命令已登记且授予宿主窗口', () => {
  const build = readFileSync(new URL('build.rs', root), 'utf8');
  for (const command of ['manage_instance', 'reorder_instances', 'unregister_package', 'set_autostart', 'trial_failed', 'business_settings']) {
    assert.ok(build.includes(`"${command}"`), `未登记 ${command}`);
    if (command !== 'trial_failed') assert.ok(capabilities[0].permissions.includes(`allow-${command.replaceAll('_', '-')}`));
  }
});
test('官方目录与线上安装命令仅授予管理窗口', () => {
  const build = readFileSync(new URL('build.rs', root), 'utf8');
  for (const name of ['official_catalog', 'install_official']) {
    assert.ok(build.includes('"' + name + '"'));
    for (const type of ['host', 'panel', 'surface', 'detail', 'trial']) {
      const capability = JSON.parse(readFileSync(new URL('capabilities/' + type + '.json', root), 'utf8'));
      assert.equal(capability.permissions.includes('allow-' + name.replaceAll('_', '-')), type === 'host');
    }
  }
});
test('面板分组管理仅授予管理窗口与面板，不授予组件展示窗口', () => {
  const build = readFileSync(new URL('build.rs', root), 'utf8');
  assert.ok(build.includes('"manage_panel_group"'));
  for (const name of ['host', 'panel']) {
    const cap = JSON.parse(readFileSync(new URL(`capabilities/${name}.json`, root), 'utf8'));
    assert.ok(cap.permissions.includes('allow-manage-panel-group'));
  }
  for (const name of ['surface', 'detail', 'trial']) {
    const cap = JSON.parse(readFileSync(new URL(`capabilities/${name}.json`, root), 'utf8'));
    assert.ok(!cap.permissions.includes('allow-manage-panel-group'));
  }
});
test('面板就绪和滚动更新仅授予面板壳', () => {
  const build = readFileSync(new URL('build.rs', root), 'utf8');
  assert.ok(build.includes('"panel_view"'));
  for (const name of ['panel', 'host', 'edge', 'surface', 'detail', 'trial']) {
    const capability = JSON.parse(readFileSync(new URL(`capabilities/${name}.json`, root), 'utf8'));
    assert.equal(capability.permissions.includes('allow-panel-view'), name === 'panel');
  }
});
test('挂件原生拖动仅授予展开和收起的面板窗口', () => {
  const build = readFileSync(new URL('build.rs', root), 'utf8');
  assert.ok(build.includes('"drag_panel_edge"'));
  for (const name of ['panel', 'host', 'edge', 'surface', 'detail', 'trial']) {
    const capability = JSON.parse(readFileSync(new URL(`capabilities/${name}.json`, root), 'utf8'));
    assert.equal(capability.permissions.includes('allow-drag-panel-edge'), ['panel', 'edge'].includes(name));
  }
});
test('组件本地表单可触发提交事件，同时保持CSP禁止表单导航', () => {
  const frame = readFileSync(new URL('../src/WidgetFrame.tsx', import.meta.url), 'utf8');
  assert.ok(framePermissions().sandbox.split(' ').includes('allow-forms'));
  assert.match(frame, /sandbox=\{permissions\.sandbox\}/);
  assert.match(readFileSync(new URL('src/widgets.rs', root), 'utf8'), /form-action 'none'/);
});

test('组件允许同来源音频并继续禁止直接联网', () => {
  const source = readFileSync(new URL('src/widgets.rs', root), 'utf8');
  assert.match(source, /media-src 'self'/);
  assert.match(source, /connect-src 'none'/);
});

test('键盘叠加层只有桥接与故障报告，不授予激活、拖动或实例管理',()=>{
  const overlay=JSON.parse(readFileSync(new URL('capabilities/keyboard-overlay.json',root),'utf8'));
  assert.deepEqual(overlay.windows,['keyboard-*']);
  assert.deepEqual([...overlay.permissions].sort(),['core:event:default','allow-open-session','allow-close-session','allow-widget-request','allow-surface-failed'].sort());
});

test('灵动通知和显式休息页只开放固定桥接，不开放窗口或系统输入命令',()=>{
  const overlay = JSON.parse(readFileSync(new URL('capabilities/notification-overlay.json', root), 'utf8'));
  const rest = JSON.parse(readFileSync(new URL('capabilities/reminder-surfaces.json', root), 'utf8'));
  const build = readFileSync(new URL('build.rs', root), 'utf8');
  assert.deepEqual(overlay.windows, ['island-host-*']);
  assert.deepEqual(overlay.permissions, ['allow-island-surface','core:event:allow-listen','core:event:allow-unlisten']);
  assert.deepEqual(rest.windows, ['break-*']);
  assert.deepEqual([...rest.permissions].sort(), ['core:event:default','allow-open-session','allow-close-session','allow-widget-request','allow-surface-failed'].sort());
  assert.ok(build.includes('"island_surface"') && build.includes('"notification_settings"'));
  assert.ok(capabilities[0].permissions.includes('allow-notification-settings'));
  assert.ok(!capabilities[1].permissions.includes('allow-island-surface'));
});

test('剪切板弹窗仅授予桥接和失败报告，原生窗口及输入由固定业务后台执行',()=>{
  const capability=JSON.parse(readFileSync(new URL('capabilities/clipboard-surfaces.json',root),'utf8'));
  assert.deepEqual(capability.windows,['clipboard-history-*','clipboard-search-*','clipboard-tool-*']);
  assert.deepEqual([...capability.permissions].sort(),['core:event:default','allow-open-session','allow-close-session','allow-widget-request','allow-surface-failed'].sort());
});

test('多视图助手仅向本机壳授予专用命令，远程页与登录窗不匹配权限',()=>{
  const cap=JSON.parse(readFileSync(new URL('capabilities/assistant-shell.json',root),'utf8'));
  assert.equal(cap.windows,undefined);
  assert.deepEqual(cap.webviews,['assistant-shell-*']);
  assert.deepEqual([...cap.permissions].sort(),['core:event:default','allow-assistant-open-session','allow-assistant-close-session','allow-assistant-widget-request','allow-assistant-surface-failed'].sort());
  for(const command of ['assistant_open_session','assistant_close_session','assistant_widget_request','assistant_surface_failed'])assert.ok(readFileSync(new URL('build.rs',root),'utf8').includes(`"${command}"`));
});
test('AI直聘仅向三种本机壳授予固定桥接，BOSS和真实详情页没有IPC权限',()=>{
  const cap=JSON.parse(readFileSync(new URL('capabilities/job-shell.json',root),'utf8'));
  assert.equal(cap.windows,undefined);
  assert.deepEqual(cap.webviews,['job-shell-*','job-analysis-shell-*','job-knowledge-shell-*']);
  assert.deepEqual([...cap.permissions].sort(),['core:event:default','allow-job-open-session','allow-job-close-session','allow-job-widget-request','allow-job-surface-failed'].sort());
  const build=readFileSync(new URL('build.rs',root),'utf8');
  for(const command of ['job_open_session','job_close_session','job_widget_request','job_surface_failed'])assert.ok(build.includes(`"${command}"`));
  const frame=readFileSync(new URL('../src/WidgetFrame.tsx',import.meta.url),'utf8');
  assert.match(frame,/jobShell \? `job_\$\{name\}`/);
  assert.match(readFileSync(new URL('../src/main.tsx',import.meta.url),'utf8'),/jobPanel \? <WidgetFrame instance=\{jobPanel\} page="job-panel" jobShell/);
  assert.doesNotMatch(readFileSync(new URL('src/job_webviews.rs',root),'utf8'),/initialization_script/);
  const permissions=framePermissions(['jobs.manage']);
  assert.equal(permissions.allowFullScreen,true);
  assert.equal(permissions.allow,'fullscreen');
  assert.ok(!(permissions.allow??'').includes('clipboard'));
});

test('番茄专用窗只授予固定桥接及失败报告，不开放通用窗口或输入',()=>{const c=JSON.parse(readFileSync(new URL('capabilities/pomodoro-window.json',root),'utf8'));assert.deepEqual(c.windows,['pomodoro-window-*']);assert.deepEqual([...c.permissions].sort(),['core:event:default','allow-open-session','allow-close-session','allow-widget-request','allow-surface-failed'].sort());});
