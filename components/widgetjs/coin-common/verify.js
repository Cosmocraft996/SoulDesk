import { formatDecimal } from '../metal/metal-model.js';
const $ = id => document.getElementById(id);
const same = (a, b) => Object.keys(a).length === Object.keys(b).length && Object.entries(b).every(([key, value]) => a[key] === value);
async function until(test) {
  for (let n = 0; n < 600; n++) { if (await test()) return; await new Promise(resolve => setTimeout(resolve, 50)); }
  throw Error('虚拟币真实来源验收等待超时');
}
function active(host) { return !host.preview && new URLSearchParams(location.search).get('verify') === 'widgetjs-coins'; }
function pixels(canvas) { const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data; return { width: canvas.width, height: canvas.height, nonEmpty: data.some((v, i) => i % 4 === 3 && v > 0) }; }
async function saveForm(values) {
  $('settings').click();
  for (const [key, value] of Object.entries(values)) if ($(key).type === 'checkbox') $(key).checked = value; else $(key).value = String(value);
  $('form').dispatchEvent(new Event('submit', { cancelable: true }));
  await until(() => !$('save').disabled);
}
function themeSettings(host, before, medium) {
  const suffix = parseInt(host.instanceId.slice(-2), 16);
  return { ...before, refreshMs: medium ? 3000 : 30000, upColorRed: !before.upColorRed, background: '#' + host.instanceId.replaceAll('-', '').slice(0, 6), foreground: '#ffffff', primary: '#a6adc8', border: '#41444d', radius: 4 + suffix % 36 };
}
export async function verifySmall({ host, model, getState, isOld }) {
  if (!active(host)) return;
  const stored = await host.request('storage.get'), prior = stored.verification || {};
  let progress = { ...prior };
  async function record(value) { Object.assign(progress, value); if (!getState().disposed) await host.request('storage.set', { ...stored, verification: progress }); }
  async function quote() { await until(() => !model.state.loading && (model.state.quote || model.state.error)); if (!model.state.quote) throw Error(model.state.error || '来源未提供指数'); }
  function inspect() {
    const q = model.state.quote, settings = getState().settings;
    const old = Boolean(model.state.error || isOld(q, Date.now(), settings.refreshMs));
    return { instrument: q.instrument, price: q.price, timestamp: q.timestamp, reference: q.reference, basis: q.basis, sourceAgeMs: Date.now() - q.timestamp, old, staleTransparent: $('time').textContent.startsWith('旧数据 · ') === old, rawTitleMatches: $('price').title === '原始指数价：' + q.price, formattedPriceMatches: $('price').textContent === '$' + formatDecimal(q.price), identityMatches: $('identity').textContent === 'OKX · ' + settings.coin + ' · 指数价 Index · USD', sourceTimeShown: $('time').textContent !== '等待源时间', points: model.state.points.length, chart: pixels($('chart')) };
  }
  try {
    await quote(); const initial = inspect();
    if (prior.initialUiVerified) {
      const settings = await host.request('settings.get'), configurationRetained = same(settings, prior.testSettings);
      if (!configurationRetained) throw Error('恢复后的指数设置与已保存基准不一致');
      await record({ phase: 'restoring', restoreQuote: inspect() });
      let current;
      // 面板切换时可能先得到报价、稍后才有可绘制尺寸，不能把该瞬间当成首次测试。
      await until(() => {
        if (getState().disposed) return true;
        current = inspect();
        return current.instrument === settings.coin && current.rawTitleMatches && current.formattedPriceMatches && current.identityMatches && current.staleTransparent && current.points >= 1 && current.chart.nonEmpty;
      });
      if (getState().disposed) return;
      await record({ ...prior, phase: 'restored', failure: null, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, businessStateVerified: true, latestQuote: current, uiVerified: true }); return;
    }
    await record({ phase: 'initial-source-read', initialQuote: initial, fixtureData: false, uiVerified: false });
    const before = await host.request('settings.get');
    await saveForm({ proxyMode: 'http', proxyHost: 'invalid/host', proxyPort: 0 });
    const invalidSaveRejected = same(await host.request('settings.get'), before) && $('config').open && $('error').textContent.includes('设置未保存');
    const testSettings = { ...themeSettings(host, before, false), coin: 'ETH-USD' }; await saveForm(testSettings); await quote();
    const switched = inspect(), currencySwitchVerified = initial.instrument === 'BTC-USD' && switched.instrument === 'ETH-USD';
    $('refresh').click(); await quote(); const latest = inspect();
    const configurationSaved = same(await host.request('settings.get'), testSettings), businessStateVerified = latest.instrument === 'ETH-USD' && latest.rawTitleMatches && latest.formattedPriceMatches && latest.identityMatches && latest.sourceTimeShown && latest.staleTransparent && latest.chart.nonEmpty;
    const stylesLoaded = getComputedStyle($('root')).display === 'flex' && getComputedStyle($('root')).borderTopLeftRadius === testSettings.radius + 'px';
    const initialUiVerified = currencySwitchVerified && invalidSaveRejected && configurationSaved && businessStateVerified && stylesLoaded && $('coin').options.length === 14;
    await record({ runtimeLoads: 1, testSettings, initialQuote: initial, latestQuote: latest, currencySwitchVerified, invalidSaveRejected, configurationSaved, configurationRetained: configurationSaved, businessStateVerified, stylesLoaded, imagesLoaded: $('logo').complete && $('logo').naturalWidth > 0, availableCoins: $('coin').options.length, fixtureData: false, actualMouseVerified: false, customProxyConnectionVerified: false, scope: 'production-OKX-index-and-WebView; actual-BTC-to-ETH-switch-and-chart; programmatic-settings-and-restore', initialUiVerified, uiVerified: initialUiVerified });
  } catch (error) { await record({ ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false, fixtureData: false }); }
}
export async function verifyMedium({ host, model, display, rows, getState, quoteIsOld }) {
  if (!active(host)) return;
  const stored = await host.request('storage.get'), prior = stored.verification || {};
  let progress = { ...prior };
  async function record(value) { Object.assign(progress, value); if (!getState().disposed) await host.request('storage.set', { ...stored, verification: progress }); }
  async function live() { await until(() => model.state.snapshot?.connection === 'connected' && model.state.snapshot.quotes.size > 0); }
  function inspect() {
    const snapshot = model.state.snapshot, shown = display.quotes, now = Date.now(), coverage = [];
    let pricesMatch = true, missingTransparent = true, staleTransparent = true;
    for (const [currency, view] of rows) {
      const q = shown.get(currency);
      if (q) {
        pricesMatch &&= view.price.textContent === '$' + formatDecimal(q.price) && view.price.title === '原始网页报价：' + q.price;
        const old = Boolean(model.state.error || quoteIsOld(snapshot, q, now, getState().settings.refreshMs));
        staleTransparent &&= view.row.classList.contains('stale') === old;
        coverage.push({ currency, price: q.price, timestamp: q.timestamp, receivedAt: q.receivedAt, epoch: q.epoch, old });
      } else missingTransparent &&= view.price.textContent === '—' && view.meta.textContent.includes('等待来源');
    }
    return { provider: snapshot.provider, source: snapshot.source, channel: snapshot.channel, connectionId: snapshot.connectionId, connection: snapshot.connection, epoch: snapshot.epoch, rowCount: rows.size, receivedCurrencies: snapshot.quotes.size, displayedCurrencies: shown.size, pricesMatch, missingTransparent, staleTransparent, coverage };
  }
  try {
    await live();
    if (prior.initialUiVerified) {
      const settings = await host.request('settings.get'), configurationRetained = same(settings, prior.testSettings);
      if (!configurationRetained) throw Error('恢复后的网页币种设置与已保存基准不一致');
      let snapshot;
      await until(() => {
        if (getState().disposed) return true;
        snapshot = inspect();
        return snapshot.rowCount === 14 && snapshot.displayedCurrencies > 0 && snapshot.pricesMatch && snapshot.missingTransparent && snapshot.staleTransparent;
      });
      if (getState().disposed) return;
      await record({ ...prior, phase: 'restored', failure: null, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, businessStateVerified: true, latestSnapshot: snapshot, uiVerified: true }); return;
    }
    await record({ phase: 'initial-source-read', initialSnapshot: inspect(), fixtureData: false, uiVerified: false });
    const before = await host.request('settings.get');
    await saveForm({ proxyMode: 'http', proxyHost: 'invalid/host', proxyPort: 0 });
    const invalidSaveRejected = same(await host.request('settings.get'), before) && $('config').open && $('error').textContent.includes('设置未保存');
    const testSettings = themeSettings(host, before, true); await saveForm(testSettings); await live();
    $('refresh').click(); await until(() => !model.state.loading); await live();
    const snapshot = inspect(), configurationSaved = same(await host.request('settings.get'), testSettings), businessStateVerified = snapshot.rowCount === 14 && snapshot.pricesMatch && snapshot.missingTransparent && snapshot.staleTransparent && snapshot.displayedCurrencies > 0;
    const stylesLoaded = getComputedStyle($('root')).display === 'flex' && getComputedStyle($('root')).borderTopLeftRadius === testSettings.radius + 'px';
    const images = [...document.querySelectorAll('.logo')];
    const initialUiVerified = invalidSaveRejected && configurationSaved && businessStateVerified && stylesLoaded;
    await record({ runtimeLoads: 1, testSettings, latestSnapshot: snapshot, invalidSaveRejected, configurationSaved, configurationRetained: configurationSaved, businessStateVerified, stylesLoaded, imagesLoaded: images.length === 14, decodedImagesAtCheck: images.filter(img => img.complete && img.naturalWidth > 0).length, fixtureData: false, actualMouseVerified: false, customProxyConnectionVerified: false, scope: 'production-OKX-websocket-gateway-and-WebView; original-14-rows-prices-epoch-and-stale-status; programmatic-settings-and-restore', initialUiVerified, uiVerified: initialUiVerified });
  } catch (error) { await record({ ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false, fixtureData: false }); }
}
