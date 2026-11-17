import { host, contentFailure, contentStatus, hotspotEvidence } from './native';
import { providerForURL, previewBody, validateBody } from './endpoints.js';

function create(defaults: Record<string, any> = {}) {
  return {
    async get<T = any>(path: string, options: Record<string, any> = {}): Promise<{ data: T }> {
      const address = new URL(path, defaults.baseURL || location.href).href;
      const args = providerForURL(address, options.params);
      if (host.preview) return { data: previewBody(args.source) as T };
      const verify = new URLSearchParams(location.search).get('verify') === 'widgetjs-hotspots';
      let stage = 'network';
      if (verify) Object.assign(hotspotEvidence, { source: args.source, startedAt: Date.now(), completedAt: 0, outcome: 'loading', error: '', itemCount: null });
      contentStatus.loading++;
      try {
        const result = await host.request('content.get', args);
        stage = 'validation';
        const data = validateBody(args.source, result.data);
        if (verify) {
          const nested = data as any;
          const values: Record<string, any> = { bilibili: nested.list, bangumi: nested.result, 'bilibili-rcmd': nested.data?.item, 'cls-telegraph': nested.data?.l, douyin: nested.data?.word_list, 'tou-tiao': nested.data, weibo: nested.data?.realtime, zhihu: nested.data };
          if (args.source === 'world-cup') values[args.source] = (typeof nested.response === 'string' ? JSON.parse(nested.response) : nested.response || nested)?.matches;
          const items = values[args.source];
          Object.assign(hotspotEvidence, { outcome: 'success', endpoint: result.source, fetchedAt: result.fetchedAt, itemCount: Array.isArray(items) ? items.length : items && typeof items === 'object' ? Object.keys(items).length : null });
          if (args.source === 'bangumi') hotspotEvidence.seasonCount = nested.result.reduce((count: number, day: any) => count + day.seasons.length, 0);
        }
        contentStatus.error = ''; contentStatus.updatedAt = result.fetchedAt;
        return { data };
      } catch (error: any) { if (verify) Object.assign(hotspotEvidence, { outcome: stage + '-failed', error: String(error?.message || error) }); contentFailure(error); throw error; }
      finally { contentStatus.loading--; if (verify) hotspotEvidence.completedAt = Date.now(); }
    },
  };
}
export default { ...create(), create };
