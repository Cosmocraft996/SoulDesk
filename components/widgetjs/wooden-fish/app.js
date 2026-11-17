import { connectHost } from './host.js';
const $ = id => document.getElementById(id);
const host = await connectHost();
const verifying = !host.preview && new URLSearchParams(location.search).get('verify') === 'widgetjs';
let settings = await host.request('settings.get');
let storage = await host.request('storage.get');
let count = 0, musicVersion = 0, disposed = false, saving = Promise.resolve();
const voices = new Set(), bgm = new Audio('./bgm.mp3');
bgm.loop = true;
bgm.preload = 'metadata';
if (verifying) bgm.muted = true;
function apply() {
  if (!/^#[0-9a-f]{6}$/i.test(settings.background) || !/^#[0-9a-f]{6}$/i.test(settings.foreground)) throw new Error('颜色配置无效');
  $('component').style.background = settings.background;
  $('component').style.color = settings.foreground;
  $('component').style.borderRadius = settings.radius + 'px';
}
function musicState(playing) { $('music').setAttribute('aria-pressed', String(playing)); $('music').setAttribute('aria-label', playing ? '暂停背景音乐' : '播放背景音乐'); }
async function music(playing, persist = true) {
  const version = ++musicVersion;
  if (!playing) { bgm.pause(); musicState(false); }
  else {
    try { await bgm.play(); }
    catch { if (version === musicVersion) { musicState(false); $('message').textContent = '音乐未播放，请点击♫重试'; } return; }
    if (disposed || version !== musicVersion) return;
    musicState(true); $('message').textContent = '';
  }
  if (persist && !host.preview) {
    const next = { ...storage, playMusic: playing };
    try { saving = saving.catch(() => {}).then(() => host.request('storage.set', next)); await saving; storage = next; }
    catch (error) { $('message').textContent = '音乐偏好未保存：' + error.message; }
  }
}
$('fish').addEventListener('click', () => {
  count++; $('count').textContent = '功德数：' + count;
  const merit = document.createElement('div'); merit.className = 'merits'; merit.textContent = '功德+1'; $('component').append(merit);
  const animation = merit.animate([{ transform: 'translateY(0)', opacity: 1 }, { transform: 'translateY(-40px)', opacity: 0 }], { duration: 1000 });
  animation.finished.then(() => merit.remove(), () => merit.remove());
  // 连续敲击保持重叠音效，同时限制活跃声道，避免无限创建播放器。
  if (voices.size >= 12) { const old = voices.values().next().value; old.pause(); voices.delete(old); }
  const hit = new Audio('./hit.mp3'); if (verifying) hit.muted = true; voices.add(hit);
  hit.addEventListener('ended', () => voices.delete(hit), { once: true });
  hit.play().catch(() => { voices.delete(hit); $('message').textContent = '敲击音效未播放'; });
});
$('music').addEventListener('click', () => music(bgm.paused));
$('settings').addEventListener('click', () => { for (const [key, value] of Object.entries(settings)) $('form').elements.namedItem(key).value = value; $('error').textContent = ''; $('dialog').showModal(); });
$('cancel').addEventListener('click', () => $('dialog').close());
$('form').addEventListener('submit', async event => {
  event.preventDefault(); const data = new FormData(event.target);
  const value = { background: data.get('background'), foreground: data.get('foreground'), radius: Number(data.get('radius')) };
  try { settings = await host.request('settings.update', value); apply(); $('dialog').close(); }
  catch (error) { $('error').textContent = error.message; }
});
window.addEventListener('pagehide', () => { disposed = true; ++musicVersion; bgm.pause(); voices.forEach(voice => voice.pause()); voices.clear(); }, { once: true });
apply(); await host.ready();
if (verifying) {
  const prior = storage.verification || {}, background = '#' + host.instanceId.replaceAll('-', '').slice(0, 6);
  async function until(test) { for (let n = 0; n < 120; n++) { if (test()) return; await new Promise(resolve => setTimeout(resolve, 25)); } throw Error('木鱼验收条件等待超时'); }
  let invalidSaveRejected = prior.invalidSaveRejected === true, actionsVerified = prior.actionsVerified === true;
  const initialCount = count;
  try {
    if (!prior.uiVerified) {
      $('fish').click(); $('fish').click(); $('fish').click();
      const tapped = count === 3 && $('count').textContent === '功德数：3' && document.querySelectorAll('.merits').length === 3;
      $('settings').click(); const form = $('form'), before = await host.request('settings.get');
      form.noValidate = true; form.elements.namedItem('radius').value = '99'; form.requestSubmit();
      await until(() => $('error').textContent.length > 0);
      invalidSaveRejected = $('dialog').open && JSON.stringify(await host.request('settings.get')) === JSON.stringify(before);
      form.elements.namedItem('background').value = background; form.elements.namedItem('foreground').value = '#ffffff'; form.elements.namedItem('radius').value = '12'; form.requestSubmit();
      await until(() => !$('dialog').open); form.noValidate = false;
      $('music').click(); await until(() => $('music').getAttribute('aria-pressed') === 'true'); await saving;
      const played = (await host.request('storage.get')).playMusic === true;
      $('music').click(); await until(() => $('music').getAttribute('aria-pressed') === 'false'); await saving;
      const paused = (await host.request('storage.get')).playMusic === false;
      $('music').click(); await until(() => $('music').getAttribute('aria-pressed') === 'true'); await saving;
      actionsVerified = tapped && played && paused && storage.playMusic === true;
      voices.forEach(voice => voice.pause()); voices.clear();
    }
    const image = $('fish').querySelector('img'); await image.decode();
    const probe = new Audio('./hit.mp3'); probe.muted = true; probe.preload = 'metadata';
    await until(() => probe.readyState >= 1 && bgm.readyState >= 1);
    const imagesLoaded = image.complete && image.naturalWidth > 0, audioLoaded = Number.isFinite(probe.duration) && Number.isFinite(bgm.duration);
    const fresh = await host.request('storage.get'), configurationSaved = settings.background === background && settings.foreground === '#ffffff' && settings.radius === 12;
    const stylesLoaded = getComputedStyle($('component')).borderRadius === '12px' && getComputedStyle($('component')).display === 'flex';
    const musicPreferenceRetained = fresh.playMusic === true, sessionCountReset = !prior.uiVerified || initialCount === 0;
    storage = { ...fresh, verification: { ...prior, runtimeLoads: (prior.runtimeLoads || 0) + 1, invalidSaveRejected, actionsVerified, imagesLoaded, audioLoaded, configurationSaved, stylesLoaded, musicPreferenceRetained, sessionCountReset, audioPlaybackMuted: true, uiVerified: invalidSaveRejected && actionsVerified && imagesLoaded && audioLoaded && configurationSaved && stylesLoaded && musicPreferenceRetained && sessionCountReset } };
    await host.request('storage.set', storage);
  } catch (error) { storage = { ...await host.request('storage.get'), verification: { ...prior, failure: String(error?.message || error) } }; await host.request('storage.set', storage); }
}
if (!host.preview && storage.playMusic === true) music(true, false);
