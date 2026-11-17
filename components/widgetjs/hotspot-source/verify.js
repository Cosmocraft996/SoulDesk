import { nextTick } from 'vue';
import { settings, contentStatus, hotspotEvidence, imageStatus } from './native';
const same = (a, b) => Object.keys(a).length === Object.keys(b).length && Object.entries(b).every(([key, value]) => a[key] === value);
async function until(test) {
  for (let n = 0; n < 400; n++) { if (await test()) return; await new Promise(resolve => setTimeout(resolve, 50)); }
  throw Error('热点组件验收等待超时');
}
export async function verifyHotspot(host, getState) {
  if (host.preview || new URLSearchParams(location.search).get('verify') !== 'widgetjs-hotspots') return;
  const stored = await host.request('storage.get'), prior = stored.verification || {}, variant = document.body.dataset.variant;
  async function record(value) { if (getState().alive) await host.request('storage.set', { ...stored, verification: value }); }
  try {
    await until(() => hotspotEvidence.completedAt > 0 && !contentStatus.loading); await nextTick();
    const source = { ...hotspotEvidence }, footer = document.querySelector('.content-status').textContent.trim();
    let dateSwitches = 0;
    if (variant === 'bangumi' && source.outcome === 'success') {
      for (const button of document.querySelectorAll('.weekday')) {
        button.click(); dateSwitches++; await nextTick();
        if (document.querySelector('.bangumi-cover img')) break;
      }
    }
    const avatarReplaced = variant !== 'bangumi' || source.outcome !== 'success' || !source.seasonCount || Boolean(document.querySelector('.bangumi-cover img')) && !document.querySelector('el-avatar');
    const sourceIdentityVerified = source.source === variant, statusTransparent = source.outcome === 'success' ? contentStatus.updatedAt > 0 && (footer.startsWith('更新于') || footer.startsWith('获取失败：')) : Boolean(contentStatus.error) && footer.startsWith('获取失败：');
    const root = document.getElementById('hotspot-root'), stylesLoaded = getComputedStyle(root).position === 'relative' && getComputedStyle(document.querySelector('.content-view')).position === 'absolute';
    const rendered = document.querySelector('.content-view').children.length > 0;
    if (prior.uiVerified) {
      const actual = await host.request('settings.get'), configurationRetained = same(actual, prior.testSettings) && same({ ...settings }, prior.testSettings);
      await record({ ...prior, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, stylesLoaded, rendered, latestSource: source, latestStatus: footer, sourceIdentityVerified, statusTransparent, avatarReplaced, dateSwitches, failedExternalImages: imageStatus.failed, uiVerified: configurationRetained && stylesLoaded && rendered && sourceIdentityVerified && statusTransparent && avatarReplaced });
      return;
    }
    document.querySelector('.settings-button').click(); await nextTick();
    const before = await host.request('settings.get'), inputs = document.querySelectorAll('dialog input');
    inputs[2].value = '41'; inputs[2].dispatchEvent(new Event('input', { bubbles: true })); await nextTick();
    document.querySelector('form').dispatchEvent(new Event('submit', { cancelable: true })); await until(() => !getState().saving); await nextTick();
    const invalidSaveRejected = same(await host.request('settings.get'), before) && document.querySelector('dialog').open && document.querySelector('form output').textContent.includes('保存失败');
    const suffix = parseInt(host.instanceId.slice(-2), 16), testSettings = { background: '#' + host.instanceId.replaceAll('-', '').slice(0, 6), foreground: '#ffffff', radius: 4 + suffix % 36 };
    for (const [index, value] of Object.values(testSettings).entries()) { inputs[index].value = String(value); inputs[index].dispatchEvent(new Event('input', { bubbles: true })); }
    await nextTick(); document.querySelector('form').dispatchEvent(new Event('submit', { cancelable: true })); await until(() => !getState().saving); await nextTick();
    const configurationSaved = same(await host.request('settings.get'), testSettings) && same({ ...settings }, testSettings) && !document.querySelector('dialog').open;
    const themeApplied = root.style.getPropertyValue('--widget-background-color') === testSettings.background && getComputedStyle(root).borderTopLeftRadius === testSettings.radius + 'px';
    let unknownSourceRejected = false; try { await host.request('content.get', { source: 'unlisted-fixture-source' }); } catch { unknownSourceRejected = true; }
    const images = [...document.querySelectorAll('.content-view img')];
    await record({ runtimeLoads: 1, variant, source, latestSource: source, latestStatus: footer, sourceIdentityVerified, statusTransparent, rendered, stylesLoaded, invalidSaveRejected, configurationSaved, configurationRetained: configurationSaved, businessStateVerified: rendered && statusTransparent && sourceIdentityVerified, themeApplied, unknownSourceRejected, testSettings, avatarReplaced, dateSwitches, failedExternalImages: imageStatus.failed, imageCount: images.length, decodedImagesAtCheck: images.filter(image => image.complete && image.naturalWidth > 0).length, actualLinksOpened: false, actualMouseVerified: false, fixtureData: false, scope: 'real-public-endpoints-through-production-gateway-and-WebView; programmatic-settings-and-status; availability-recorded-separately', uiVerified: sourceIdentityVerified && statusTransparent && rendered && stylesLoaded && invalidSaveRejected && configurationSaved && themeApplied && unknownSourceRejected && avatarReplaced });
  } catch (error) { await record({ ...prior, variant, phase: 'failed', failure: String(error?.message || error), latestSource: { ...hotspotEvidence }, uiVerified: false }); }
}
