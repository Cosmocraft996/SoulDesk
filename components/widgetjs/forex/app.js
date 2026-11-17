import { connectHost } from './host.js';
import { CurrencyMapping, PopularCurrencyMapping, SoutheastAsiaCurrencyMapping } from './currencies.js';
import { convertAmount, createRates } from './forex-model.js';
const $ = id => document.getElementById(id), inputs = [$('amount-0'), $('amount-1')], selects = [$('currency-0'), $('currency-1')];
const host = await connectHost();
let settings = await host.request('settings.get'), lastEdited = 0, inputError = '', savingPair = false, disposed = false, timer;
const lookup = code => CurrencyMapping.find(item => item.code === code);
if (!lookup(settings.from) || !lookup(settings.to)) throw new Error('已保存的币种不受当前组件支持，请保留配置后检查');
const rates = createRates(async ([from, to]) => {
  if (from === to) return { body: { ResultCode: '0', Result: { revCode: [{ price: '1', list: [] }] } } };
  const result = await host.request('content.get', { source: 'forex', from, to });
  return { body: result.data, fetchedAt: result.fetchedAt };
}, render);
function render() {
  const state = rates.state, values = [lookup(settings.from), lookup(settings.to)];
  const activeRate = state.pair?.[0] === settings.from && state.pair?.[1] === settings.to ? state.rate : null;
  $('title').textContent = values.map(value => value.sortName || value.name).join('兑');
  values.forEach((currency, index) => { $('flag-' + index).src = currency.flag; $('symbol-' + index).textContent = currency.symbol; selects[index].value = currency.code; selects[index].disabled = savingPair; inputs[index].disabled = savingPair || host.preview; });
  $('component').style.background = settings.background; $('component').style.color = settings.foreground; $('component').style.borderRadius = settings.radius + 'px';
  $('refresh').classList.toggle('rotating', state.loading); $('refresh').disabled = savingPair || host.preview || state.loading;
  if (activeRate && !savingPair) {
    try { inputs[1 - lastEdited].value = convertAmount(inputs[lastEdited].value, activeRate.price, lastEdited === 1); }
    catch (error) { inputError = error.message; inputs[1 - lastEdited].value = ''; }
  } else if (!activeRate) inputs[1 - lastEdited].value = '';
  $('changes').replaceChildren();
  for (const item of activeRate?.list || []) { const span = document.createElement('span'); span.textContent = item.value; span.title = item.text; span.className = item.value.includes('+') ? 'up' : item.value.includes('-') ? 'down' : ''; $('changes').append(span); }
  $('status').textContent = host.preview ? '预览 · 不读取汇率' : savingPair ? '正在保存币种选择，暂停换算…' : inputError || (state.error ? '获取失败：' + state.error + (activeRate ? ' · 当前为上次成功汇率' : '') : state.loading ? activeRate ? '正在更新 · 当前按上次汇率计算' : '正在更新…' : settings.from === settings.to ? '同币种按1:1换算，无需联网' : state.fetchedAt ? '更新于 ' + new Date(state.fetchedAt).toLocaleString() : '等待公开数据');
}
for (const select of selects) for (const [label, currencies] of [['常用币种', PopularCurrencyMapping], ['东南亚', SoutheastAsiaCurrencyMapping]]) {
  const group = document.createElement('optgroup'); group.label = label;
  currencies.forEach(currency => { const option = document.createElement('option'); option.value = currency.code; option.textContent = currency.name + '/' + currency.code; group.append(option); }); select.append(group);
}
selects.forEach(select => select.addEventListener('change', async () => {
  if (savingPair) return;
  const next = { ...settings, from: selects[0].value, to: selects[1].value };
  savingPair = true; inputError = ''; render();
  try { settings = await host.request('settings.update', next); lastEdited = 0; }
  catch (error) { inputError = '币种选择未保存：' + error.message; }
  finally { savingPair = false; render(); }
  if (!inputError && !host.preview) rates.refresh(settings.from, settings.to);
}));
inputs.forEach((input, index) => input.addEventListener('input', () => { lastEdited = index; inputError = ''; render(); }));
$('refresh').addEventListener('click', () => { inputError = ''; rates.refresh(settings.from, settings.to); });
$('settings').addEventListener('click', () => { for (const key of ['background', 'foreground', 'radius']) $('theme-form').elements.namedItem(key).value = settings[key]; $('theme-error').textContent = ''; $('dialog').showModal(); });
$('cancel').addEventListener('click', () => $('dialog').close());
$('theme-form').addEventListener('submit', async event => {
  event.preventDefault(); const submit = event.target.querySelector('[type=submit]'); if (submit.disabled) return; submit.disabled = true;
  const form = new FormData(event.target), next = { ...settings, background: form.get('background'), foreground: form.get('foreground'), radius: Number(form.get('radius')) };
  try { settings = await host.request('settings.update', next); render(); $('dialog').close(); }
  catch (error) { $('theme-error').textContent = '保存失败：' + error.message; }
  finally { submit.disabled = false; }
});
render(); await host.ready();
if (!host.preview && new URLSearchParams(location.search).get('verify') === 'widgetjs') {
  const storage = await host.request('storage.get'), prior = storage.verification || {}, background = '#' + host.instanceId.replaceAll('-', '').slice(0, 6);
  async function until(test) { for (let n = 0; n < 160; n++) { if (test()) return; await new Promise(resolve => setTimeout(resolve, 25)); } throw Error('汇率验收条件等待超时'); }
  let liveSource = prior.liveSource, invalidSaveRejected = prior.invalidSaveRejected === true, actionsVerified = prior.actionsVerified === true;
  try {
    if (!prior.uiVerified) {
      // 先实际请求原源；此结果单独记录，后面的1:1边界不冒充联网换算成功。
      await rates.refresh(settings.from, settings.to);
      liveSource = { from: settings.from, to: settings.to, state: rates.state.error ? 'unavailable' : 'available', error: rates.state.error || null, rate: rates.state.rate?.price || null, fetchedAt: rates.state.fetchedAt || null, renderedStatus: $('status').textContent, fromAmount: inputs[0].value, toAmount: inputs[1].value };
      liveSource.resultShownHonestly = rates.state.error ? $('status').textContent.includes('获取失败') && inputs[1].value === '' : inputs[1].value === convertAmount(inputs[0].value, rates.state.rate.price);
      selects[1].value = 'USD'; selects[1].dispatchEvent(new Event('change', { bubbles: true }));
      await until(() => !savingPair && !rates.state.loading && settings.to === 'USD');
      inputs[0].value = '9007199254740993.12345678'; inputs[0].dispatchEvent(new Event('input', { bubbles: true }));
      const forward = inputs[1].value === '9007199254740993.12345678';
      inputs[1].value = '0.000000005'; inputs[1].dispatchEvent(new Event('input', { bubbles: true }));
      const reverse = inputs[0].value === '0.00000001';
      inputs[0].value = '1e3'; inputs[0].dispatchEvent(new Event('input', { bubbles: true }));
      const invalidAmount = inputs[1].value === '' && $('status').textContent.includes('不支持');
      inputs[0].value = '100'; inputs[0].dispatchEvent(new Event('input', { bubbles: true }));
      const corrected = inputs[1].value === '100';
      $('settings').click(); const form = $('theme-form'), before = await host.request('settings.get');
      form.noValidate = true; form.elements.namedItem('radius').value = '99'; form.requestSubmit();
      await until(() => $('theme-error').textContent.length > 0);
      invalidSaveRejected = $('dialog').open && JSON.stringify(before) === JSON.stringify(await host.request('settings.get'));
      form.elements.namedItem('background').value = background; form.elements.namedItem('foreground').value = '#ffffff'; form.elements.namedItem('radius').value = '12'; form.requestSubmit();
      await until(() => !$('dialog').open); form.noValidate = false;
      actionsVerified = liveSource.resultShownHonestly && forward && reverse && invalidAmount && corrected && selects.every(select => new Set([...select.options].map(o => o.value)).size === 14);
    }
    await rates.refresh(settings.from, settings.to);
    const flags = await Promise.allSettled([$('flag-0').decode(), $('flag-1').decode()]);
    const imagesLoaded = flags.every(result => result.status === 'fulfilled') && [$('flag-0'), $('flag-1')].every(image => image.naturalWidth > 0);
    const stylesLoaded = getComputedStyle($('component')).borderRadius === '12px' && getComputedStyle(inputs[0]).fontSize === '24px';
    const configurationSaved = settings.from === 'USD' && settings.to === 'USD' && settings.background === background && settings.foreground === '#ffffff' && settings.radius === 12;
    await host.request('storage.set', { ...storage, verification: { ...prior, liveSource, runtimeLoads: (prior.runtimeLoads || 0) + 1, invalidSaveRejected, actionsVerified, imagesLoaded, stylesLoaded, configurationSaved, uiVerified: invalidSaveRejected && actionsVerified && stylesLoaded && configurationSaved, sourceAvailabilityVerified: liveSource?.state === 'available', sameCurrencyOnlyForDeterministicChecks: true } });
  } catch (error) { await host.request('storage.set', { ...storage, verification: { ...prior, liveSource, failure: String(error?.message || error) } }); }
}
if (!host.preview) { rates.refresh(settings.from, settings.to); timer = setInterval(() => { if (!disposed && !rates.state.loading && !savingPair) rates.refresh(settings.from, settings.to); }, 60000); }
window.addEventListener('pagehide', () => { disposed = true; clearInterval(timer); }, { once: true });
