import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import appPackage from '../package.json' with { type: 'json' };
import { invoke } from './host';
import { OfficialGallery } from './OfficialGallery';
import { WidgetFrame } from './WidgetFrame';
import { NetworkSettings } from './NetworkSettings';
import { PanelCanvas, type PanelScroll } from './PanelCanvas';
import { defaultEdge, edgeStyles, EdgeStylePicker, PanelEdge, readEdgeStyle, type EdgeStyle, type EdgeVisual } from './PanelEdge';
import { PanelGroups, MovePanelGroupDialog, ungrouped, type PanelGroupsState, type GroupChange } from './PanelGroups';
import { WidgetSurface, type SurfaceAction } from './WidgetSurface';
import { Icon } from './Icon';
import { UpdateSettings, type UpdateState } from './UpdateSettings';
import type { PanelLayout } from './panel-layout';

interface Package { id: string; name: string; version: string; digest?: string; capabilities?: string[]; defaultSize: { width: number; height: number }; minSize: { width: number; height: number } }
interface Instance { id: string; packageId: string; location: 'desktop' | 'panel'; visible: boolean; pinned: boolean; width: number | null; height: number | null }
const appThemes = [
  { id: 'ink', name: '墨蓝', tone: '深色' },
  { id: 'pine', name: '松烟', tone: '深色' },
  { id: 'oat', name: '燕麦', tone: '浅色' },
  { id: 'iris', name: '雾紫', tone: '浅色' },
] as const;
type AppTheme = (typeof appThemes)[number]['id'];
type PanelChanges = Partial<Pick<Snapshot['panel'], 'tint' | 'slideInMs' | 'slideOutMs' | 'desktopDoubleClick' | 'edgeStyle'>>;
interface Snapshot {
  appTheme?: AppTheme;
  panelGroups: PanelGroupsState;
  widgetBackdrop: boolean;
  panelScroll?: PanelScroll; panelTarget?: string | null;
  packages: Package[]; instances: Instance[]; autostart: boolean | null; hiddenAll: boolean; panelOpen: boolean; panelLocked: boolean; panelLowered?: boolean; panelShortcutEnabled?: boolean; panelGeometry?: { effective_ratio: number }; panel: { ratio: number; tint: number; slideInMs: number; slideOutMs: number; desktopDoubleClick: boolean; edge: EdgeVisual['position']; edgeStyle: EdgeStyle }; panelLayouts: Record<string, PanelLayout>; updates: UpdateState;
  panelShortcutRegistered?: boolean;
  diagnostics: { ticks: number; tasks: number; sessions: number; views: number; error: string | null; dataRoot: string; shortcut: string };
}
const action = (action: string, instance?: string) => invoke('window_action', { action, instance });
function RemoveInstanceDialog({ name, busy, message, onCancel, onConfirm }: { name: string; busy: boolean; message: string; onCancel: () => void; onConfirm: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => { const node = dialog.current!; node.showModal(); return () => node.close(); }, []);
  return <dialog ref={dialog} className="remove-instance-dialog" aria-label="删除实例确认" onCancel={event => { event.preventDefault(); if (!busy) onCancel(); }}>
    <form onSubmit={event => { event.preventDefault(); if (!busy) onConfirm(); }}>
      <h2>删除“{name}”的此实例？</h2>
      <p>实例配置、保存内容及自选将一并移除，无法撤销。已有价格警报和历史保留。</p>
      {message && <p role="alert" className="error">{message}</p>}
      <div className="actions"><button type="button" data-action="cancel-remove" autoFocus disabled={busy} onClick={onCancel}>取消</button><button type="submit" data-action="confirm-remove" className="danger" disabled={busy}>{busy ? '正在删除…' : '确认删除'}</button></div>
    </form>
  </dialog>;
}
export function App() {
  const query = new URLSearchParams(location.search);
  const desktop = query.get('instance'), detail = query.get('detail'), trial = query.get('trial');
  const panel = query.has('panel'), edge = query.has('edge');
  const panelView = panel ? query.get('view') : null;
  const [panelReady, setPanelReady] = useState(false);
  const [data, setData] = useState<Snapshot>();
  const appTheme = data?.appTheme ?? appThemes.find(theme => theme.id === window.__SOULDESK_APP_THEME__)?.id ?? 'ink';
  const [message, setMessage] = useState(''), [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<string>();
  const [movingGroup, setMovingGroup] = useState<string>();
  const [focusedPackage, setFocusedPackage] = useState<string>();
  const [tab, setTab] = useState<'gallery' | 'library' | 'instances' | 'checks'>(query.get('tab') === 'checks' ? 'checks' : 'library');
  const [shortcut, setShortcut] = useState<string>();
  const [menu, setMenu] = useState(false);
  const [resizingPanel, setResizingPanel] = useState(false);
  const [panelDraft, setPanelDraft] = useState<PanelChanges>({});
  const queuedPanelChanges = useRef<PanelChanges | undefined>(undefined);
  const savingPanel = useRef(false);
  const snapshotRequest = useRef(0);
  const [edgeVisual, setEdgeVisual] = useState<EdgeVisual>();
  const [effective, setEffective] = useState<number>();
  const [detailPage, setDetailPage] = useState(query.get('page') ?? 'quotes');
  const shortcutEnabled = data?.panelShortcutEnabled ?? true;
  const configuredShortcut = data?.diagnostics.shortcut;
  const shortcutInput = shortcut ?? configuredShortcut ?? '';
  const shortcutUnavailable = shortcutEnabled && data?.panelShortcutRegistered === false;
  const refresh = useCallback(async () => {
    const request = ++snapshotRequest.current;
    try {
      const snapshot = await invoke<Snapshot>('snapshot');
      if (request === snapshotRequest.current) setData(snapshot);
    } catch (e) { if (request === snapshotRequest.current) setMessage(String(e)); }
  }, []);
  const hasPanelData = panel && !!data;
  useLayoutEffect(() => {
    if (!panelView || !hasPanelData) return;
    let current = true;
    void invoke('panel_view', { viewId: panelView, update: { event: 'ready' } }).then(() => {
      if (current) setPanelReady(true);
    }).catch(error => { if (current) setMessage(String(error)); });
    return () => { current = false; };
  }, [panelView, hasPanelData]);
  const savePanelScroll = useCallback((group: string | null, position: PanelScroll) => {
    if (panelView) void invoke('panel_view', { viewId: panelView, update: { event: 'scroll', group, position } })
      .catch(error => setMessage(String(error)));
  }, [panelView]);
  useLayoutEffect(() => {
    document.documentElement.dataset.surface = edge ? 'edge' : panel ? 'panel' : desktop ? 'desktop' : detail ? 'detail' : 'management';
  }, [edge, panel, desktop, detail]);
  useLayoutEffect(() => {
    if (edge || detail || trial) return;
    document.documentElement.dataset.appTheme = appTheme;
    return () => { delete document.documentElement.dataset.appTheme; };
  }, [appTheme, edge, detail, trial]);
  useEffect(() => {
    if (!isTauri() || edge || trial) return;
    void refresh();
    const off = listen('host:changed', refresh);
    const nav = listen<string>('host:navigate', e => { if (e.payload === 'checks') setTab('checks'); });
    const geometry = listen<{ effective_ratio: number }>('host:panel-geometry', e => setEffective(e.payload.effective_ratio));
    const offPage = listen<string>('host:detail-page', e => setDetailPage(e.payload));
    const timer = setInterval(refresh, 3000);
    return () => { clearInterval(timer); for (const item of [off, nav, geometry, offPage]) void item.then(fn => fn()); };
  }, [refresh, edge, trial]);
  useEffect(() => {
    if (!isTauri() || !(panel || edge)) return;
    const off = listen<EdgeVisual>('host:panel-edge', e => setEdgeVisual(e.payload));
    return () => { void off.then(unlisten => unlisten()); };
  }, [edge, panel]);
  const run = async (task: () => Promise<unknown>, success = '') => {
    setBusy(true); setMessage('');
    try { await task(); setMessage(success); await refresh(); return true; } catch (e) { setMessage(`操作失败：${String(e)}`); return false; } finally { setBusy(false); }
  };
  const managePackageInstances = (packageId: string) => {
    setFocusedPackage(packageId);
    setTab('instances');
  };
  const unregister = (packageId: string) =>
    void run(() => invoke('unregister_package', { packageId }), '已卸载组件');
  const version = (id: string) => data?.packages.find(p => p.id === data.instances.find(i => i.id === id)?.packageId)?.version;
  const groups = data?.panelGroups ?? ungrouped;
  const panelInstances = data?.instances.filter(i => i.visible && i.location === 'panel' && (groups.membership[i.id] ?? null) === groups.active) ?? [];
  const changeGroup = (change: GroupChange) => run(() => invoke('manage_panel_group', { change }));
  const moveGroupDialog = movingGroup && <MovePanelGroupDialog key={movingGroup} instance={movingGroup} name={data?.packages.find(p => p.id === data.instances.find(i => i.id === movingGroup)?.packageId)?.name ?? '组件'} state={groups} busy={busy} message={message.startsWith('操作失败：') ? message : ''} onChange={changeGroup} onClose={() => setMovingGroup(undefined)} />;
  useEffect(() => { if (data?.panelGeometry) setEffective(data.panelGeometry.effective_ratio); }, [data?.panelGeometry?.effective_ratio]);
  const widthRatio = (data?.panelOpen ? effective : undefined) ?? data?.panel.ratio ?? .4;
  const panelLocked = data?.panelLocked ?? false;
  const widgetBackdrop = data?.widgetBackdrop === true;
  const resizePanel = async () => {
    if (busy || resizingPanel) return;
    setResizingPanel(true);
    try { await run(() => action('resize-panel')); }
    finally { setResizingPanel(false); }
  };
  const activeEdge = edgeVisual ?? {
    style: edge ? readEdgeStyle(query.get('style')) : data?.panel.edgeStyle ?? defaultEdge.style,
    position: edge ? { ...defaultEdge.position, side: query.get('side') === 'left' ? 'left' as const : 'right' as const } : data?.panel.edge ?? defaultEdge.position,
  };
  const edgeOnLeft = activeEdge.position.side === 'left';
  const edgeSize = edgeStyles[activeEdge.style];
  const preferences = { ...data?.panel, ...panelDraft };
  const tint = preferences.tint ?? .5;
  const { slideInMs: slideIn = 100, slideOutMs: slideOut = 100, desktopDoubleClick: doubleClick = true, edgeStyle = 'classic' } = preferences;
  const updatePanel = async (change: PanelChanges) => {
    setPanelDraft(previous => ({ ...previous, ...change }));
    queuedPanelChanges.current = { ...queuedPanelChanges.current, ...change };
    if (savingPanel.current) return;
    savingPanel.current = true;
    setMessage('');
    try {
      do {
        while (queuedPanelChanges.current) {
          const next = queuedPanelChanges.current;
          queuedPanelChanges.current = undefined;
          try { await invoke('set_panel', next); }
          catch (error) { setMessage(`保存面板偏好失败：${String(error)}`); }
        }
        await refresh();
      } while (queuedPanelChanges.current);
    } finally {
      setPanelDraft({});
      savingPanel.current = false;
    }
  };
  const panelControls = <fieldset className="panel-preferences" disabled={!data} aria-label="面板偏好">
    <p className="panel-width-hint">面板宽度 <output>{(widthRatio * 100).toFixed(0)}%</output> · 拖动朝屏幕内的边沿调整，松开自动保存。</p>
    <label className="panel-tint">背景遮罩 <output>{Math.round(tint * 100)}%</output><input aria-label="面板背景遮罩" type="range" min="0" max="100" value={Math.round(tint * 100)} onChange={e => void updatePanel({ tint: e.target.valueAsNumber / 100 })} /></label>
    <div className="panel-animation-row">
      <label>滑入 <output>{slideIn ? `${slideIn}毫秒` : '无动效'}</output><input aria-label="滑入时长" type="range" min="0" max="1000" step="10" value={slideIn} onChange={e => void updatePanel({ slideInMs: e.target.valueAsNumber })} /></label>
      <label>滑出 <output>{slideOut ? `${slideOut}毫秒` : '无动效'}</output><input aria-label="滑出时长" type="range" min="0" max="1000" step="10" value={slideOut} onChange={e => void updatePanel({ slideOutMs: e.target.valueAsNumber })} /></label>
    </div>
    <label className="panel-double-click"><input type="checkbox" checked={doubleClick} onChange={e => void updatePanel({ desktopDoubleClick: e.target.checked })} />双击桌面空白处开关面板</label>
    <EdgeStylePicker value={edgeStyle} onChange={style => void updatePanel({ edgeStyle: style })} />
    <small>面板宽度遵守组件最小尺寸，拖动时按 Esc 可取消。</small>
  </fieldset>;
  const notice = message && <p className="notice" role="status">{message}</p>;
  const requestRemove = (id: string) => { setMessage(''); setRemoving(id); };
  const removeDialog = removing && <RemoveInstanceDialog name={data?.packages.find(p => p.id === data.instances.find(i => i.id === removing)?.packageId)?.name ?? '组件'} busy={busy} message={message} onCancel={() => setRemoving(undefined)} onConfirm={() => void run(() => action('remove', removing), '实例已删除').then(removed => { if (removed) setRemoving(undefined); })} />;
  const controls = (id: string, inPanel: boolean): SurfaceAction[] => [
    { label: inPanel ? '放回桌面' : '移入面板', icon: inPanel ? 'desktop' : 'panel', disabled: busy, action: () => void run(() => action(inPanel ? 'float' : 'embed', id)) },
    ...(!inPanel ? [{ label: data?.instances.find(i => i.id === id)?.pinned ? '取消置顶' : '置顶', icon: 'pin' as const, disabled: busy, action: () => void run(() => action('pin', id)) }] : []),
    { label: '隐藏组件', icon: 'eye', disabled: busy, action: () => void run(() => action('hide', id)) },
    ...(inPanel ? [{ label: '移动到分组', icon: 'moveToGroup' as const, placement: 'always' as const, disabled: busy, action: () => { setMessage(''); setMovingGroup(id); } }] : []),
    { label: '删除实例', icon: 'trash', placement: 'always', disabled: busy, action: () => requestRemove(id) },
    ...(data?.instances.find(i => i.id === id)?.packageId === 'local.souldesk.market' ? [{ label: '自选与警报', icon: 'detail' as const, placement: 'more' as const, disabled: busy, action: () => void run(() => action('detail', id)) }] : []),
  ];
  const panelToggle = (expanded: boolean) => <PanelEdge visual={activeEdge} expanded={expanded} locked={panelLocked} onError={setMessage} onHover={() => void action('edge-hover').catch(e => setMessage(String(e)))} onActivate={() => void action('edge-activate').catch(e => setMessage(String(e)))} />;
  if (edge) return <>{panelToggle(false)}{message && <span className="edge-error" role="alert" title={message}>!</span>}</>;
  if (trial) return <WidgetFrame instance={trial} />;
  if (detail) return <main className="detail-shell"><WidgetFrame key={version(detail)} instance={detail} page={detailPage} /></main>;
  if (desktop) return <main className="desktop-shell" data-widget-backdrop={widgetBackdrop || undefined}>
    <WidgetSurface instance={desktop} items={controls(desktop, false)} notice={notice} onError={setMessage} onDrag={e => { if (e.button === 0) void invoke('surface_pointer', { resize: false }).catch(e => setMessage(String(e))); }} onResize={e => { if (e.button === 0) void invoke('surface_pointer', { resize: true }).catch(e => setMessage(String(e))); }}>
    <WidgetFrame key={version(desktop)} instance={desktop} />
    </WidgetSurface>
    {removeDialog}
  </main>;
  if (panel) return <main data-edge-side={activeEdge.position.side} data-widget-backdrop={widgetBackdrop || undefined} className={`panel-shell${resizingPanel ? ' resizing-width' : ''}`} style={{ '--panel-tint': tint, '--edge-width': `${edgeSize.width}px`, '--edge-height': `${edgeSize.height}px`, '--edge-offset': activeEdge.position.offset } as React.CSSProperties}>
    <button className="panel-width-handle" role="separator" aria-orientation="vertical" aria-label="调整面板宽度" aria-valuemin={1} aria-valuemax={100} aria-valuenow={Math.round(widthRatio * 100)} title="拖动调整面板宽度；左右方向键微调" disabled={busy || !data} onPointerDown={e => { if (e.button === 0) { e.preventDefault(); void resizePanel(); } }} onKeyDown={e => { if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); if (!busy) void run(() => invoke('set_panel', { ratio: Math.max(.01, Math.min(1, widthRatio + ((e.key === 'ArrowLeft') !== edgeOnLeft ? .02 : -.02))) })); } }} />
    {resizingPanel && <output className="panel-width-output">{(widthRatio * 100).toFixed(0)}%</output>}
    <header className="panel-topbar">
      {data && <PanelGroups state={groups} instances={data.instances} busy={busy} message={message.startsWith('操作失败：') ? message : ''} onChange={changeGroup} />}
      <nav className="panel-tools" aria-label="面板操作">
        <button className="icon-button" title="管理组件" aria-label="管理组件" onClick={() => void run(() => action('show'))}><Icon name="grid" /></button>
        <button className="icon-button" title="面板设置" aria-label="面板设置" aria-expanded={menu} onClick={() => setMenu(!menu)}><Icon name="sliders" /></button>
        <button className="icon-button" title={panelLocked ? '解锁面板' : '锁定面板'} aria-label="锁定面板" aria-pressed={panelLocked} disabled={busy || !data} onClick={() => void run(() => action(panelLocked ? 'unlock-panel' : 'lock-panel'))}>
          <Icon name={panelLocked ? 'pin' : 'pinOff'} fill={panelLocked ? 'currentColor' : 'none'} />
        </button>
      </nav>
    </header>
    {!data?.panelLowered && panelToggle(true)}{removeDialog}{moveGroupDialog}
    {menu && <aside className="panel-settings-popover">{panelControls}<button className="icon-button" title="关闭设置" aria-label="关闭设置" onClick={() => setMenu(false)}><Icon name="close" /></button></aside>}
    {notice && <div className="panel-notice">{notice}</div>}
    <section id="panel-group-content" className="panel-group-content" role="tabpanel" aria-label={groups.groups.find(g => g.id === groups.active)?.name ?? '未分组'}>
      {!data ? <div className="empty panel-empty" role="status">正在加载面板…</div> : panelInstances.length ? <PanelCanvas key={groups.active ?? 'ungrouped'} group={groups.active} instances={panelInstances} packages={data.packages} saved={data.panelLayouts} version={version} items={id => controls(id, true)} onError={setMessage} position={data.panelScroll} onScrollPosition={savePanelScroll} enabled={!panelView || panelReady} target={data.panelTarget} /> : <div className="empty panel-empty"><button className="icon-button" title="添加组件" aria-label="添加组件" onClick={() => void action('show')}><Icon name="plus" width="28" height="28" /></button><p>{groups.active ? '这个分组还没有可见组件' : '从组件库添加到面板'}<br />可用组件工具栏的“移动到分组”按钮调整归属</p></div>}
    </section>
  </main>;
  return <main className="management-shell" onPointerDownCapture={e => {
    if (isTauri() && e.isPrimary && e.button === 0 && !(e.target instanceof Element && e.target.closest('[data-panel-toggle]'))) {
      void action('outside-panel').catch(error => setMessage(String(error)));
    }
  }}>
    <aside><div className="brand">SoulDesk<small>桌面组件</small></div><nav aria-label="管理导航">{([['gallery', '官方广场', '◇'], ['library', '组件库', '＋'], ['instances', '已添加', '▦'], ['checks', '设置', '⚙']] as const).map(([id, label, icon]) => <button key={id} className={tab === id ? 'selected' : ''} onClick={() => { setFocusedPackage(undefined); setTab(id); }}><span>{icon}</span>{label}{id === 'instances' && <small>{data?.instances.length ?? 0}</small>}</button>)}</nav><div className="aside-note">公开行情 · 本地提醒<br /><span className="muted">关闭管理窗口后，组件与后台继续运行。</span></div><span className="local-status"><i className={data ? 'dot' : 'dot offline'} />{data ? '核心后台在线' : '等待桌面宿主'}</span></aside>
    <section className="management-content"><header className="topbar"><span>{tab === 'gallery' ? '官方广场' : tab === 'library' ? '组件库' : tab === 'instances' ? '已添加的组件' : '应用设置'}</span><div><button data-panel-toggle onClick={() => void run(() => action('toggle'))}>组件面板{shortcutEnabled && data?.panelShortcutRegistered && <kbd>{configuredShortcut}</kbd>}</button><button onClick={() => void run(() => action(data?.hiddenAll ? 'restore-all' : 'hide-all'))}>{data?.hiddenAll ? '恢复组件显示' : '隐藏全部'}</button></div></header>
    <div className="content">{!isTauri() && <p className="notice">网页仅供布局预览，桌面操作需在 SoulDesk 中运行。</p>}{notice}{data?.diagnostics.error && <p role="alert" className="notice error">{data.diagnostics.error}</p>}
      {shortcutUnavailable && tab !== 'checks' && <div className="notice error"><span role="alert">面板快捷键 {configuredShortcut} 未能注册。</span><button onClick={() => setTab('checks')}>快捷键设置</button></div>}
      {data?.hiddenAll && <div className="notice">所有组件当前隐藏。请先恢复显示，再添加或显示单个组件。<button onClick={() => void run(() => action('restore-all'))}>恢复组件显示</button></div>}
      {tab === 'gallery' ? <OfficialGallery packages={data?.packages ?? []}
        instances={data?.instances ?? []} unavailable={busy || !data} canAdd={!data?.hiddenAll}
        onAdd={packageId => void run(() => invoke('create_instance', { packageId }), '已添加到侧边面板')}
        onUninstall={unregister} onManage={managePackageInstances} onRefresh={refresh} />
      : tab === 'library' ? <><div className="heading"><div><small>添加到面板，按需摆到桌面</small><h1>从一个组件开始</h1><p>添加后出现在侧边面板当前分组，可从组件标题栏放到桌面。同一种组件可添加多份，各自保存配置。</p></div><button disabled={busy || !data} onClick={() => void run(() => invoke('import_package'), '导入操作结束；已登记的兼容版本可在已添加中恢复。')}>导入本地 ZIP</button></div>
      <div className="library-grid">{data?.packages.map(p => <article className="package" key={p.id}><div className="package-symbol" data-kind={p.id}>▦</div><span className="badge">本地组件 · {p.version}</span><h2>{p.name}</h2><p>独立本地组件，通过受限宿主接口运行。</p><div className="package-actions"><button className="primary" disabled={busy || data.hiddenAll} onClick={() => void run(() => invoke('create_instance', { packageId: p.id }), '已添加到侧边面板')}>＋ 添加到面板</button><small>{data.instances.filter(i => i.packageId === p.id).length} 个实例</small></div><div className="gallery-actions actions">
        {data.instances.some(i => i.packageId === p.id) ? <button disabled={busy} onClick={() => managePackageInstances(p.id)}>管理实例</button>
          : <button disabled={busy} onClick={() => unregister(p.id)}>卸载</button>}
      </div></article>)}</div>
      {data?.packages.length === 0 && <div className="empty"><h2>还没有组件</h2><p>从官方广场安装组件，或导入本地 ZIP。</p><button onClick={() => setTab('gallery')}>浏览官方广场</button></div>}
      <p className="muted">本地组件按你信任的代码运行，组件包可独立导入。部分联网组件需自行配置服务或登录。</p></> : tab === 'instances' ? <><div className="heading"><div><small>桌面与面板，互不牵连</small><h1>已添加的组件</h1><p>眼睛按钮仅隐藏视图，垃圾桶按钮需确认后删除实例及其配置与自选，价格警报独立保留。</p></div></div>
      {focusedPackage && <p className="notice">正在管理 {data?.packages.find(p => p.id === focusedPackage)?.name ?? '组件'} 的实例。
        <button onClick={() => setFocusedPackage(undefined)}>显示全部</button></p>}
      {!data?.instances.some(i => !focusedPackage || i.packageId === focusedPackage) && <div className="empty"><h2>桌面还很安静</h2><p>到组件库添加第一个组件。</p><button onClick={() => setTab('library')}>浏览组件库</button></div>}
      {data?.instances.filter(i => !focusedPackage || i.packageId === focusedPackage).map(i => { const p = data.packages.find(p => p.id === i.packageId); return <article className="instance-row" key={i.id}><div className="instance-title"><h2>{p?.name ?? i.packageId}</h2><span className="badge">{i.location === 'desktop' ? '桌面' : '面板'} · {i.visible ? '已显示' : '已隐藏'}{i.pinned ? ' · 置顶' : ''}</span><small>实例 {i.id.slice(0, 8)} · v{p?.version}</small></div><div className="actions"><button disabled={busy || data.hiddenAll} onClick={() => void run(() => action(i.visible ? 'hide' : 'restore', i.id))}>{i.visible ? '隐藏' : '显示'}</button><button disabled={busy || data.hiddenAll} onClick={() => void run(() => action(i.location === 'desktop' ? 'embed' : 'float', i.id))}>{i.location === 'desktop' ? '移入面板' : '放回桌面'}</button>{i.visible && i.location === 'desktop' && <button disabled={busy || data.hiddenAll} onClick={() => void run(() => action('pin', i.id))}>{i.pinned ? '取消置顶' : '置顶'}</button>}{p?.id === 'local.souldesk.market' && <button disabled={busy || data.hiddenAll || !i.visible} onClick={() => void run(() => action('detail', i.id))}>自选与警报</button>}</div>
      <details><summary>尺寸、排序与删除</summary><form className="size-form" key={`${i.id}-${i.location}-${data.panelLayouts[i.id]?.width}-${data.panelLayouts[i.id]?.height}-${i.width}-${i.height}`} onSubmit={e => { e.preventDefault(); const f = new FormData(e.currentTarget); void run(() => invoke('manage_instance', { instance: i.id, action: 'resize', width: Number(f.get('width')), height: Number(f.get('height')) }), '尺寸已保存'); }}><label>宽度<input name="width" type="number" min={p?.minSize.width} max={i.location === "panel" ? 16384 : 1600} defaultValue={(i.location === "panel" ? data.panelLayouts[i.id]?.width : undefined) ?? i.width ?? p?.defaultSize.width} required /></label><label>高度<input name="height" type="number" min={p?.minSize.height} max={i.location === "panel" ? 16384 : 1200} defaultValue={(i.location === "panel" ? data.panelLayouts[i.id]?.height : undefined) ?? i.height ?? p?.defaultSize.height} required /></label><button disabled={busy}>保存尺寸</button><button type="button" disabled={busy || data.instances[0]?.id === i.id} onClick={() => { const ids = data.instances.map(i => i.id), at = ids.indexOf(i.id); [ids[at - 1], ids[at]] = [ids[at], ids[at - 1]]; void run(() => invoke('reorder_instances', { ids })); }}>列表前移</button><button className="danger" type="button" disabled={busy} onClick={() => { if (window.confirm('删除此实例及其配置和自选？已有价格警报与历史会保留，请在警报管理中另行暂停或删除。')) void run(() => invoke('manage_instance', { instance: i.id, action: 'remove' })); }}>删除实例</button></form></details></article>; })}</> : <>
      <div className="heading"><div><small>按你的习惯运行</small><h1>应用设置</h1><p>组件启动时恢复上次的桌面位置，面板与管理窗口保持关闭。</p></div></div>
      <section className="settings-panel app-theme-settings">
        <fieldset className="app-theme-picker" disabled={!data}>
          <legend>应用主题</legend>
          <div className="app-theme-options">{appThemes.map(theme => <label className="app-theme-option" key={theme.id}>
            <span className="app-theme-sample" data-app-theme={theme.id} aria-hidden="true"><i /><i /><i /></span>
            <span className="app-theme-caption">
              <input type="radio" name="app-theme" value={theme.id} checked={appTheme === theme.id} aria-disabled={busy} onChange={() => {
                if (!busy) void run(() => invoke('window_action', { action: 'set-app-theme', theme: theme.id }));
              }} />
              <span>{theme.name}</span><small>{theme.tone}</small>
            </span>
          </label>)}</div>
        </fieldset>
        <p className="muted">应用于管理窗口、侧边面板和桌面组件的宿主工具栏。</p>
      </section>
      <section className="settings-panel"><h2>组件外观</h2><label className="check"><input type="checkbox" checked={widgetBackdrop} disabled={busy || !data} onChange={e => { void run(() => action(e.target.checked ? 'enable-widget-backdrop' : 'disable-widget-backdrop')); }} />显示宿主衬底</label><p className="muted">为普通组件的透明区域铺上半透明主题底色。</p></section>
      <section className="settings-panel"><h2>组件面板</h2><p>未锁定时，移出面板或失焦 3 秒后收起，点击外部立即收起。锁定时，点击外部会置底，点击挂件切换置顶和置底。快捷键或双击桌面空白处可锁定并展开，或解锁并收起。</p>{panelControls}</section>
      <section className="settings-panel">
        <h2>快捷键与启动</h2>
        {shortcutUnavailable && <p role="alert" className="notice error">面板快捷键 {configuredShortcut} 未能注册。可更换键位，或释放原键后重新保存。</p>}
        <label className="check"><input type="checkbox" checked={shortcutEnabled} disabled={busy || !data} onChange={e => { const enabled = e.target.checked; void run(() => invoke('set_shortcut', { enabled }), '快捷键设置已保存'); }} />启用面板快捷键</label>
        <div className="setting-row"><label htmlFor="shortcut">开关辅助面板<small>设置的键位 {configuredShortcut ?? '—'}，冲突时保留原键</small></label><input id="shortcut" placeholder="Ctrl+Alt+Space" value={shortcutInput} onChange={e => setShortcut(e.target.value)} /><button disabled={busy || !data || !shortcutInput.trim()} onClick={() => void run(() => invoke('set_shortcut', { value: shortcutInput.trim() }), '快捷键已保存')}>保存</button></div>
        <label className="check"><input type="checkbox" disabled={busy || data?.autostart == null} checked={data?.autostart === true} onChange={e => { const enabled = e.target.checked; void run(() => invoke('set_autostart', { enabled }), '登录启动设置已保存'); }} />登录后后台启动</label>
        <button onClick={() => void run(() => action('repair'), '已校正可达位置')}>找回屏幕外组件</button>
      </section>
      <UpdateSettings state={data?.updates} refresh={refresh} /><NetworkSettings /><section className="settings-panel"><h2>本地数据与诊断</h2><p>数据目录：{data?.diagnostics.dataRoot ?? '未连接'}</p><p className="muted">{data?.diagnostics.views ?? 0} 个宿主窗口 · {data?.diagnostics.sessions ?? 0} 个组件会话。隐藏组件释放展示，自选与有效规则继续运行。</p><div className="actions"><button disabled={busy} onClick={() => void run(() => invoke('schedule_test', { notification: true, sound: false, delay: 3 }), '已安排通知测试')}>测试通知</button><button disabled={busy} onClick={() => void run(() => invoke('schedule_test', { notification: false, sound: true, delay: 3 }), '已安排声音测试')}>测试声音</button></div></section>
      </>}
    </div><footer className="statusbar"><span><i className="dot" /> 本地运行 · 组件隐藏不停止有效规则</span><span>SoulDesk {data?.updates.currentVersion ?? appPackage.version}</span></footer></section>
  </main>;
}
