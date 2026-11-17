import test from 'node:test';
import assert from 'node:assert/strict';
const imported = await import('./endpoints.js').catch(() => null);

test('前端地址只能映射到固定的九个内容源，查询字符串不能换域或换接口', () => {
  assert.ok(imported, '热点接口映射尚未实现');
  const { providerForURL } = imported;
  assert.equal(providerForURL('https://s.search.bilibili.com/main/hotword').source, 'bilibili');
  assert.equal(providerForURL('https://api.bilibili.com/x/web-interface/wbi/index/top/feed/rcmd?brush=4', { last_show_list: 'av_1,av_2' }).lastShown, 'av_1,av_2');
  for (const address of ['https://attacker.example/x', 'https://s.search.bilibili.com/private', 'http://s.search.bilibili.com/main/hotword', 'https://s.search.bilibili.com@evil.example/main/hotword']) assert.throws(() => providerForURL(address));
});

test('预览返回明确的空数据，不读网络；真实响应校验失败时拒绝替代为空榜', () => {
  assert.ok(imported, '热点响应校验尚未实现');
  const { previewBody, validateBody } = imported;
  assert.deepEqual(previewBody('bilibili'), { list: [] });
  assert.deepEqual(previewBody('bangumi'), { code: 0, result: [] });
  assert.throws(() => validateBody('bilibili', { code: -403, message: '访问受限' }), /访问受限/);
  assert.throws(() => validateBody('weibo', { ok: 0 }), /格式/);
  assert.throws(() => validateBody('bilibili', { list: [{ keyword: 1 }] }), /格式/);
  const valid = { list: [{ keyword: '测试', show_name: '测试' }] };
  assert.equal(validateBody('bilibili', valid), valid);
  assert.throws(() => validateBody('world-cup', { response: 'bad json' }));
});
