import { connectHost } from './host.js';
const host = await connectHost();
const settings = await host.request('settings.get');
document.body.className = 'compact-market';
document.body.innerHTML = `<main class="compact-shell"><header class="compact-header"><div><small>币安 · USD-M 永续</small><h1 id="compact-title"></h1></div><span id="compact-connection">等待核心</span></header><div class="compact-toolbar"><span id="compact-count">0 / 20 自选</span><div><button data-page="quotes">自选</button><button data-page="rules">警报</button><button data-page="history">历史</button></div></div><p id="compact-error" role="status" hidden></p><div class="compact-list"><table><thead><tr><th>合约 / Last 24h</th><th>Last 成交价</th><th>Mark 标记价</th></tr></thead><tbody id="compact-prices"></tbody></table></div><footer><span id="compact-time">等待新鲜报价</span><span>USDT · 本地规则</span></footer></main>`;
const $ = id => document.getElementById(id);
$('compact-title').textContent = settings.title;
let symbols = [], catalog = [], unsubscribe, lastSymbols = '', running = false;
const showError = error => { $('compact-error').textContent = String(error); $('compact-error').hidden = !error; };
for (const b of document.querySelectorAll('[data-page]')) b.onclick = async () => { try { await host.request('view.openDetails', { page: b.dataset.page }); } catch (e) { showError(e); } };
const statuses = { idle: '未订阅', connected: '连接正常', connecting: '连接中', disconnected: '连接中断' };
const quality = { fresh: '新鲜', stale: '过期', recovering: '恢复中', invalid: '无效' };
const cell = (tag, text, className) => { const el = document.createElement(tag); el.textContent = text; if (className) el.className = className; return el; };
function render(snapshot = {}) {
  $('compact-count').textContent = `${symbols.length} / 20 自选`;
  $('compact-connection').textContent = statuses[snapshot.status] ?? snapshot.status ?? (symbols.length ? '等待新价' : '未订阅');
  $('compact-connection').dataset.state = snapshot.status ?? 'idle';
  const body = $('compact-prices'); body.replaceChildren();
  if (!symbols.length) { const row = cell('tr', ''); const el = cell('td', '添加自选，开始观察市场', 'compact-empty'); el.colSpan = 3; row.append(el); body.append(row); }
  for (const symbol of symbols) {
    const row = cell('tr', ''), contract = catalog.find(c => c.symbol === symbol);
    const name = cell('td', symbol.replace(/USDT$/, '')); name.title = `${symbol} · 币安 USD-M · ${contract?.status ?? '目录未确认'}`;
    const delta = snapshot.tickers?.[symbol]?.changePercent;
    const sub = cell('small', `${typeof delta === 'string' ? delta + '%' : '—'}${contract?.contractType === 'TRADIFI_PERPETUAL' ? ' · TradFi' : ''}`, delta?.startsWith('-') ? 'negative' : 'positive');
    sub.title = `Last 24小时涨跌，非 Mark 涨跌。合约状态：${contract?.status ?? '未确认'}`; name.append(sub); row.append(name);
    for (const kind of ['last', 'mark']) {
      const q = snapshot.quotes?.find(q => q.key.symbol === symbol && q.key.priceKind === kind);
      const el = cell('td', q?.price ?? '—', `compact-number ${q?.quality === 'fresh' ? '' : 'stale'}`);
      const state = cell('small', q ? `${quality[q.quality] ?? q.quality} · ${q.ageMs < 1000 ? '<1' : Math.floor(q.ageMs / 1000)}秒` : '等待新价'); el.append(state);
      el.title = q ? `币安 USD-M ${symbol} / ${kind === 'last' ? 'Last 成交价' : 'Mark 标记价'}\n源时间 ${new Date(q.sourceUtcMs).toLocaleString('zh-CN', { hour12: false })}；接收距今 ${q.ageMs}ms` : '尚无有效报价'; row.append(el);
    }
    body.append(row);
  }
  $('compact-time').textContent = snapshot.sourceNow ? `源时钟 ${new Date(snapshot.sourceNow).toLocaleTimeString('zh-CN', { hour12: false })}` : '等待新鲜报价';
  if (snapshot.error) showError(snapshot.error);
}
async function refresh() {
  if (running) return; running = true;
  try {
    const next = await host.request('watchlist.get');
    if (!catalog.length) catalog = await host.request('market.catalog');
    if (JSON.stringify(next) !== lastSymbols) {
      if (unsubscribe) await unsubscribe(); unsubscribe = undefined;
      symbols = next; lastSymbols = JSON.stringify(next); render();
      if (symbols.length) unsubscribe = await host.subscribe(symbols.flatMap(symbol => ['last', 'mark'].map(priceKind => ({ exchange: 'binance', market: 'usd_m', symbol, priceKind }))), value => { if (value.error && !value.status) showError(value.error); else { showError(''); render(value); } });
    }
  } catch (e) { showError(e); lastSymbols = ''; } finally { running = false; }
}
await host.ready();
if (host.preview) render();
else { await refresh(); setInterval(refresh, 2000); }
