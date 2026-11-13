import { useCallback, useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { invoke } from './host';
import { listen } from '@tauri-apps/api/event';
import { WidgetFrame } from './WidgetFrame';
import { NetworkSettings } from './NetworkSettings';

interface Package { id: string; name: string; version: string; preset: boolean; installed: boolean; description?: string; symbol?: string; defaultSize: { width: number; height: number }; minSize: { width: number; height: number } }
interface Instance { id: string; packageId: string; floating: boolean; width: number | null; height: number | null }
interface TestEvent { id: string; created: number; completed: number | null; notification: string; sound: string; viewCount: number | null }
interface Snapshot {
  packages: Package[]; instances: Instance[]; events: TestEvent[];
  autostart: boolean | null;
  diagnostics: { ticks: number; tasks: number; sessions: number; views: number; error: string | null; dataRoot: string; shortcut: string; docked: boolean };
}
const action = (action: string, instance?: string) => invoke('window_action', { action, instance });
const time = (value: number) => new Date(value).toLocaleTimeString('zh-CN', { hour12: false });
const status = (value: string) => ({ sent: '系统调用成功', suppressed: '已关闭', pending: '等待执行', sending: '递送中', unknown: '结果不确定' })[value] ?? value;

export function App() {
  const query = new URLSearchParams(location.search);
  const floating = query.get('instance');
  const trial = query.get('trial');
  const edge = query.has('edge');
  const [data, setData] = useState<Snapshot>();
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<'desk' | 'checks'>(query.get('tab') === 'checks' ? 'checks' : 'desk');
  const [shortcut, setShortcut] = useState('');
  const refresh = useCallback(async () => {
    try { setData(await invoke<Snapshot>('snapshot')); } catch (e) { setMessage(String(e)); }
  }, []);
  useEffect(() => {
    if (!isTauri() || edge || trial) return;
    void refresh();
    const unlisten = listen('host:changed', refresh);
    const unlistenNavigate = listen<string>('host:navigate', e => { if (e.payload === 'checks') setTab('checks'); });
    const timer = setInterval(refresh, 2000);
    return () => { clearInterval(timer); void unlisten.then(fn => fn()); void unlistenNavigate.then(fn => fn()); };
  }, [refresh, edge, trial]);
  const run = async (task: () => Promise<unknown>, success = '') => {
    setBusy(true); setMessage('');
    try { await task(); if (success) setMessage(success); await refresh(); } catch (e) { setMessage(`操作失败：${String(e)}`); } finally { setBusy(false); }
  };
  const move = (instance: string, direction: number) => {
    if (!data) return;
    const ids = data.instances.map(i => i.id); const index = ids.indexOf(instance); const target = index + direction;
    if (index < 0 || target < 0 || target >= ids.length) return;
    [ids[index], ids[target]] = [ids[target], ids[index]];
    void run(() => invoke('reorder_instances', { ids }));
  };
  if (edge) return <button className="edge" aria-label="展开 SoulDesk" onMouseEnter={() => void action('edge-enter')} onClick={() => void action('edge-enter')}><span /></button>;
  if (trial) return <WidgetFrame instance={trial} />;
  if (floating) return <main className="float-shell"><header><b>SOUL<span>DESK</span></b><div><button onClick={() => void run(() => action('pin', floating), '已切换置顶状态')}>切换置顶</button><button onClick={() => void run(() => action('embed', floating))}>收回工作台 ↙</button></div></header>{message && <p role="status">{message}</p>}<WidgetFrame key={data?.packages.find(p => p.id === data.instances.find(i => i.id === floating)?.packageId)?.version} instance={floating} /></main>;
  return <main className="shell" onMouseEnter={() => { if (isTauri()) void action('enter'); }} onMouseLeave={() => { if (isTauri()) void action('leave'); }}>
    <aside>
      <div className="brand"><div className="brand-mark">S</div><div>SOUL<span>DESK</span><small>个人桌面工作台</small></div></div>
      <div className="nav-label">空间 / SPACE</div>
      <button className={`nav ${tab === 'desk' ? 'selected' : ''}`} onClick={() => setTab('desk')}><span>▦</span> 我的工作台 <em>{data?.instances.length ?? '—'}</em></button>
      <button className={`nav ${tab === 'checks' ? 'selected' : ''}`} onClick={() => setTab('checks')}><span>⌁</span> 设置与验证</button>
      <div className="aside-note"><span className="tiny-label">组件 · 行情与警报</span><p>预置组件，随时添加。<br />后台持续运行。</p><div className="rule" /><span className="muted">公开行情 · 本地提醒</span></div>
      <div className="connection"><i className={data ? 'dot' : 'dot offline'} /><span>{data ? '核心后台在线' : '等待桌面宿主'}</span></div>
    </aside>
    <section className="workspace">
      <header className="topbar"><span>工作空间 <span className="slash">/</span> {tab === 'desk' ? '默认工作台' : '设置与验证'}</span><div><button onClick={() => void run(() => action('dock'))}>{data?.diagnostics.docked ? '取消停靠' : '侧边停靠'}</button><button onClick={() => void run(() => action('hide-all'))}>隐藏全部 <kbd>⌃ ⌥ Space</kbd></button></div></header>
      <div className="content">
        <div className="heading"><div><div className="eyebrow">YOUR DESKTOP, AT EASE</div><h1>{tab === 'desk' ? '留一处，给专注。' : '让每个结果有据可查。'}</h1><p>{tab === 'desk' ? '把需要的组件放在身边，余下的交给后台。' : '管理桌面行为、行情网络及后台测试；测试事件独立于价格规则。'}</p></div>{tab === 'desk' && <button className="primary" disabled={busy || !data} onClick={() => void run(() => invoke('import_package'), '导入操作完成；取消选择不会导入文件。')}>＋ 导入组件包</button>}</div>
        {!isTauri() && <div className="notice">这是网页布局预览。组件导入、托盘与后台验证需要通过 pnpm dev 启动桌面宿主。</div>}
        {message && <div role="status" className="notice">{message}</div>}
        {data?.diagnostics.error && <div role="alert" className="notice error">{data.diagnostics.error}</div>}
        {tab === 'desk' ? <>
          <div className="section-head"><h2>我的组件 <span>{data?.instances.length ?? 0}</span></h2><span>工作台与浮窗共用实例</span></div>
          <div className="widget-grid">{data?.instances.filter(i => !i.floating).map(i => {
            const p = data.packages.find(p => p.id === i.packageId); const width = i.width ?? p?.defaultSize.width ?? 400; const height = i.height ?? p?.defaultSize.height ?? 300;
            return <article className="widget-card" key={i.id} style={{ width }}><header><div><i className="dot" /> {p?.name ?? i.packageId}</div><button title="移到独立浮窗" onClick={() => void run(() => action('float', i.id))}>↗ 浮窗</button></header><div style={{ height }}><WidgetFrame key={`${i.id}:${p?.version}`} instance={i.id} /></div><footer>实例 {i.id.slice(0, 8)}<span><button disabled={busy || data.instances[0]?.id === i.id} onClick={() => move(i.id, -1)}>前移</button><button disabled={busy || data.instances.at(-1)?.id === i.id} onClick={() => move(i.id, 1)}>后移</button></span></footer>
              <details className="instance-layout"><summary>尺寸与实例</summary><form onSubmit={event => { event.preventDefault(); const form = new FormData(event.currentTarget); void run(() => invoke('manage_instance', { instance: i.id, action: 'resize', width: Number(form.get('width')), height: Number(form.get('height')) }), '尺寸已保存'); }}><label>宽度<input name="width" type="number" min={p?.minSize.width ?? 120} max="1600" defaultValue={width} required /></label><label>高度<input name="height" type="number" min={p?.minSize.height ?? 80} max="1200" defaultValue={height} required /></label><button disabled={busy}>保存</button><button type="button" disabled={busy} onClick={() => { if (window.confirm('移除此组件实例及其配置？其他实例不受影响。')) void run(() => invoke('manage_instance', { instance: i.id, action: 'remove' })); }}>移除实例</button></form></details></article>;
          })}
            {!data?.instances.some(i => !i.floating) && <div className="empty"><div className="empty-art"><span>＋</span><i /><i /></div><h2>从一个组件开始</h2><p>从下方组件库添加预置组件，<br />也可导入本地 .zip 包，每个实例独立配置。</p><button onClick={() => document.getElementById('component-library')?.scrollIntoView({ block: 'start' })}>浏览预置组件 ↓</button></div>}
          </div>
          {data?.instances.some(i => i.floating) && <p className="muted">{data.instances.filter(i => i.floating).length} 个实例正在独立浮窗中。关闭浮窗后会回到这里。</p>}
          <section className="package-list" id="component-library">
            <div className="section-head"><h2>组件库 <span>{data?.packages.length ?? 0}</span></h2><span>预置组件直接添加 · 支持导入本地包</span></div>
            {data?.packages.length ? data.packages.map(p => <div className="package" key={p.id}>
              <div className="package-symbol" aria-hidden="true">{p.symbol ?? '▦'}</div>
              <div className="package-info"><div className="package-title"><b>{p.name}</b><span className="badge">{p.preset ? '预置' : '本地导入'}</span></div>{p.description && <p>{p.description}</p>}<small>v{p.version} · {data.instances.filter(i => i.packageId === p.id).length} 个实例</small></div>
              <div className="package-actions">
                <button disabled={busy} onClick={() => void run(() => invoke('create_instance', { packageId: p.id }), `已添加${p.name}`)}>{p.installed ? '＋ 创建实例' : '＋ 添加到工作台'}</button>
                {p.installed && <button disabled={busy || data.instances.some(i => i.packageId === p.id)} title={p.preset ? '先移除该组件的实例；取消登记后仍可从预置组件库重新添加' : '先移除该组件的实例；保留本地历史版本文件'} onClick={() => void run(() => invoke('unregister_package', { packageId: p.id }), p.preset ? '已取消登记，预置组件仍可随时添加' : '已取消组件登记')}>取消登记</button>}
              </div>
            </div>) : <p className="muted">{data ? '组件库暂无可用组件。' : '连接桌面宿主后显示预置组件。'}</p>}
          </section>
        </> : <>
          <div className="metrics">{[['核心心跳', data?.diagnostics.ticks, '次'], ['待执行测试', data?.diagnostics.tasks, '个'], ['视图 / 会话', `${data?.diagnostics.views ?? 0} / ${data?.diagnostics.sessions ?? 0}`, '']].map(([label, value, unit]) => <article key={label}><small>{label}</small><strong>{value ?? '—'}<span>{unit}</span></strong></article>)}</div>
          <section className="settings-panel"><h2>快捷键与窗口</h2><div className="setting-row"><label htmlFor="shortcut">全局显隐快捷键<small>当前：{data?.diagnostics.shortcut ?? '—'}；注册失败保留旧键</small></label><input id="shortcut" placeholder="Ctrl+Alt+Space" value={shortcut} onChange={e => setShortcut(e.target.value)} /><button disabled={busy || !shortcut} onClick={() => void run(() => invoke('set_shortcut', { value: shortcut }), '快捷键已保存')}>保存</button></div><div className="setting-row"><label><input type="checkbox" disabled={busy || data?.autostart == null} checked={data?.autostart === true} onChange={event => { const enabled = event.target.checked; void run(() => invoke('set_autostart', { enabled }), enabled ? '已开启登录后后台启动' : '已关闭登录启动'); }} /> 登录后后台启动<small>{data?.autostart == null ? '无法读取系统登记状态' : '由你选择；启动时保持工作台隐藏'}</small></label></div><button onClick={() => void run(() => action('repair'), '窗口已校正到可用工作区')}>找回屏幕外窗口</button></section>
          <NetworkSettings /><section className="settings-panel"><h2>后台测试事件</h2><p>5 秒后执行。通知与声音独立开关；“系统调用成功”不代表用户一定看到或听到。</p><div className="actions"><button disabled={busy || !data} onClick={() => void run(() => invoke('schedule_test', { notification: true, sound: false, delay: 5 }), '已安排仅通知事件')}>仅通知</button><button disabled={busy || !data} onClick={() => void run(() => invoke('schedule_test', { notification: false, sound: true, delay: 5 }), '已安排仅声音事件')}>仅声音</button><button disabled={busy || !data} onClick={() => void run(async () => { await invoke('schedule_test', { notification: false, sound: false, delay: 3 }); await invoke('schedule_test', { notification: true, sound: false, delay: 6 }); await invoke('schedule_test', { notification: false, sound: true, delay: 9 }); await action('destroy'); })}>销毁全部视图并测试</button></div><p className="muted">销毁后 12 秒，用快捷键或托盘重建工作台，核对下方视图数为 0 的记录。</p>
            <div className="event-list">{data?.events.map(e => <div className="event" key={e.id}><span>{time(e.created)}<small>{e.completed ? `完成 ${time(e.completed)}` : '等待后台执行'}</small></span><span>通知：{status(e.notification)}<small>声音：{status(e.sound)}</small></span><span className="view-count">{e.viewCount ?? '—'}<small>执行时视图</small></span></div>)}</div>
          </section><p className="data-path">数据目录：{data?.diagnostics.dataRoot ?? '未连接宿主'}</p>
        </>}
      </div>
      <footer className="statusbar"><span><i className="dot" /> 本地运行 · 无账户</span><span>SOULDESK <b>0.3.3</b></span></footer>
    </section>
  </main>;
}
