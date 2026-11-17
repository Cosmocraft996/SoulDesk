import { nextTick } from 'vue';
const same = (a, b) => Object.keys(a).length === Object.keys(b).length && Object.entries(b).every(([key, value]) => a[key] === value);
async function until(test) {
  for (let n = 0; n < 240; n++) { if (await test()) return; await new Promise(resolve => setTimeout(resolve, 50)); }
  throw Error('打工进度验收等待超时');
}
function shiftAround(settings, endAfterMinutes) {
  const date = new Date(), minute = date.getHours() * 60 + date.getMinutes(), start = (minute - 60 + 1440) % 1440, end = (minute + endAfterMinutes) % 1440;
  return { ...settings, startHour: Math.floor(start / 60), startMinute: start % 60, endHour: Math.floor(end / 60), endMinute: end % 60 };
}
export async function verifyLabor({ host, getState, images }) {
  if (host.preview || new URLSearchParams(location.search).get('verify') !== 'widgetjs-labor') return;
  const stored = await host.request('storage.get'), prior = stored.verification || {};
  async function record(value) { await host.request('storage.set', { ...stored, verification: value }); }
  async function settled() { await until(() => !getState().loading && !getState().saving && getState().snapshot); await nextTick(); }
  async function save(values) {
    document.querySelector('.settings').click(); await nextTick();
    const inputs = document.querySelectorAll('dialog input');
    for (const [index, value] of Object.entries(values)) { inputs[index].value = String(value); inputs[index].dispatchEvent(new Event('input', { bubbles: true })); }
    await nextTick(); document.querySelector('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await until(() => !getState().saving); await settled();
  }
  try {
    await settled();
    const decoded = [];
    for (const [path, src] of Object.entries(images)) { const image = new Image(); image.src = src; await image.decode(); decoded.push({ name: path.split('/').at(-1), width: image.naturalWidth, height: image.naturalHeight }); }
    const allImagesDecoded = decoded.length === 11 && decoded.every(v => v.width > 0 && v.height > 0);
    if (prior.uiVerified) {
      const state = getState(), settings = await host.request('settings.get'), image = document.querySelector('.emoji'); await image.decode();
      const configurationRetained = same(settings, prior.testSettings), businessStateVerified = state.snapshot.endAt > state.snapshot.startAt && Math.abs(Number(document.querySelector('.percent').textContent.replace('%', '')) - state.snapshot.percent) <= .2;
      await record({ ...prior, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, businessStateVerified, imagesLoaded: allImagesDecoded && image.naturalWidth > 0, notificationTestSettings: shiftAround(settings, 3), notificationPreparedAt: Date.now(), uiVerified: configurationRetained && businessStateVerified && allImagesDecoded });
      return;
    }
    const before = await host.request('settings.get');
    await save({ 0: '06:00', 1: '06:00' });
    const invalidSaveRejected = same(await host.request('settings.get'), before) && document.querySelector('dialog').open && document.querySelector('.error').textContent.includes('相同');
    const nightEnd = new Date().getHours() === 5 ? 7 : 6;
    await save({ 0: '22:00', 1: String(nightEnd).padStart(2, '0') + ':00' });
    const night = getState().snapshot, nightShiftVerified = new Date(night.startAt).getHours() === 22 && new Date(night.endAt).getHours() === nightEnd && night.endAt > night.startAt;
    const suffix = parseInt(host.instanceId.slice(-2), 16), testSettings = shiftAround({ ...before, background: '#' + host.instanceId.replaceAll('-', '').slice(0, 6), radius: 12 + suffix % 36 }, 480);
    const time = (hour, minute) => String(hour).padStart(2, '0') + ':' + String(minute).padStart(2, '0');
    await save({ 0: time(testSettings.startHour, testSettings.startMinute), 1: time(testSettings.endHour, testSettings.endMinute), 2: testSettings.background, 3: testSettings.radius });
    const state = getState(), settings = await host.request('settings.get'), image = document.querySelector('.emoji'); await image.decode();
    const actualPercent = Number(document.querySelector('.percent').textContent.replace('%', '')), barPercent = parseFloat(document.querySelector('.progress').style.width);
    const businessStateVerified = Number.isFinite(actualPercent) && actualPercent >= 0 && actualPercent <= 100 && Math.abs(actualPercent - state.snapshot.percent) <= .2 && actualPercent === barPercent;
    const configurationSaved = same(settings, testSettings), imagesLoaded = allImagesDecoded && image.naturalWidth > 0;
    const stylesLoaded = getComputedStyle(document.querySelector('.container')).display === 'flex' && getComputedStyle(document.querySelector('.progress-outline')).outlineWidth === '4px' && getComputedStyle(document.querySelector('.widget')).borderTopLeftRadius === settings.radius + 'px';
    const clockShown = /^\d{2}:\d{2}$/.test(document.querySelector('.time').textContent), imageMatchesState = image.src === new URL(state.current.image, location.href).href;
    await record({ runtimeLoads: 1, invalidSaveRejected, nightShiftVerified, nightShift: { startAt: night.startAt, endAt: night.endAt }, configurationSaved, configurationRetained: configurationSaved, businessStateVerified, imagesLoaded, stylesLoaded, clockShown, imageMatchesState, decodedImages: decoded, percent: actualPercent, corePercent: state.snapshot.percent, testSettings: settings, notificationTestSettings: shiftAround(settings, 3), notificationPreparedAt: Date.now(), actualMouseVerified: false, actualNotificationDisplayVerified: false, scope: 'real-labor-core-and-WebView; programmatic-forms-night-shift-percentage-and-original-images', uiVerified: invalidSaveRejected && nightShiftVerified && configurationSaved && businessStateVerified && imagesLoaded && stylesLoaded && clockShown && imageMatchesState });
  } catch (error) { await record({ ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false }); }
}
