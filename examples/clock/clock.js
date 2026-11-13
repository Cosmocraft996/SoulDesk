import { connectHost } from './host.js';

const $ = id => document.getElementById(id);
try {
  const host = await connectHost();
  let settings = await host.request('settings.get');
  $('bridge').textContent = `桥接已连接 · ${host.instanceId.slice(0, 8)}`;
  $('name').value = settings.title;
  $('seconds').checked = settings.showSeconds;
  function render() {
    $('title').textContent = settings.title;
    const now = new Date();
    $('clock').textContent = now.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit', ...(settings.showSeconds ? { second: '2-digit' } : {}) });
    $('date').textContent = now.toLocaleDateString('zh-CN', { year: 'numeric', month: 'long', day: 'numeric', weekday: 'long' });
  }
  render(); setInterval(render, 1000);
  let saving = Promise.resolve();
  $('save').addEventListener('click', () => {
    saving = (async () => {
      try { settings = await host.request('settings.update', { title: $('name').value, showSeconds: $('seconds').checked }); $('message').textContent = '当前实例配置已保存到宿主'; render(); }
      catch (error) { $('message').textContent = `保存失败：${error.message}`; throw error; }
    })();
    void saving.catch(() => {});
  });
  $('name').addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); $('save').click(); } });
  await host.ready();
  // 仅显式验收模式执行：经真实消息桥读写，记录模块、CSS 和来源证据。
  if (new URLSearchParams(location.search).has('verify')) {
    const titleBefore = settings.title;
    $('name').value = `验证-${host.instanceId.slice(0, 8)}`;
    $('save').click();
    await saving;
    const saved = await host.request('settings.get');
    let denied = false;
    try { await host.request('shell.exec', {}); } catch { denied = true; }
    let directHostResult = '接口不存在';
    if (window.__TAURI_INTERNALS__?.invoke) {
      directHostResult = await Promise.race([
        window.__TAURI_INTERNALS__.invoke('snapshot').then(() => '意外成功', () => '明确拒绝'),
        new Promise(resolve => setTimeout(() => resolve('未收到回执'), 400)),
      ]);
    }
    await host.request('storage.set', { moduleExecuted: true, buttonSave: saved.title === $('name').value, hostTokenAbsent: typeof window.__SOULDESK_HOST_TOKEN__ === 'undefined', directHostResult, origin: location.origin, background: getComputedStyle(document.documentElement).backgroundColor, titleBefore, savedTitle: saved.title, denied });
    render();
  }
} catch (error) { $('message').textContent = error.message; throw error; }
