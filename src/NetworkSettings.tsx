import { useEffect, useState } from 'react';
import { invoke } from './host';
interface Network { mode: 'system' | 'direct' | 'http' | 'socks5'; host?: string; port?: number }
interface MonitorGap {
  key: { symbol: string; market: 'spot' | 'usd_m'; priceKind: 'last' | 'mark' };
  fromUtcMs: number | null;
  toUtcMs: number;
  reason: string;
}
interface Data { config: Network; market: { status: string; error: string | null; generation: number; subscriptions: number; received: number; interruptions: number; queueOverflows: number }; monitorGaps: MonitorGap[] }
const gapDate = (time: number | null) => time === null ? '起点无法确认' : new Date(time).toLocaleString();
function gapText(gap: MonitorGap) {
  const { market, priceKind, symbol } = gap.key;
  return `币安 ${symbol} ${market === 'spot' ? '现货' : '永续'} ${priceKind === 'mark' ? '标记价' : '成交价'} · ${gapDate(gap.fromUtcMs)} 至 ${gapDate(gap.toUtcMs)} · ${gap.reason}`;
}
export function NetworkSettings() {
  const [config, setConfig] = useState<Network>({ mode: 'system' });
  const [data, setData] = useState<Data>();
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const refresh = async () => { const value = await invoke<Data>('business_settings', { action: 'get' }); setData(value); return value; };
  useEffect(() => { let alive = true; void refresh().then(v => { if (alive) setConfig(v.config); }).catch(e => { if (alive) setMessage(String(e)); }); const timer = setInterval(() => void refresh().catch(e => { if (alive) setMessage(String(e)); }), 3000); return () => { alive = false; clearInterval(timer); }; }, []);
  const run = async (action: 'set' | 'reconnect') => { setBusy(true); setMessage(''); try { await invoke('business_settings', { action, config }); setMessage(action === 'set' ? '网络设置已保存，正在重新连接行情。' : '已安排受控重连'); await refresh(); } catch (e) { setMessage(`操作失败：${String(e)}`); } finally { setBusy(false); } };
  const labels: Record<string, string> = { connected: '连接正常', reconnecting: '正在恢复', connecting: '正在连接', idle: '目录已就绪，等待消费者', stopped: '已停止' };
  const gaps = data?.monitorGaps ?? [];
  return <section className="settings-panel" id="market-network" tabIndex={-1}>
    <h2>公开行情网络</h2>
    <p>公开行情请求与实时行情流共用连接设置。系统模式支持静态代理。</p>
    <form onSubmit={event => { event.preventDefault(); void run('set'); }}>
      <div className="settings-network-form">
        <label>连接方式<select value={config.mode} onChange={event => setConfig({ mode: event.target.value as Network['mode'], host: '127.0.0.1', port: 7890 })}>
          {[['system', '系统静态代理'], ['direct', '直连'], ['http', '手动 HTTP'], ['socks5', '手动 SOCKS5']].map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
        {(config.mode === 'http' || config.mode === 'socks5') && <>
          <label>代理主机<input value={config.host ?? ''} required onChange={event => setConfig({ ...config, host: event.target.value })} /></label>
          <label>端口<input type="number" min="1" max="65535" required value={config.port ?? ''} onChange={event => setConfig({ ...config, port: Number(event.target.value) })} /></label>
        </>}
        <button disabled={busy}>保存并连接</button>
      </div>
    </form>
    <div className="actions"><button disabled={busy} onClick={() => void run('reconnect')}>重新连接与刷新目录</button></div>
    {message && <p role="status">{message}</p>}
    <p>{data ? `${labels[data.market.status] ?? data.market.status} · 代次 ${data.market.generation} · ${data.market.subscriptions} 个源流 · ${data.market.received} 笔有效观测` : '等待后台状态'}</p>
    {data?.market.error && <p role="alert" className="error">{data.market.error}</p>}
    <small className="muted">中断 {data?.market.interruptions ?? 0} 次 · 队列溢出 {data?.market.queueOverflows ?? 0} 次。</small>
    {gaps.length > 0 && <>
      <p>最近未能核实的监控时段 {gapText(gaps[0])}</p>
      <details>
        <summary>监控缺口历史</summary>
        <ul>{gaps.map(gap => <li key={`${gap.key.market}/${gap.key.symbol}/${gap.key.priceKind}/${gap.toUtcMs}`}>{gapText(gap)}</li>)}</ul>
      </details>
    </>}
  </section>;
}
