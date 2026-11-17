import { connectHost } from './host.js';
import { createNote, fontAfterWheel } from './note-model.js';

const input = document.getElementById('note'), status = document.getElementById('status'), retry = document.getElementById('retry');
const host = await connectHost();
const stored = await host.request('storage.get');
let inputError = '';
function render(state) {
  input.style.fontSize = state.value.fontSize + 'px';
  input.style.lineHeight = state.value.fontSize * 1.1 + 'px';
  status.textContent = inputError || (host.preview ? '预览，不写入实例' : state.status === 'error' ? '保存失败：' + state.error : state.status === 'saving' ? '保存中，请稍候…' : '已保存');
  retry.hidden = state.status !== 'error';
}
const note = createNote(stored, value => host.preview ? Promise.resolve() : host.request('storage.set', value), render);
input.value = note.state.value.text;
render(note.state);
input.disabled = false;
input.addEventListener('input', () => {
  try { inputError = ''; note.edit({ text: input.value }); }
  catch (error) { inputError = error.message; render(note.state); }
});
document.getElementById('page').addEventListener('wheel', event => {
  if (!event.ctrlKey) return;
  event.preventDefault(); event.stopPropagation();
  try { note.edit({ fontSize: fontAfterWheel(note.state.value.fontSize, event.deltaY) }); }
  catch (error) { status.textContent = error.message; }
}, { passive: false });
retry.addEventListener('click', () => {
  try { inputError = ''; note.edit({ text: input.value }); }
  catch (error) { inputError = error.message; render(note.state); }
});
await host.ready();
// 仅由显式隔离验收入口执行；正常使用和试加载均不写入测试内容。
if (!host.preview && new URLSearchParams(location.search).get('verify') === 'widgetjs') {
  const label = '便签' + host.instanceId.slice(0, 8), text = label + '\n第二行 · 🙂';
  const previous = stored.verification || {};
  let invalidSaveRejected = previous.invalidSaveRejected === true, actionsVerified = previous.actionsVerified === true;
  try {
    if (!previous.uiVerified) {
      input.value = '较早输入'; input.dispatchEvent(new Event('input', { bubbles: true }));
      input.value = text; input.dispatchEvent(new Event('input', { bubbles: true }));
      await note.retry();
      document.getElementById('page').dispatchEvent(new WheelEvent('wheel', { deltaY: -120, ctrlKey: true, bubbles: true, cancelable: true }));
      await note.retry();
      const before = await host.request('storage.get');
      input.value = '中'.repeat(1_400_000); input.dispatchEvent(new Event('input', { bubbles: true }));
      invalidSaveRejected = status.textContent.includes('未保存') && input.value.length === 1_400_000 && JSON.stringify(await host.request('storage.get')) === JSON.stringify(before);
      input.value = text; input.dispatchEvent(new Event('input', { bubbles: true })); await note.retry();
      const saved = await host.request('storage.get');
      actionsVerified = saved.text === text && saved.fontSize === 21 && status.textContent === '已保存';
    }
    const value = await host.request('storage.get'), style = getComputedStyle(document.getElementById('page'));
    const stylesLoaded = style.borderRadius === '10px' && style.backgroundImage.includes('linear-gradient') && getComputedStyle(input).fontSize === value.fontSize + 'px';
    const retained = value.text === text && value.fontSize === 21 && input.value === text;
    await note.edit({ verification: { ...previous, runtimeLoads: (previous.runtimeLoads || 0) + 1, savedLabel: label, invalidSaveRejected, actionsVerified, stylesLoaded, retained, uiVerified: invalidSaveRejected && actionsVerified && stylesLoaded && retained } });
  } catch (error) {
    await note.edit({ verification: { ...previous, failure: String(error?.message || error) } });
  }
}
