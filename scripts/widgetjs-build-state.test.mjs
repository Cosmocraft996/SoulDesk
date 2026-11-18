import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { assertPackageVersions, cacheMatches, inputGroups, packageState, withBuildLock } from './widgetjs-build-state.mjs';

function files(version = '0.1.0', text = '甲') {
  return { 'manifest.json': Buffer.from(JSON.stringify({ id: 'local.test.card', version, entry: 'dist/index.html' })), 'dist/index.html': Buffer.from(text) };
}

test('包内容指纹不依赖文件顺序或ZIP压缩选项，但能识别真实资源变化', () => {
  const original = files(), reordered = Object.fromEntries(Object.entries(original).reverse());
  assert.deepEqual(packageState(original), packageState(reordered));
  assert.notEqual(packageState(original).digest, packageState(files('0.1.0', '乙')).digest);
  assert.throws(() => packageState({ 'manifest.json': original['manifest.json'] }), /入口缺失/);
});

test('同版本内容变化和降级被拒绝，提升版本及新增独立组件可通过', () => {
  const previous = [packageState(files())];
  assert.doesNotThrow(() => assertPackageVersions(previous, previous));
  assert.throws(() => assertPackageVersions(previous, [packageState(files('0.1.0', '乙'))]), /版本未提升/);
  assert.throws(() => assertPackageVersions(previous, [packageState(files('0.0.9'))]), /降级/);
  assert.doesNotThrow(() => assertPackageVersions(previous, [packageState(files('0.1.1', '乙'))]));
  assert.throws(() => assertPackageVersions(previous, []), /消失/);
  assert.throws(() => assertPackageVersions([], [...previous, ...previous]), /重复/);
});

test('源码、共享SDK、依赖或构建脚本变化使缓存失效，宿主界面修改不触发组件重建', () => {
  const root = mkdtempSync(join(tmpdir(), 'souldesk-build-state-'));
  for (const dir of ['components/widgetjs/card', 'components/widgetjs/card-bundle', 'scripts', 'sdk', 'src']) mkdirSync(join(root, dir), { recursive: true });
  for (const [path, text] of Object.entries({ 'components/widgetjs/card/app.js': '甲', 'components/widgetjs/card-bundle/app.js': '产物', 'scripts/build-widgetjs.mjs': '构建', 'sdk/host.js': '桥接', 'package.json': '{}', 'src/App.tsx': '宿主' })) writeFileSync(join(root, path), text);
  const inputs = inputGroups(root), packages = [packageState(files())], state = { inputs, packages };
  assert.equal(cacheMatches(state, inputs, packages), true);
  writeFileSync(join(root, 'src/App.tsx'), '宿主调整');
  assert.deepEqual(inputGroups(root), inputs);
  writeFileSync(join(root, 'components/widgetjs/card/app.js'), '乙');
  assert.equal(cacheMatches(state, inputGroups(root), packages), false);
  writeFileSync(join(root, 'components/widgetjs/card/app.js'), '甲');
  writeFileSync(join(root, 'sdk/host.js'), '桥接调整');
  assert.equal(cacheMatches(state, inputGroups(root), packages), false);
  assert.equal(cacheMatches(state, inputs, [packageState(files('0.1.1'))]), false);
});

test('并发构建不会同时改写资源，失败后只释放自己的锁以便修正后重试', () => {
  const root = mkdtempSync(join(tmpdir(), 'souldesk-build-lock-'));
  withBuildLock(() => assert.throws(() => withBuildLock(() => assert.fail('不应进入第二个构建'), root), /已有组件构建/), root);
  assert.throws(() => withBuildLock(() => { throw new Error('受控构建失败'); }, root), /受控构建失败/);
  assert.equal(withBuildLock(() => 42, root), 42);
});
