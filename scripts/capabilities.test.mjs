import { readFileSync } from 'node:fs';
import test from 'node:test';
import assert from 'node:assert/strict';
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
test('组件本地表单可触发提交事件，同时保持CSP禁止表单导航', () => {
  const frame = readFileSync(new URL('../src/WidgetFrame.tsx', import.meta.url), 'utf8');
  assert.match(frame, /sandbox="[^"]*allow-forms/);
  assert.match(readFileSync(new URL('src/lib.rs', root), 'utf8'), /form-action 'none'/);
});
