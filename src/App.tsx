import { useCallback, useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { invoke } from './host';
import { WidgetFrame } from './WidgetFrame';
import { NetworkSettings } from './NetworkSettings';
import { PanelCanvas } from './PanelCanvas';
import { WidgetSurface } from './WidgetSurface';
import { Icon, type IconName } from './Icon';
import { UpdateSettings, type UpdateState } from './UpdateSettings';
import type { PanelLayout } from './panel-layout';

interface Package { id: string; name: string; version: string; preset: boolean; installed: boolean; availableVersion?: string; description?: string; symbol?: string; defaultSize: { width: number; height: number }; minSize: { width: number; height: number } }
interface Instance { id: string; packageId: string; location: 'desktop' | 'panel'; visible: boolean; pinned: boolean; width: number | null; height: number | null }
interface Snapshot {
  packages: Package[]; instances: Instance[]; autostart: boolean | null; hiddenAll: boolean; panelOpen: boolean; panelGeometry?: { effective_ratio: number }; panel: { ratio: number; tint: number; slideInMs: number; slideOutMs: number; desktopDoubleClick: boolean }; panelLayouts: Record<string, PanelLayout>; updates: UpdateState;
  diagnostics: { ticks: number; tasks: number; sessions: number; views: number; error: string | null; dataRoot: string; shortcut: string };
}
const action = (action: string, instance?: string) => invoke('window_action', { action, instance });
export function App() {
  const query = new URLSearchParams(location.search);
  const desktop = query.get('instance'), detail = query.get('detail'), trial = query.get('trial');
  const panel = query.has('panel'), edge = query.has('edge');
  const [data, setData] = useState<Snapshot>();
  const [message, setMessage] = useState(''), [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<'library' | 'instances' | 'checks'>(query.get('tab') === 'checks' ? 'checks' : 'library');
  const [shortcut, setShortcut] = useState('');
  const [ratio, setRatio] = useState(40), [tint, setTint] = useState(12), [menu, setMenu] = useState(false);
  const [slideIn, setSlideIn] = useState(250), [slideOut, setSlideOut] = useState(250), [doubleClick, setDoubleClick] = useState(true);
  const [effective, setEffective] = useState<number>();
  const [detailPage, setDetailPage] = useState(query.get('page') ?? 'quotes');
  const refresh = useCallback(async () => {
    try { setData(await invoke<Snapshot>('snapshot')); } catch (e) { setMessage(String(e)); }
  }, []);
  useEffect(() => {
    document.documentElement.dataset.surface = edge ? 'edge' : panel ? 'panel' : desktop ? 'desktop' : detail ? 'detail' : 'management';
    if (!isTauri() || edge || trial) return;
    void refresh();
    const off = listen('host:changed', refresh);
    const nav = listen<string>('host:navigate', e => { if (e.payload === 'checks') setTab('checks'); });
    const geometry = listen<{ effective_ratio: number }>('host:panel-geometry', e => setEffective(e.payload.effective_ratio));
    const offPage = listen<string>('host:detail-page', e => setDetailPage(e.payload));
    const timer = setInterval(refresh, 3000);
    return () => { clearInterval(timer); for (const item of [off, nav, geometry, offPage]) void item.then(fn => fn()); };
  }, [refresh, edge, trial, panel, desktop, detail]);
  useEffect(() => { if (data?.panel) { setRatio(data.panel.ratio * 100); setTint(data.panel.tint * 100); } }, [data?.panel.ratio, data?.panel.tint]);
  useEffect(() => { if (data?.panel) { setSlideIn(data.panel.slideInMs); setSlideOut(data.panel.slideOutMs); setDoubleClick(data.panel.desktopDoubleClick); } }, [data?.panel.slideInMs, data?.panel.slideOutMs, data?.panel.desktopDoubleClick]);
  useEffect(() => {
    if (!panel || !menu) return;
    void action('menu-open').catch(e => setMessage(String(e)));
    return () => { void action('menu-close').catch(() => {}); };
  }, [panel, menu]);
  const run = async (task: () => Promise<unknown>, success = '') => {
    setBusy(true); setMessage('');
    try { await task(); setMessage(success); await refresh(); } catch (e) { setMessage(`操作失败：${String(e)}`); } finally { setBusy(false); }
  };
  const version = (id: string) => data?.packages.find(p => p.id === data.instances.find(i => i.id === id)?.packageId)?.version;
  useEffect(() => { if (data?.panelGeometry) setEffective(data.panelGeometry.effective_ratio); }, [data?.panelGeometry?.effective_ratio]);
  const savePanel = () => run(() => invoke('set_panel', { ratio: ratio / 100, tint: tint / 100, slideInMs: slideIn, slideOutMs: slideOut, desktopDoubleClick: doubleClick }), '面板偏好已保存');
  const panelControls = <form className="panel-preferences" onSubmit={e => { e.preventDefault(); void savePanel(); }}>
    <label>宽度比例 <output>{ratio.toFixed(0)}%</output><input aria-label="面板宽度比例" type="range" min="1" max="100" value={ratio} onChange={e => setRatio(Number(e.target.value))} /></label>
    <label>背景遮罩 <output>{tint.toFixed(0)}%</output><input aria-label="面板背景遮罩" type="range" min="0" max="100" value={tint} onChange={e => setTint(Number(e.target.value))} /></label>
    <label>滑入 <output>{slideIn ? `${slideIn}毫秒` : '无动效'}</output><input aria-label="滑入时长" type="range" min="0" max="1000" step="10" value={slideIn} onChange={e => setSlideIn(Number(e.target.value))} /></label>
    <label>滑出 <output>{slideOut ? `${slideOut}毫秒` : '无动效'}</output><input aria-label="滑出时长" type="range" min="0" max="1000" step="10" value={slideOut} onChange={e => setSlideOut(Number(e.target.value))} /></label>
    <label className="panel-double-click"><input type="checkbox" checked={doubleClick} onChange={e => setDoubleClick(e.target.checked)} />双击桌面空白处开关面板</label>
    <button disabled={busy}>保存</button>{effective && <small>实际宽度 {(effective * 100).toFixed(0)}% · 遵守组件最小尺寸</small>}
  </form>;
  const notice = message && <p className="notice" role="status">{message}</p>;
  const controls = (id: string, inPanel: boolean): { label: string; icon: IconName; disabled?: boolean; action: () => void }[] => [
    { label: inPanel ? '放回桌面' : '移入面板', icon: inPanel ? 'desktop' : 'panel', disabled: busy, action: () => void run(() => action(inPanel ? 'float' : 'embed', id)) },
    ...(!inPanel ? [{ label: data?.instances.find(i => i.id === id)?.pinned ? '取消置顶' : '置顶', icon: 'pin' as const, disabled: busy, action: () => void run(() => action('pin', id)) }] : []),
    ...(data?.instances.find(i => i.id === id)?.packageId === 'local.souldesk.market' ? [{ label: '自选与警报', icon: 'detail' as const, disabled: busy, action: () => void run(() => action('detail', id)) }] : []),
    { label: '隐藏组件', icon: 'hide', disabled: busy, action: () => void run(() => action('hide', id)) },
    { label: '管理组件', icon: 'settings', action: () => void run(() => action('show')) },
  ];
  if (edge) return <button className="edge" aria-label="展开组件面板" onMouseEnter={() => void action('edge-enter')} onClick={() => void action('edge-enter')}><span /></button>;
  if (trial) return <WidgetFrame instance={trial} />;
  if (detail) return <main className="detail-shell"><WidgetFrame key={version(detail)} instance={detail} page={detailPage} /></main>;
  if (desktop) return <main className="desktop-shell">
    <WidgetSurface instance={desktop} items={controls(desktop, false)} notice={notice} onError={setMessage} onDrag={e => { if (e.button === 0) void invoke('surface_pointer', { resize: false }).catch(e => setMessage(String(e))); }} onResize={e => { if (e.button === 0) void invoke('surface_pointer', { resize: true }).catch(e => setMessage(String(e))); }}>
    <WidgetFrame key={version(desktop)} instance={desktop} />
    </WidgetSurface>
  </main>;
  if (panel) return <main className="panel-shell" style={{ '--panel-tint': data?.panel.tint ?? .12 } as React.CSSProperties} onFocusCapture={e => void action(e.target instanceof HTMLElement && e.target.matches('input,textarea,select,[contenteditable=true]') ? 'editing' : 'idle')} onBlurCapture={() => void action('idle')} onMouseEnter={() => void action('enter')} onMouseLeave={() => void action('leave')}>
    <nav className="panel-tools" aria-label="面板操作"><button className="icon-button" title="添加与管理组件" aria-label="添加与管理组件" onClick={() => void run(() => action('show'))}><Icon name="plus" /></button><button className="icon-button" title="面板设置" aria-label="面板设置" aria-expanded={menu} onClick={() => setMenu(!menu)}><Icon name="settings" /></button><button className="icon-button" title="收起面板" aria-label="收起面板" onClick={() => void run(() => action('collapse'))}><Icon name="collapse" /></button></nav>
    {menu && <aside className="panel-settings-popover">{panelControls}<button className="icon-button" title="关闭设置" aria-label="关闭设置" onClick={() => setMenu(false)}><Icon name="close" /></button></aside>}
    {notice && <div className="panel-notice">{notice}</div>}
    {data && data.instances.some(i => i.visible && i.location === 'panel') ? <PanelCanvas instances={data.instances.filter(i => i.visible && i.location === 'panel')} packages={data.packages} saved={data.panelLayouts} version={version} items={id => controls(id, true)} onError={setMessage} /> : <div className="empty panel-empty"><button className="icon-button" title="添加组件" aria-label="添加组件" onClick={() => void action('show')}><Icon name="plus" width="28" height="28" /></button><p>从组件菜单移入面板</p></div>}
  </main>;
  return <main className="management-shell">
    <aside><div className="brand">SoulDesk<small>桌面组件</small></div><nav aria-label="管理导航">{([['library', '组件库', '＋'], ['instances', '已添加', '▦'], ['checks', '设置', '⚙']] as const).map(([id, label, icon]) => <button key={id} className={tab === id ? 'selected' : ''} onClick={() => setTab(id)}><span>{icon}</span>{label}{id === 'instances' && <small>{data?.instances.length ?? 0}</small>}</button>)}</nav><div className="aside-note">公开行情 · 本地提醒<br /><span className="muted">关闭管理窗口后，组件与后台继续运行。</span></div><span className="local-status"><i className={data ? 'dot' : 'dot offline'} />{data ? '核心后台在线' : '等待桌面宿主'}</span></aside>
    <section className="management-content"><header className="topbar"><span>{tab === 'library' ? '组件库' : tab === 'instances' ? '已添加的组件' : '应用设置'}</span><div><button onClick={() => void run(() => action('toggle'))}>组件面板 <kbd>Ctrl Alt Space</kbd></button><button onClick={() => void run(() => action(data?.hiddenAll ? 'restore-all' : 'hide-all'))}>{data?.hiddenAll ? '恢复组件显示' : '隐藏全部'}</button></div></header>
    <div className="content">{!isTauri() && <p className="notice">网页仅供布局预览，桌面操作需在 SoulDesk 中运行。</p>}{notice}{data?.diagnostics.error && <p role="alert" className="notice error">{data.diagnostics.error}</p>}
      {data?.hiddenAll && <div className="notice">所有组件当前隐藏。请先恢复显示，再添加或显示单个组件。<button onClick={() => void run(() => action('restore-all'))}>恢复组件显示</button></div>}
      {tab === 'library' ? <><div className="heading"><div><small>放在桌面，随时看见</small><h1>从一个组件开始</h1><p>添加后直接出现在桌面。同一种组件可添加多份，各自保存配置。</p></div><button disabled={busy || !data} onClick={() => void run(() => invoke('import_package'), '导入操作结束；已登记的兼容版本可在已添加中恢复。')}>导入本地 ZIP</button></div>
      <div className="library-grid">{data?.packages.map(p => <article className="package" key={p.id}><div className="package-symbol" data-kind={p.id}>{p.symbol ?? '▦'}</div><span className="badge">{p.preset ? '预置组件' : '本地组件'} · {p.version}</span><h2>{p.name}</h2><p>{p.description ?? '独立本地组件，通过受限宿主接口运行。'}</p><div className="package-actions"><button className="primary" disabled={busy || data.hiddenAll} onClick={() => void run(() => invoke('create_instance', { packageId: p.id }), '已添加到桌面')}>＋ 添加到桌面</button><small>{data.instances.filter(i => i.packageId === p.id).length} 个实例</small></div>{p.installed && p.preset && p.availableVersion !== p.version && <button disabled={busy} onClick={() => void run(() => invoke('update_preset', { packageId: p.id }), '预置包已升级；配置和显式保存的尺寸保留')}>升级至 {p.availableVersion}</button>}</article>)}</div>
      <p className="muted">本地组件按你信任的代码运行。无需账户或密钥，组件包可独立导入。</p></> : tab === 'instances' ? <><div className="heading"><div><small>桌面与面板，互不牵连</small><h1>已添加的组件</h1><p>关闭组件只隐藏视图；删除实例会移除它的配置与自选，价格警报独立保留。</p></div></div>
      {!data?.instances.length && <div className="empty"><h2>桌面还很安静</h2><p>到组件库添加第一个组件。</p><button onClick={() => setTab('library')}>浏览组件库</button></div>}
      {data?.instances.map(i => { const p = data.packages.find(p => p.id === i.packageId); return <article className="instance-row" key={i.id}><div className="instance-title"><h2>{p?.name ?? i.packageId}</h2><span className="badge">{i.location === 'desktop' ? '桌面' : '面板'} · {i.visible ? '已显示' : '已隐藏'}{i.pinned ? ' · 置顶' : ''}</span><small>实例 {i.id.slice(0, 8)} · v{p?.version}</small></div><div className="actions"><button disabled={busy || data.hiddenAll} onClick={() => void run(() => action(i.visible ? 'hide' : 'restore', i.id))}>{i.visible ? '隐藏' : '显示'}</button><button disabled={busy || data.hiddenAll} onClick={() => void run(() => action(i.location === 'desktop' ? 'embed' : 'float', i.id))}>{i.location === 'desktop' ? '移入面板' : '放回桌面'}</button>{i.visible && i.location === 'desktop' && <button disabled={busy || data.hiddenAll} onClick={() => void run(() => action('pin', i.id))}>{i.pinned ? '取消置顶' : '置顶'}</button>}{p?.id === 'local.souldesk.market' && <button disabled={busy || data.hiddenAll || !i.visible} onClick={() => void run(() => action('detail', i.id))}>自选与警报</button>}</div>
      <details><summary>尺寸、排序与删除</summary><form className="size-form" key={`${i.id}-${i.location}-${data.panelLayouts[i.id]?.width}-${data.panelLayouts[i.id]?.height}-${i.width}-${i.height}`} onSubmit={e => { e.preventDefault(); const f = new FormData(e.currentTarget); void run(() => invoke('manage_instance', { instance: i.id, action: 'resize', width: Number(f.get('width')), height: Number(f.get('height')) }), '尺寸已保存'); }}><label>宽度<input name="width" type="number" min={p?.minSize.width} max={i.location === "panel" ? 16384 : 1600} defaultValue={(i.location === "panel" ? data.panelLayouts[i.id]?.width : undefined) ?? i.width ?? p?.defaultSize.width} required /></label><label>高度<input name="height" type="number" min={p?.minSize.height} max={i.location === "panel" ? 16384 : 1200} defaultValue={(i.location === "panel" ? data.panelLayouts[i.id]?.height : undefined) ?? i.height ?? p?.defaultSize.height} required /></label><button disabled={busy}>保存尺寸</button><button type="button" disabled={busy || data.instances[0]?.id === i.id} onClick={() => { const ids = data.instances.map(i => i.id), at = ids.indexOf(i.id); [ids[at - 1], ids[at]] = [ids[at], ids[at - 1]]; void run(() => invoke('reorder_instances', { ids })); }}>列表前移</button><button className="danger" type="button" disabled={busy} onClick={() => { if (window.confirm('删除此实例及其配置和自选？已有价格警报与历史会保留，请在警报管理中另行暂停或删除。')) void run(() => invoke('manage_instance', { instance: i.id, action: 'remove' })); }}>删除实例</button></form></details></article>; })}</> : <>
      <div className="heading"><div><small>按你的习惯运行</small><h1>应用设置</h1><p>组件启动时恢复上次的桌面位置，面板与管理窗口保持关闭。</p></div></div>
      <section className="settings-panel"><h2>组件面板</h2><p>鼠标移出立即收起；双击、快捷键打开后保持展开。时长为0表示无动效。</p>{panelControls}</section>
      <section className="settings-panel"><h2>快捷键与启动</h2><div className="setting-row"><label htmlFor="shortcut">开关辅助面板<small>当前：{data?.diagnostics.shortcut ?? '—'}，冲突时保留原键</small></label><input id="shortcut" placeholder="Ctrl+Alt+Space" value={shortcut} onChange={e => setShortcut(e.target.value)} /><button disabled={busy || !shortcut} onClick={() => void run(() => invoke('set_shortcut', { value: shortcut }), '快捷键已保存')}>保存</button></div><label className="check"><input type="checkbox" disabled={busy || data?.autostart == null} checked={data?.autostart === true} onChange={e => { const enabled = e.target.checked; void run(() => invoke('set_autostart', { enabled }), '登录启动设置已保存'); }} />登录后后台启动</label><button onClick={() => void run(() => action('repair'), '已校正可达位置')}>找回屏幕外组件</button></section>
      <UpdateSettings state={data?.updates} refresh={refresh} /><NetworkSettings /><section className="settings-panel"><h2>本地数据与诊断</h2><p>数据目录：{data?.diagnostics.dataRoot ?? '未连接'}</p><p className="muted">{data?.diagnostics.views ?? 0} 个宿主窗口 · {data?.diagnostics.sessions ?? 0} 个组件会话。隐藏组件释放展示，自选与有效规则继续运行。</p><div className="actions"><button disabled={busy} onClick={() => void run(() => invoke('schedule_test', { notification: true, sound: false, delay: 3 }), '已安排通知测试')}>测试通知</button><button disabled={busy} onClick={() => void run(() => invoke('schedule_test', { notification: false, sound: true, delay: 3 }), '已安排声音测试')}>测试声音</button></div></section>
      </>}
    </div><footer className="statusbar"><span><i className="dot" /> 本地运行 · 组件隐藏不停止有效规则</span><span>SoulDesk {data?.updates.currentVersion ?? '0.5.0'}</span></footer></section>
  </main>;
}
