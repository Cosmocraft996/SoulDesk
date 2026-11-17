const providers = new Map([
  ['https://s.search.bilibili.com/main/hotword', 'bilibili'],
  ['https://bangumi.bilibili.com/web_api/timeline_global', 'bangumi'],
  ['https://api.bilibili.com/x/web-interface/wbi/index/top/feed/rcmd', 'bilibili-rcmd'],
  ['https://www.cls.cn/api/cache', 'cls-telegraph'],
  ['https://aweme.snssdk.com/aweme/v1/hot/search/list/', 'douyin'],
  ['https://www.toutiao.com/hot-event/hot-board/', 'tou-tiao'],
  ['https://weibo.com/ajax/side/hotSearch', 'weibo'],
  ['https://facai365.vip/hub/api', 'world-cup'],
  ['https://www.zhihu.com/api/v3/feed/topstory/hot-lists/total', 'zhihu'],
]);
export function providerForURL(address, params = {}) {
  const url = new URL(address);
  const source = providers.get(url.origin + url.pathname);
  if (!source || url.username || url.password) throw new Error('未开放的热点接口地址');
  return { source, ...(source === 'bilibili-rcmd' ? { lastShown: String(params.last_show_list ?? '') } : {}) };
}
export function previewBody(source) {
  const bodies = {
    bilibili: { list: [] }, bangumi: { code: 0, result: [] },
    'bilibili-rcmd': { code: 0, data: { item: [] } },
    'cls-telegraph': { data: { l: {} } }, douyin: { data: { word_list: [] } },
    'tou-tiao': { data: [] }, weibo: { data: { realtime: [] } },
    zhihu: { data: [] }, 'world-cup': { name: 'World Cup 2026', matches: [] },
  };
  if (!bodies[source]) throw new Error('未知热点类型');
  return structuredClone(bodies[source]);
}
export function validateBody(source, body) {
  if (!body || typeof body !== 'object') throw new Error('内容源响应格式变化');
  if (typeof body.code === 'number' && body.code !== 0) throw new Error('内容源拒绝请求：' + (body.message || body.code));
  const checks = {
    bilibili: () => Array.isArray(body.list), bangumi: () => Array.isArray(body.result),
    'bilibili-rcmd': () => Array.isArray(body.data?.item),
    'cls-telegraph': () => body.data?.l && typeof body.data.l === 'object',
    douyin: () => Array.isArray(body.data?.word_list), 'tou-tiao': () => Array.isArray(body.data),
    weibo: () => Array.isArray(body.data?.realtime), zhihu: () => Array.isArray(body.data),
    'world-cup': () => Array.isArray((typeof body.response === 'string' ? JSON.parse(body.response) : body.response || body)?.matches),
  };
  if (!checks[source]?.()) throw new Error('内容源响应格式变化，未替代为空榜');
  const arrays = {
    bilibili: [body.list, item => typeof item.keyword === 'string' && typeof item.show_name === 'string'],
    bangumi: [body.result, item => Number.isFinite(item.date_ts) && Array.isArray(item.seasons)],
    'bilibili-rcmd': [body.data?.item, item => typeof item.bvid === 'string' && typeof item.uri === 'string' && typeof item.title === 'string' && item.stat && item.owner],
    douyin: [body.data?.word_list, item => typeof item.word === 'string'],
    'tou-tiao': [body.data, item => typeof item.Title === 'string' && typeof item.ClusterIdStr === 'string'],
    weibo: [body.data?.realtime, item => typeof item.word === 'string'],
    zhihu: [body.data, item => item.target && typeof item.target.title === 'string' && typeof item.target.url === 'string'],
  };
  if (arrays[source]) {
    const [items, valid] = arrays[source];
    if (!items.every(item => item && valid(item))) throw new Error('内容条目格式变化，保留上次成功数据');
  }
  return body;
}
