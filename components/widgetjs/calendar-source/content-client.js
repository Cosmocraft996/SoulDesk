// 新宿主启用短缓存；旧宿主明确拒绝新增元数据时，保留原公开读取方式。
export async function readAlmanac(host, forceRefresh = false) {
  try {
    return await host.request('content.get', { source: 'itime-calendar', cache: true, ...(forceRefresh === true ? { forceRefresh: true } : {}) });
  } catch (error) {
    if (!String(error?.message ?? error).includes('黄历来源不接受额外参数')) throw error;
    return host.request('content.get', { source: 'itime-calendar' });
  }
}
