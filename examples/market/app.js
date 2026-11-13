import { connectHost } from './host.js';
import { alreadyMet } from './price.js';
const $ = id => document.getElementById(id);
const host = await connectHost();
const settings = await host.request('settings.get');
$('title').textContent = settings.title;
let catalog = [], symbols = [], snapshot = {}, rules = [], editing, unsubscribe, offset = 0, loading = false;
const statusNames = { connected: '连接正常', idle: '未订阅', connecting: '正在连接', reconnecting: '正在恢复', stopped: '连接停止' };
const reasons = { initial_met: '启用时已到价', reached: '到价', cooldown_due: '冷却后补提醒', recovered_met: '恢复后到价' };
const deliveryNames = { pending: '待递送', sending: '递送中', sent: '系统调用成功', failed: '失败', unknown: '结果不确定', expired: '已过期', suppressed: '已关闭' };
const key = (symbol, priceKind) => ({ exchange: 'binance', market: 'usd_m', symbol, priceKind });
const clock = value => value ? new Date(value).toLocaleString('zh-CN', { hour12: false }) : '—';
const message = value => { $('message').textContent = value; $('message').hidden = !value; };
const node = (tag, text, className) => { const el = document.createElement(tag); if (text !== undefined) el.textContent = text; if (className) el.className = className; return el; };
const button = (text, action) => { const el = node('button', text); el.type = 'button'; el.onclick = () => void run(action); return el; };
async function run(action) { try { await action(); } catch (error) { message(String(error)); } }
async function confirmAction(text) {
  const dialog = node('dialog'); const content = node('p', text); const actions = node('div', undefined, 'actions');
  return new Promise(resolve => {
    const finish = value => { dialog.close(); dialog.remove(); resolve(value); };
    actions.append(button('确认', () => finish(true)), button('取消', () => finish(false))); dialog.append(content, actions); dialog.oncancel = event => { event.preventDefault(); finish(false); }; document.body.append(dialog); dialog.showModal();
  });
}
const quote = (symbol, kind) => snapshot.quotes?.find(q => q.key.symbol === symbol && q.key.priceKind === kind);
function renderPrices() {
  $('count').textContent = `${symbols.length}/20`;
  $('prices').replaceChildren();
  if (!symbols.length) { const row = node('tr'); const cell = node('td', '搜索并添加合约，最多20个', 'empty'); cell.colSpan = 5; row.append(cell); $('prices').append(row); }
  for (const [index, symbol] of symbols.entries()) {
    const contract = catalog.find(c => c.symbol === symbol), row = node('tr');
    const label = node('td', symbol); label.append(node('small', `${contract?.contractType === 'TRADIFI_PERPETUAL' ? 'TradFi 永续' : '加密永续'} · ${contract?.status ?? '目录未确认'}`)); row.append(label);
    for (const kind of ['last', 'mark']) {
      const q = quote(symbol, kind), cell = node('td', q?.price ?? '—', `number ${q?.quality !== 'fresh' ? 'stale' : ''}`);
      cell.title = q ? `源时间 ${clock(q.sourceUtcMs)}；接收距今 ${q.ageMs}ms` : '尚无有效报价';
      cell.append(node('small', !q ? '等待新价' : q.quality === 'fresh' ? '新鲜' : q.quality === 'stale' ? '报价过期' : '上次报价 · 恢复中')); row.append(cell);
    }
    const t = snapshot.tickers?.[symbol], delta = t?.changePercent;
    const daily = node('td', typeof delta === 'string' ? `${delta}%` : '—', `number ${delta?.startsWith('-') ? 'negative' : 'positive'}`); daily.title = `Last 24小时滚动统计；更新时间 ${clock(t?.time)}`; row.append(daily);
    const actions = node('td'); const up = button('↑', async () => { if (index === 0) return; const list = [...symbols]; [list[index - 1], list[index]] = [list[index], list[index - 1]]; await saveSymbols(list); }); up.disabled = index === 0;
    const down = button('↓', async () => { if (index === symbols.length - 1) return; const list = [...symbols]; [list[index + 1], list[index]] = [list[index], list[index + 1]]; await saveSymbols(list); }); down.disabled = index === symbols.length - 1;
    actions.append(up, down, button('×', () => saveSymbols(symbols.filter(s => s !== symbol)))); row.append(actions); $('prices').append(row);
  }
}
async function subscribe() {
  if (unsubscribe) { await unsubscribe(); unsubscribe = undefined; }
  if (!symbols.length) { snapshot = {}; renderPrices(); return; }
  unsubscribe = await host.subscribe(symbols.flatMap(s => [key(s, 'last'), key(s, 'mark')]), value => {
    if (value.error && !value.status) { message(value.error); return; } snapshot = value;
    $('connection').textContent = statusNames[value.status] ?? value.status;
    $('connection').title = value.error ?? '';
    $('received').textContent = `${value.subscriptions} 个共享源流 · 代次 ${value.generation}`;
    const sessions = Object.entries(value.sessions ?? {}).map(([event, v]) => `${({ EquityUpdate: '美国股票', CommodityUpdate: '商品', KR_EquityUpdate: '韩国股票', HK_EquityUpdate: '香港股票', CN_EquityUpdate: '中国股票', FXUpdate: '外汇' })[event] ?? event}：${v.generation === value.generation && value.sourceNow - v.time <= 10000 ? v.session : '时段数据过期'}`).join(' / ');
    if (sessions) $('tradfi-note').textContent = `标的市场时段：${sessions}。永续合约状态单列，休市不等于停盘。`;
    if (value.error) message(value.error);
    renderPrices(); updateWarning();
  });
}
async function saveSymbols(next) { if (next.length > 20) throw new Error('自选最多20个合约'); symbols = await host.request('watchlist.set', next); await subscribe(); renderSearch(); }
function renderSearch() { const term = $('search').value.trim().toUpperCase(); $('results').replaceChildren(); if (!term) return; for (const c of catalog.filter(c => c.status === 'TRADING' && (c.symbol.includes(term) || c.baseAsset.includes(term))).slice(0, 12)) { const b = button(`＋ ${c.symbol}${c.contractType === 'TRADIFI_PERPETUAL' ? ' · TradFi' : ''}`, async () => { await saveSymbols([...symbols, c.symbol]); $('search').value = ''; renderSearch(); }); b.disabled = symbols.includes(c.symbol) || symbols.length >= 20; $('results').append(b); } }
async function loadCatalog() { catalog = await host.request('market.catalog'); renderSearch(); renderPrices(); }
async function loadRules() { rules = await host.request('alerts.list'); renderRules(); }
function renderRules() {
  $('rule-list').replaceChildren(); if (!rules.length) $('rule-list').append(node('p', '尚无规则。从合约、价格口径和阈值开始。', 'empty'));
  for (const { rule: r, state: s } of rules) { const row = node('article', undefined, 'rule'); row.append(node('h3', r.name), node('div', `${r.key.symbol} / ${r.key.priceKind === 'last' ? 'Last' : 'Mark'} ${r.direction === 'up' ? '≥' : '≤'} ${r.threshold}`), node('small', `${!r.enabled ? '已暂停' : s.completed ? '单次已完成' : s.pending ? '冷却中待复核' : '监控中'} · ${r.mode === 'once' ? '单次' : '重复'} · 冷却 ${r.cooldownMs / 1000}s · 事件 ${s.sequence} · v${r.revision}`)); const actions = node('div', undefined, 'actions');
    actions.append(button('编辑', () => openEditor(r)), button(r.enabled ? '暂停' : '开启', async () => { const enabled = !r.enabled; if (enabled && !(await confirmAction('重新开启会作为新版本用第一笔新鲜报价检查，已满足时提醒一次。'))) return; await host.request('alerts.save', { rule: { ...r, revision: r.revision + 1, enabled }, expectedRevision: r.revision }); await loadRules(); }), button('删除', async () => { if (!(await confirmAction('删除此规则？已有历史保留。'))) return; await host.request('alerts.delete', { id: r.id, revision: r.revision }); await loadRules(); })); row.append(actions); $('rule-list').append(row);
  }
}
const form = $('rule-form');
function openEditor(rule) {
  editing = rule; form.reset(); $('editor-error').textContent = '';
  const options = catalog.filter(c => c.status === 'TRADING' || c.symbol === rule?.key.symbol);
  form.elements.symbol.replaceChildren(...options.map(c => { const o = node('option', `${c.symbol}${c.contractType === 'TRADIFI_PERPETUAL' ? ' · TradFi' : ''}`); o.value = c.symbol; return o; }));
  if (!options.length) throw new Error('目录尚未准备好，请检查宿主网络设置');
  for (const [name, value] of Object.entries(rule ? { name: rule.name, symbol: rule.key.symbol, kind: rule.key.priceKind, direction: rule.direction, threshold: rule.threshold, mode: rule.mode, cooldown: rule.cooldownMs / 1000 } : { symbol: symbols[0] ?? options[0].symbol })) form.elements[name].value = value;
  if (rule) for (const name of ['enabled', 'notification', 'sound']) form.elements[name].checked = rule[name];
  $('editor-title').textContent = rule ? '编辑到价规则' : '新建到价规则'; $('editor').showModal(); updateWarning();
}
function updateWarning() { if (!$('editor').open) return; const f = form.elements, q = quote(f.symbol.value, f.kind.value); let text = '启用后等待第一笔新鲜有效报价；已满足条件会提醒一次。'; try { if (q?.quality === 'fresh' && alreadyMet(q.price, f.threshold.value, f.direction.value)) text = `当前新鲜报价 ${q.price} 已满足条件。保存并启用后会提醒一次。`; } catch { /* 尚未输入合法阈值时保留一般提示。 */ } $('initial-warning').textContent = text; }
form.oninput = updateWarning;
form.onsubmit = async event => {
  event.preventDefault(); const f = form.elements; $('save-rule').disabled = true;
  try { const r = { id: editing?.id ?? crypto.randomUUID(), revision: (editing?.revision ?? 0) + 1, name: f.name.value.trim(), key: key(f.symbol.value, f.kind.value), direction: f.direction.value, threshold: f.threshold.value.trim(), mode: f.mode.value, cooldownMs: Number(f.cooldown.value) * 1000, enabled: f.enabled.checked, notification: f.notification.checked, sound: f.sound.checked };
    alreadyMet(r.threshold, r.threshold, r.direction);
    await host.request('alerts.save', { rule: r, expectedRevision: editing?.revision ?? null }); $('editor').close(); await loadRules(); message('规则已保存到核心后台');
  } catch (error) { $('editor-error').textContent = String(error); } finally { $('save-rule').disabled = false; }
};
async function loadHistory() {
  const events = await host.request('alerts.history', { offset, limit: 30 }); $('event-list').replaceChildren(); $('page').textContent = `第${offset / 30 + 1}页`; $('previous').disabled = offset === 0; $('next').disabled = events.length < 30;
  if (!events.length) $('event-list').append(node('p', '尚无触发记录', 'empty'));
  for (const e of events) { const row = node('article', undefined, 'event'); row.append(node('b', `${e.ruleName} · ${reasons[e.reason] ?? e.reason}`), node('p', `${e.key.symbol} / ${e.key.priceKind === 'last' ? 'Last' : 'Mark'} 观测 ${e.observedPrice} ${e.direction === 'up' ? '≥' : '≤'} ${e.threshold}`), node('small', `源时间 ${clock(e.sourceUtcMs)} · v${e.revision} / 第${e.sequence}次`));
    for (const channel of ['notification', 'sound']) { const line = node('div', `${channel === 'notification' ? '通知' : '声音'}：${deliveryNames[e[channel]] ?? e[channel]} · 已尝试 ${e[`${channel}Attempts`]}次`, 'delivery'); if (e[`${channel}Error`]) line.append(node('small', e[`${channel}Error`])); if (['failed', 'unknown', 'expired'].includes(e[channel]) && e[`${channel}Attempts`] < 3) line.append(button('重试此渠道', async () => { await host.request('alerts.retry', { id: e.id, channel }); await loadHistory(); })); row.append(line); } $('event-list').append(row);
  }
}
for (const el of document.querySelectorAll('[data-tab]')) el.onclick = () => void run(async () => { for (const b of document.querySelectorAll('[data-tab]')) b.classList.toggle('active', b === el); for (const id of ['quotes', 'rules', 'history']) $(id).hidden = id !== el.dataset.tab; if (el.dataset.tab === 'rules') await loadRules(); if (el.dataset.tab === 'history') await loadHistory(); });
$('search').oninput = renderSearch; $('search-form').onsubmit = e => e.preventDefault(); $('reload').onclick = () => void run(loadCatalog); $('new-rule').onclick = () => void run(() => openEditor()); $('cancel').onclick = () => $('editor').close(); $('refresh-history').onclick = () => void run(loadHistory); $('previous').onclick = () => void run(async () => { offset = Math.max(0, offset - 30); await loadHistory(); }); $('next').onclick = () => void run(async () => { offset += 30; await loadHistory(); });
await host.ready();
if (host.preview) { $('connection').textContent = '隔离试加载'; renderPrices(); }
else {
  await run(async () => { symbols = await host.request('watchlist.get'); await loadCatalog(); await subscribe(); await loadRules(); });
  setInterval(() => { if (loading) return; loading = true; void run(async () => { if (!catalog.length) await loadCatalog(); if (!$('rules').hidden) await loadRules(); if (!$('history').hidden) await loadHistory(); }).finally(() => { loading = false; }); }, 3000);
  if (new URLSearchParams(location.search).has('verify')) {
    try {
      const saved = await host.request('storage.get');
      const wait = async predicate => { const start = Date.now(); while (!predicate()) { if ($('editor-error').textContent) throw new Error($('editor-error').textContent); if (Date.now() - start > 100000) throw new Error('行情组件验收等待超时'); await new Promise(resolve => setTimeout(resolve, 250)); } };
      await wait(() => catalog.length > 0);
      if (!symbols.length) {
        const preferred = ['BTCUSDT', 'ETHUSDT', 'SOLUSDT', 'BNBUSDT', 'XRPUSDT', 'DOGEUSDT', 'ADAUSDT', 'LINKUSDT', 'AVAXUSDT', 'LTCUSDT', 'BCHUSDT', 'DOTUSDT', 'SUIUSDT', 'TRXUSDT', 'NEARUSDT', 'UNIUSDT', 'AAVEUSDT', 'ETCUSDT', 'FILUSDT'];
        const selected = preferred.filter(s => catalog.some(c => c.symbol === s && c.status === 'TRADING'));
        const tradfi = catalog.find(c => c.contractType === 'TRADIFI_PERPETUAL' && c.status === 'TRADING'); if (tradfi) selected.push(tradfi.symbol);
        for (const c of catalog) { if (selected.length >= 20) break; if (c.status === 'TRADING' && !selected.includes(c.symbol)) selected.push(c.symbol); }
        await saveSymbols(selected.slice(0, 20));
      }
      await wait(() => symbols.every(s => ['last', 'mark'].every(k => quote(s, k)?.quality === 'fresh')));
      await host.request('settings.update', { title: `验证-${host.instanceId.slice(0, 8)}` });
      if (!saved.ruleId) {
        openEditor(); form.elements.name.value = `验证-${host.instanceId.slice(0, 8)}`; form.elements.symbol.value = 'BTCUSDT'; form.elements.kind.value = 'mark'; form.elements.threshold.value = '0.00000001'; form.elements.notification.checked = false; form.elements.sound.checked = false; form.requestSubmit();
        await wait(() => !$('editor').open);
        await loadRules(); saved.ruleId = rules.find(v => v.rule.name === `验证-${host.instanceId.slice(0, 8)}`)?.rule.id;
        if (!saved.ruleId) throw new Error('实际规则表单未保存');
      }
      await host.request('storage.set', { ...saved, g2Ready: true, loads: (saved.loads ?? 0) + 1, origin: location.origin, hostTokenAbsent: typeof window.__SOULDESK_HOST_TOKEN__ === 'undefined', symbols, priceRows: $('prices').children.length, observedKeys: snapshot.quotes.length, generation: snapshot.generation, background: getComputedStyle(document.documentElement).backgroundColor });
    } catch (error) { message(String(error)); await host.request('storage.set', { g2Error: String(error), editorOpen: $('editor').open, editorError: $('editor-error').textContent, invalidFields: [...form.elements].filter(el => el.validity && !el.validity.valid).map(el => ({ name: el.name, value: el.value, message: el.validationMessage })) }); throw error; }
  }
}
