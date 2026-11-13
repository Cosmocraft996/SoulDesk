// 宿主接口 v1：组件只持有当前实例会话，不能传入其他 instanceId。
export function connectHost() {
  return new Promise((resolve, reject) => {
    const hostOrigin = new URLSearchParams(location.search).get('hostOrigin');
    if (!hostOrigin || window.parent === window) { reject(new Error('请从 SoulDesk 导入此组件')); return; }
    const timer = setTimeout(() => { window.removeEventListener('message', receive); reject(new Error('宿主握手超时')); }, 10000);
    function receive(event) {
      if (event.source !== parent || event.origin !== hostOrigin || event.data?.type !== 'souldesk:init' || event.data.protocol !== 1 || !event.ports[0]) return;
      clearTimeout(timer); window.removeEventListener('message', receive);
      const { session, instanceId, preview = false } = event.data;
      const port = event.ports[0];
      const pending = new Map();
      const subscriptions = new Map();
      port.onmessage = ({ data }) => {
        if (data.protocol !== 1 || data.session !== session) return;
        if (data.event) { const item = subscriptions.get(data.result?.subscriptionId); if (item) item(data.event, data.result); return; }
        const item = pending.get(data.id); if (!item) return;
        pending.delete(data.id); clearTimeout(item.timer);
        if (data.error) item.reject(new Error(data.error.message)); else item.resolve(data.result);
      };
      const host = { instanceId, preview, request(method, args = null) {
        return new Promise((resolve, reject) => {
          const id = crypto.randomUUID();
          const timer = setTimeout(() => { pending.delete(id); reject(new Error('宿主请求超时')); }, 10000);
          pending.set(id, { resolve, reject, timer });
          port.postMessage({ protocol: 1, session, id, method, args });
        });
      } };
      host.ready = () => host.request('runtime.ready');
      host.subscribe = async (keys, callback) => {
        const { subscriptionId } = await host.request('market.subscribe', keys);
        let sequence = 0;
        subscriptions.set(subscriptionId, (event, result) => {
          if (event === 'market:error') { callback({ error: result.message }); return; }
          if (result.sequence > sequence) { sequence = result.sequence; callback(result.data); }
        });
        return async () => { subscriptions.delete(subscriptionId); await host.request('market.unsubscribe', { subscriptionId }); };
      };
      resolve(host);
    }
    window.addEventListener('message', receive);
    parent.postMessage({ type: 'souldesk:ready', protocol: 1 }, hostOrigin);
  });
}
