import test from 'node:test';
import assert from 'node:assert/strict';
import { MessageChannel } from 'node:worker_threads';
import { connectHost } from './host.js';

test('用户文件对话框超过普通10秒仍等待选择，其他请求按时拒绝，取消会正常返回', async () => {
  const originals = Object.fromEntries(['window','parent','location','setTimeout','clearTimeout'].map(key => [key, globalThis[key]]));
  const timers = new Map(), channel = new MessageChannel(), requests = [];
  let next = 0, listener;
  globalThis.setTimeout = (fn, delay) => { timers.set(++next, { fn, delay }); return next; };
  globalThis.clearTimeout = id => timers.delete(id);
  globalThis.location = { search: '?hostOrigin=http%3A%2F%2Ftauri.localhost' };
  globalThis.parent = { postMessage: () => listener({ source: parent, origin: 'http://tauri.localhost', data: { type:'souldesk:init', protocol:1, session:'session', instanceId:'instance' }, ports:[channel.port1] }) };
  globalThis.window = { parent, addEventListener: (_type, value) => { listener = value; }, removeEventListener: () => {} };
  channel.port2.onmessage = ({data}) => requests.push(data);
  try {
    const host = await connectHost();
    let documentFinished = false;
    const selected = host.request('documents.open').finally(() => { documentFinished = true; });
    const ordinary = host.request('settings.get');
    const rejected = assert.rejects(ordinary, /宿主请求超时/);
    for (const [id, timer] of [...timers]) if (timer.delay <= 10000) { timers.delete(id); timer.fn(); }
    await rejected; await new Promise(resolve => setImmediate(resolve));
    assert.equal(documentFinished, false, '文件选择不能被普通请求超时误判失败');
    const request = requests.find(item => item.method === 'documents.open');
    channel.port2.postMessage({ protocol:1, session:'session', id:request.id, result:null });
    assert.equal(await selected, null);
  } finally {
    channel.port1.close(); channel.port2.close();
    for (const [key,value] of Object.entries(originals)) { if (value === undefined) delete globalThis[key]; else globalThis[key] = value; }
  }
});
