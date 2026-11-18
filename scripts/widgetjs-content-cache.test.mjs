// 假宿主检查请求标志、原始时间和失败保留，不访问外部接口。
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { resolve } from 'node:path';
import { readAlmanac } from '../components/widgetjs/calendar-source/content-client.js';

test('两款日历共用缓存读取，旧宿主拒绝元数据时兼容原接口', async () => {
  const calls = [], response = { data: {}, fetchedAt: 123 };
  const host = { async request(_, args) { calls.push(args); if (args.cache) throw Error('INVALID_ARGUMENT: 黄历来源不接受额外参数'); return response; } };
  assert.equal(await readAlmanac(host, true), response);
  assert.deepEqual(calls, [{ source: 'itime-calendar', cache: true, forceRefresh: true }, { source: 'itime-calendar' }]);
  calls.length = 0;
  assert.equal(await readAlmanac({ async request(_, args) { calls.push(args); return response; } }), response);
  assert.deepEqual(calls, [{ source: 'itime-calendar', cache: true }]);
});

test('日历真实网络失败不自动降级重试或伪装为缓存成功', async () => {
  let calls = 0;
  await assert.rejects(readAlmanac({ async request() { calls++; throw Error('DATA_UNAVAILABLE: HTTP 503'); } }, true), /503/);
  assert.equal(calls, 1);
});

test('热点自动读取显式启用缓存，手动绕过，失败保留上次真实获取时间', async () => {
  const oldLocation = globalThis.location;
  globalThis.location = { href: 'http://widget.fixture/', search: '' };
  const vite = await createServer({ configFile: resolve('components/widgetjs/hotspot-source/vite.config.mjs'), server: { middlewareMode: true, hmr: false, ws: false, watch: null }, appType: 'custom', logLevel: 'silent', optimizeDeps: { noDiscovery: true, include: [] } });
  try {
    const native = await vite.ssrLoadModule('/native.ts'), { default: client } = await vite.ssrLoadModule('/client.ts');
    const calls = []; let failed = false;
    native.setHost({ preview: false, async request(method, args) {
      assert.equal(method, 'content.get'); calls.push(args);
      if (failed) throw Error('DATA_UNAVAILABLE: HTTP 503');
      return { data: { list: [{ keyword: '保留榜单', show_name: '保留榜单' }] }, source: 'https://s.search.bilibili.com/main/hotword', fetchedAt: 1234 };
    } });
    await client.get('https://s.search.bilibili.com/main/hotword');
    assert.deepEqual(calls[0], { source: 'bilibili', cache: true });
    assert.equal(native.contentStatus.updatedAt, 1234);
    await client.get('https://s.search.bilibili.com/main/hotword', { forceRefresh: true });
    assert.deepEqual(calls[1], { source: 'bilibili', cache: true, forceRefresh: true });
    assert.equal(native.contentStatus.updatedAt, 1234);
    failed = true;
    await assert.rejects(client.get('https://s.search.bilibili.com/main/hotword', { forceRefresh: true }), /503/);
    assert.equal(native.contentStatus.updatedAt, 1234); assert.match(native.contentStatus.error, /503/);
    assert.equal(native.contentStatus.loading, 0);
  } finally { await vite.close(); if (oldLocation === undefined) delete globalThis.location; else globalThis.location = oldLocation; }
});

test('热点预览不请求缓存，推荐保留上次展示参数', async () => {
  const oldLocation = globalThis.location; globalThis.location = { href: 'http://widget.fixture/', search: '' };
  const vite = await createServer({ configFile: resolve('components/widgetjs/hotspot-source/vite.config.mjs'), server: { middlewareMode: true, hmr: false, ws: false, watch: null }, appType: 'custom', logLevel: 'silent', optimizeDeps: { noDiscovery: true, include: [] } });
  try {
    const native = await vite.ssrLoadModule('/native.ts'), { default: client } = await vite.ssrLoadModule('/client.ts'); let called = 0;
    native.setHost({ preview: true, request() { assert.fail('预览不能请求'); } });
    await client.get('https://s.search.bilibili.com/main/hotword', { forceRefresh: true });
    native.setHost({ preview: false, async request(_, args) { called++; assert.deepEqual(args, { source: 'bilibili-rcmd', lastShown: 'av_1,av_2', cache: true }); return { data: { code: 0, data: { item: [] } }, fetchedAt: 10 }; } });
    await client.get('https://api.bilibili.com/x/web-interface/wbi/index/top/feed/rcmd', { params: { last_show_list: 'av_1,av_2' } });
    assert.equal(called, 1);
  } finally { await vite.close(); if (oldLocation === undefined) delete globalThis.location; else globalThis.location = oldLocation; }
});
