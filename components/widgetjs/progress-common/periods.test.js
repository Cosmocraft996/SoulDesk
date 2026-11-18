import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { periodBounds, elapsedPercent, remainingPercent, parseDate, validateSettings } from './periods.js';
import { progressWidgets, clockWidgets, widgetFiles, widgetArchive } from '../../../scripts/build-widgetjs.mjs';
import { unzipSync } from 'fflate';

test('年/月/周/日使用本地日历边界，跨年周仍从周一开始', () => {
  const now = new Date(2024, 0, 1, 12);
  const day = periodBounds(now, 0);
  assert.equal(day.start.getHours(), 0);
  assert.equal(day.end.getDate(), 2);
  assert.equal(remainingPercent(now, 0), 50);
  const week = periodBounds(new Date(2023, 11, 31, 12), 1);
  assert.equal(week.start.getDate(), 25);
  assert.equal(week.start.getDay(), 1);
  assert.equal(week.end.getFullYear(), 2024);
  const february = periodBounds(new Date(2024, 1, 29), 2);
  assert.equal(february.end.getMonth(), 2);
  assert.equal((february.end - february.start) / 86400000, 29);
  assert.equal((periodBounds(now, 3).end - periodBounds(now, 3).start) / 86400000, 366);
});
test('自定义事项在开始前与完成后固定在0至100，不出现负剩余', () => {
  const start = '2026-01-01T00:00:00Z', end = '2026-01-02T00:00:00Z';
  assert.equal(elapsedPercent(new Date('2025-01-01Z'), 4, start, end), 0);
  assert.equal(remainingPercent(new Date('2026-01-01T12:00:00Z'), 4, start, end), 50);
  assert.equal(remainingPercent(new Date('2027-01-01Z'), 4, start, end), 0);
  assert.throws(() => periodBounds(new Date(), 4, start, start), /结束时间/);
  assert.throws(() => periodBounds(new Date(), 4, end, start), /结束时间/);
});
test('非法日期、无效时段和颜色必须明确拒绝', () => {
  for (const value of ['', 'tomorrow', '2026-02-30T12:00', '2026-13-01T12:00', '2026-01-01T25:00', '2026-01-01T12:60']) assert.throws(() => parseDate(value));
  assert.throws(() => periodBounds(new Date('invalid'), 0));
  for (const type of [-1, 5, 1.5, '0']) assert.throws(() => periodBounds(new Date(), type));
  assert.throws(() => validateSettings('time', { radius: 10, title: '时间', background: 'url(https://example.com)', foreground: '#123456' }), /颜色/);
  assert.throws(() => validateSettings('wave', { radius: 10, period: 4, eventName: '事件', startDate: '', endDate: '', accent: '' }));
});
test('夏令时的23小时日仍在真实时间中点显示50%', () => {
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
    import { periodBounds, remainingPercent } from './components/widgetjs/progress-common/periods.js';
    const now = new Date(2026, 2, 8, 12, 30);
    const { start, end } = periodBounds(now, 0);
    process.stdout.write(JSON.stringify({ hours: (end - start) / 3600000, remaining: remainingPercent(now, 0) }));
  `], { env: { ...process.env, TZ: 'America/New_York' }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { hours: 23, remaining: 50 });
});
test('独立标准包带全部相对资源、宿主SDK和许可来源，打包可复现', () => {
  for (const name of progressWidgets) {
    const files = widgetFiles(name), text = path => new TextDecoder().decode(files[path]);
    const manifest = JSON.parse(text('manifest.json'));
    assert.equal(manifest.hostApiMajor, 1);
    assert.equal(manifest.loadCheck, 1);
    assert.ok(files[manifest.entry]);
    assert.deepEqual(manifest.capabilities, ['storage.instance']);
    assert.match(text('dist/LICENSE.txt'), /GNU GENERAL PUBLIC LICENSE/);
    assert.match(text('dist/SOURCE.txt'), /714802e66336133396ab55463eb5a26d08fc0788/);
    for (const [, path] of text(manifest.entry).matchAll(/(?:src|href)="\.\/([^"]+)"/g)) assert.ok(files['dist/' + path], path);
    for (const [, path] of text('dist/app.js').matchAll(/from '\.\/([^']+)'/g)) assert.ok(files['dist/' + path], path);
    const archive = widgetArchive(name);
    assert.deepEqual(archive, widgetArchive(name));
    assert.deepEqual(Object.keys(unzipSync(archive)).sort(), Object.keys(files).sort());
  }
});
test('五种时钟各有标准包和原始组件标识，运行包不依赖WidgetJS服务器或模块', () => {
  const identities = new Set();
  for (const name of clockWidgets) {
    const files = widgetFiles(name), text = path => new TextDecoder().decode(files[path]);
    const manifest = JSON.parse(text('manifest.json'));
    assert.ok(!identities.has(manifest.id));
    identities.add(manifest.id);
    assert.match(text('dist/index.html'), new RegExp('data-variant="' + name + '"'));
    assert.match(text('dist/SOURCE.txt'), /646bf72e584b946da424ced3d6253e0c71b8e6e1/);
    assert.doesNotMatch(text('dist/app.js'), /from\s*["']@widget-js\//);
    assert.doesNotMatch(text('dist/style.css'), /url\(["']?https?:/);
    for (const [, path] of text(manifest.entry).matchAll(/(?:src|href)="\.\/([^"]+)"/g)) assert.ok(files['dist/' + path], path);
  }
  assert.equal(identities.size, 5);
});
