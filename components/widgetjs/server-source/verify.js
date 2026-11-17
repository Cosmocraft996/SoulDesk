import { nextTick } from 'vue';
const business = value => ({ version: value.version, servers: value.servers, master: value.master });
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const same = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
async function until(test) {
  for (let n = 0; n < 240; n++) { if (await test()) return; await new Promise(resolve => setTimeout(resolve, 50)); }
  throw Error('服务器监控验收等待超时');
}
export async function verifyServer({ host, model, getState, actions }) {
  if (host.preview || new URLSearchParams(location.search).get('verify') !== 'widgetjs-server') return;
  let prior = {};
  async function record(value) { if (getState().alive) await host.request('storage.set', { ...model.snapshot(), verification: value }); }
  async function readyRows() { await until(() => getState().rows.length > 0 && getState().rows.every(row => row.reading.data && !row.reading.busy)); await nextTick(); }
  async function saveForm() { document.querySelector('dialog[open] form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })); await until(() => !getState().busy); await nextTick(); }
  try {
    await until(async () => Boolean((await host.request('storage.get')).verification?.fixtureUrl));
    prior = (await host.request('storage.get')).verification;
    const endpoint = prior.fixtureUrl + '/' + host.instanceId;
    const token = 'souldesk-fixture-token';
    if (prior.uiVerified) {
      await readyRows();
      const configurationRetained = same(business(model.snapshot()), prior.expectedConfig) && same(await host.request('settings.get'), prior.testSettings), row = getState().rows[0];
      const businessStateVerified = row.view.cpu === 36.5 && row.view.memory === 50 && row.view.disk === 75;
      await record({ ...prior, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, businessStateVerified, uiVerified: configurationRetained && businessStateVerified });
      return;
    }
    actions.openEdit(); await nextTick();
    Object.assign(getState().draft, { name: '隔离服务器-' + host.instanceId.slice(0, 8), ip: '127.0.0.1', platform: 'Other', region: '项目回环夹具', api: { url: endpoint, token, interval: 1 } });
    await nextTick(); await saveForm();
    await readyRows();
    const saved = model.snapshot().servers[0], first = getState().rows[0], infoLoaded = saved.cpu?.cores === 4 && saved.os?.distro === 'Ubuntu';
    const businessStateVerified = first.view.cpu === 36.5 && first.view.memory === 50 && first.view.disk === 75;
    const labels = [...document.querySelectorAll('.circle-progress .label')].map(node => node.textContent.trim()), metricsShown = labels.join(',') === '37%,50%,75%';
    const canvases = [...document.querySelectorAll('.circle-progress canvas')], canvasDrawn = canvases.length === 3 && canvases.every(canvas => canvas.getContext('2d').getImageData(0, 0, 100, 100).data.some((v, i) => i % 4 === 3 && v > 0));
    const images = [...document.querySelectorAll('.server img')]; for (const image of images) await image.decode();
    const imagesLoaded = images.length === 2 && images.every(image => image.naturalWidth > 0), maskedIpShown = document.querySelector('.server-header').textContent.includes('127.**.**.**');
    actions.openEdit(saved); await nextTick(); getState().draft.ip = '999.1.1.1'; await nextTick(); await saveForm();
    const invalidServerRejected = model.snapshot().servers.length === 1 && model.snapshot().servers[0].ip === saved.ip && getState().message.includes('保存未完成');
    getState().draft.ip = saved.ip; getState().draft.api.url = endpoint + '/failure'; await nextTick(); await saveForm();
    const failureRetainsConfig = model.snapshot().servers[0].api.url === endpoint && getState().message.includes('503') && document.querySelector('dialog[open]') !== null;
    getState().draft.api.url = endpoint; await nextTick(); await saveForm();
    const retrySaved = getState().message.includes('本地配置已保存') && model.snapshot().servers.length === 1;
    actions.openSync(); getState().sync.url = endpoint; getState().sync.token = token;
    await actions.syncServers('push'); const pushed = getState().message.includes('被主服务器接受');
    await actions.syncServers('pull'); await nextTick();
    const remote = model.snapshot().servers.find(server => server.id.startsWith('fixture-remote-'));
    const pulledAndMerged = model.snapshot().servers.length === 2 && Boolean(remote) && model.snapshot().servers.some(server => server.id === saved.id);
    actions.closeDialogs(); actions.openEdit(remote); await nextTick(); document.querySelector('button[data-delete]').click(); await nextTick();
    [...document.querySelectorAll('dialog[open]')].at(-1).querySelector('form').dispatchEvent(new Event('submit', { cancelable: true, bubbles: true }));
    await until(() => !getState().busy); await nextTick();
    const deleteWorks = model.snapshot().servers.length === 1 && model.snapshot().servers[0].id === saved.id;
    actions.openTheme(); await nextTick(); getState().theme.fontSize = 11;
    await actions.saveTheme(); const invalidSaveRejected = (await host.request('settings.get')).fontSize !== 11 && getState().message.includes('外观未保存');
    const suffix = parseInt(host.instanceId.slice(-2), 16);
    Object.assign(getState().theme, { background: '#' + host.instanceId.replaceAll('-', '').slice(0, 6), color: '#ffffff', borderColor: '#333333', fontSize: 14 + suffix % 6, radius: 8 + suffix % 32 });
    await actions.saveTheme(); await nextTick();
    const testSettings = await host.request('settings.get'), configurationSaved = getState().message === '外观已保存';
    await readyRows();
    const stylesLoaded = getComputedStyle(document.querySelector('.widget')).borderTopLeftRadius === testSettings.radius + 'px';
    let invalidOperationRejected = false; try { await host.request('server.request', { kind: 'exec', connection: { url: endpoint, token } }); } catch { invalidOperationRejected = true; }
    const expectedConfig = business(model.snapshot());
    await record({ fixtureUrl: prior.fixtureUrl, fixtureData: true, runtimeLoads: 1, infoLoaded, businessStateVerified, metricsShown, canvasDrawn, imagesLoaded, maskedIpShown, invalidServerRejected, failureRetainsConfig, retrySaved, pushed, pulledAndMerged, deleteWorks, invalidSaveRejected, configurationSaved, configurationRetained: configurationSaved, stylesLoaded, invalidOperationRejected, expectedConfig, testSettings, actualClipboardVerified: false, actualRemoteAgentVerified: false, actualMouseVerified: false, scope: 'real-server-gateway-http-and-WebView; deterministic-loopback-fixture; programmatic-forms-charts-sync-delete-and-restore', uiVerified: infoLoaded && businessStateVerified && metricsShown && canvasDrawn && imagesLoaded && maskedIpShown && invalidServerRejected && failureRetainsConfig && retrySaved && pushed && pulledAndMerged && deleteWorks && invalidSaveRejected && configurationSaved && stylesLoaded && invalidOperationRejected });
  } catch (error) { await record({ ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false }); }
}
