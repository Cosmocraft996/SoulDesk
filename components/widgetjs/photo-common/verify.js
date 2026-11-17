// 只在显式图片隔离验收中运行；不调用文件选择，也不读取用户目录。
const $ = id => document.getElementById(id);
const active = host => !host.preview && new URLSearchParams(location.search).get('verify') === 'widgetjs-photo';
async function until(test) {
  for (let n = 0; n < 240; n++) {
    if (await test()) return;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw Error('图片验收等待超时');
}
async function record(host, verification) {
  const storage = await host.request('storage.get');
  await host.request('storage.set', { ...storage, verification });
}
function inspect(image) {
  const canvas = document.createElement('canvas');
  canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
  const ctx = canvas.getContext('2d'); ctx.drawImage(image, 0, 0);
  return { width: canvas.width, height: canvas.height, red: [...ctx.getImageData(4, 4, 1, 1).data] };
}
export async function verifyPhoto({ host, getState, refresh, stop }) {
  if (!active(host)) return;
  const prior = (await host.request('storage.get')).verification || {};
  try {
    await until(async () => Boolean(await host.request('media.list')));
    await refresh(); stop();
    const listing = await host.request('media.list'), initial = getState(), image = inspect(initial.model.image);
    const imagesLoaded = image.width === 64 && image.height === 32 && image.red.join(',') === '255,0,0,255';
    const stylesLoaded = getComputedStyle(initial.model.image).objectFit === 'cover' && getComputedStyle($('photo')).position === 'relative';
    if (prior.uiVerified) {
      const settings = await host.request('settings.get');
      const retained = settings.duration === prior.testDuration && settings.radius === prior.testRadius && settings.random === false && listing.revision === prior.revision;
      await record(host, { ...prior, runtimeLoads: prior.runtimeLoads + 1, configurationRetained: retained, imagesLoaded, stylesLoaded, uiVerified: retained && imagesLoaded && stylesLoaded });
      return;
    }
    $('settings').click();
    const before = await host.request('settings.get');
    $('duration').value = '0'; $('form').dispatchEvent(new Event('submit', { cancelable: true }));
    await until(() => !$('form').querySelector('button[type=submit]').disabled);
    const invalidSaveRejected = JSON.stringify(await host.request('settings.get')) === JSON.stringify(before) && $('config-status').textContent.includes('INVALID_ARGUMENT');
    const testRadius = 8 + parseInt(host.instanceId.slice(0, 2), 16) % 65;
    $('duration').value = '1'; $('radius').value = String(testRadius); $('random').checked = false;
    $('form').dispatchEvent(new Event('submit', { cancelable: true }));
    await until(() => !$('form').querySelector('button[type=submit]').disabled); stop(); $('close').click();
    const saved = await host.request('settings.get'), configurationSaved = saved.duration === 1 && saved.radius === testRadius && !saved.random;
    $('next').click(); await until(() => getState().model.name === 'b.png' && !getState().model.loading); stop();
    const second = inspect(getState().model.image), nextWorks = second.width === 32 && second.height === 64;
    const retainedImage = getState().model.image;
    $('next').click(); await until(() => Boolean(getState().model.error) && !getState().model.loading); stop();
    const failedImageRetainsPrevious = getState().model.image === retainedImage && getState().model.name === 'b.png';
    $('next').click(); await until(() => getState().model.name === 'a.png' && !getState().model.loading); stop();
    $('previous').click(); await until(() => Boolean(getState().model.error) && !getState().model.loading); stop();
    $('previous').click(); await until(() => getState().model.name === 'b.png' && !getState().model.loading); stop();
    const previousWorks = getState().model.image.naturalHeight === 64;
    await refresh();
    await until(() => getState().model.name === 'b.png' && !getState().model.loading);
    $('photo').dispatchEvent(new MouseEvent('mousemove', { bubbles: true }));
    await new Promise(resolve => setTimeout(resolve, 1100)); stop();
    const carouselMoved = getState().model.name === 'b.png', mouseMovePauses = carouselMoved && !getState().model.error;
    let invalidReadRejected = false;
    try { await host.request('media.read', { name: '../not-allowed.png', revision: listing.revision }); } catch { invalidReadRejected = true; }
    const radiusApplied = getComputedStyle($('photo')).borderTopLeftRadius === testRadius + 'px';
    await record(host, { runtimeLoads: 1, imagesLoaded, stylesLoaded, configurationSaved, configurationRetained: configurationSaved, invalidSaveRejected, nextWorks, previousWorks, failedImageRetainsPrevious, carouselMoved, mouseMovePauses, invalidReadRejected, radiusApplied, image, second, files: listing.files.map(v => v.name), revision: listing.revision, testDuration: 1, testRadius, mediaGrantSeeded: true, nativeFileDialogVerified: false, scope: 'seeded-isolated-directory; real-media-gateway-and-WebView; programmatic-buttons-and-timer', uiVerified: imagesLoaded && stylesLoaded && configurationSaved && invalidSaveRejected && nextWorks && previousWorks && failedImageRetainsPrevious && carouselMoved && mouseMovePauses && invalidReadRejected && radiusApplied });
  } catch (error) { stop(); await record(host, { ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false }); }
}
export async function verifySticker({ host, apply, getImage }) {
  if (!active(host)) return;
  const stored = await host.request('storage.get'), prior = stored.verification || {};
  try {
    if (prior.uiVerified) {
      await apply(stored.image, false);
      const image = getImage(), detail = inspect(image), configurationRetained = stored.image === (await host.request('storage.get')).image && detail.width === 64 && detail.height === 32;
      await record(host, { ...prior, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, imagesLoaded: Boolean(image), uiVerified: configurationRetained });
      return;
    }
    const base = new Image();
    base.src = 'data:image/png;base64,' + TEST_PNG; await base.decode();
    const canvas = document.createElement('canvas'); canvas.width = 64; canvas.height = 32;
    const ctx = canvas.getContext('2d'); ctx.drawImage(base, 0, 0);
    const identity = host.instanceId.replaceAll('-', '');
    for (let n = 0; n < 16; n++) { ctx.fillStyle = `rgb(${parseInt(identity.slice(n * 2, n * 2 + 2), 16)},0,0)`; ctx.fillRect(n, 31, 1, 1); }
    const source = canvas.toDataURL('image/png'); await apply(source, true);
    const image = getImage(), detail = inspect(image), saved = await host.request('storage.get');
    const configurationSaved = saved.image === source, imagesLoaded = detail.width === 64 && detail.height === 32 && detail.red.join(',') === '255,0,0,255';
    const stylesLoaded = getComputedStyle(document.querySelector('.wrapper')).display === 'flex' && getComputedStyle($('sticker')).clipPath.startsWith('polygon(') && getComputedStyle(image).objectFit === 'cover';
    $('settings').click(); $('url').value = 'http://invalid.example/image.png';
    $('form').dispatchEvent(new Event('submit', { cancelable: true }));
    await until(() => !$('save').disabled);
    const invalidSaveRejected = (await host.request('storage.get')).image === source && $('message').textContent.includes('未保存') && getImage() === image;
    $('retry').click(); await until(() => !$('save').disabled);
    const retryRetainsImage = (await host.request('storage.get')).image === source && getImage() === image && $('message').textContent.includes('未保存');
    $('close').click(); window.dispatchEvent(new Event('resize'));
    const fitsContainer = parseFloat(image.style.width) <= innerWidth && parseFloat(image.style.height) <= innerHeight;
    await record(host, { runtimeLoads: 1, configurationSaved, configurationRetained: configurationSaved, imagesLoaded, stylesLoaded, invalidSaveRejected, retryRetainsImage, fitsContainer, image: detail, nativeFileDialogVerified: false, fixtureGenerated: true, scope: 'generated-data-image; actual-storage-save-and-WebView-decode; programmatic-form', uiVerified: configurationSaved && imagesLoaded && stylesLoaded && invalidSaveRejected && retryRetainsImage && fitsContainer });
  } catch (error) { await record(host, { ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false }); }
}

// 固定四区PNG；使用图片数据地址，遵循原组件禁止网络请求的策略。
const TEST_PNG = "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAgCAYAAACinX6EAAAASElEQVR4nO3QwQ0AMAgCQPZf2sYpaOJJ/MNlkmn+BwEAAAAAAAAAAAAAAAAAAADOAZT3b4H2AQAAAAAAAAAAAAAAAAAAANcAHnVr9KbdqOBTAAAAAElFTkSuQmCC";
