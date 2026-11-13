import { connectHost } from './host.js';
const message = document.getElementById('message');
const input = document.getElementById('input');
const status = document.getElementById('status');
try {
  const host = await connectHost();
  const settings = await host.request('settings.get');
  message.textContent = input.value = settings.message;
  let saving = Promise.resolve();
  document.getElementById('save').addEventListener('click', () => {
    saving = (async () => {
    try {
      const saved = await host.request('settings.update', { message: input.value });
      message.textContent = saved.message;
      status.textContent = '已保存到当前实例';
    } catch (error) { status.textContent = `保存失败：${error.message}`; throw error; }
    })();
    void saving.catch(() => {});
  });
  await host.ready();
  if (new URLSearchParams(location.search).has('verify')) {
    input.value = `实例-${host.instanceId.slice(0, 8)}`;
    document.getElementById('save').click();
    await saving;
    const saved = await host.request('settings.get');
    await host.request('storage.set', { moduleExecuted: true, buttonSave: saved.message === input.value, savedMessage: saved.message, standardsMode: document.compatMode, origin: location.origin });
  }
} catch (error) { status.textContent = `加载失败：${error.message}`; throw error; }
