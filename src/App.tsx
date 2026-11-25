import { useCallback, useEffect, useLayoutEffect, useRef, useState, type DragEvent, type KeyboardEvent } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import appPackage from '../package.json' with { type: 'json' };
import { invoke } from './host';
import { OfficialGallery, UpdateAllWidgets, type CatalogResponse } from './OfficialGallery';
import { WidgetBrowser, widgetCategory } from './WidgetBrowser';
import { WidgetFrame } from './WidgetFrame';
import { NetworkSettings } from './NetworkSettings';
import { NotificationSettings } from './NotificationSettings';
import { heldShortcutModifiers, shortcutMainKey } from './shortcut-keys.ts';
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
  packages: Package[]; instances: Instance[]; autostart: boolean | null; silentStartup: boolean; hiddenAll: boolean; panelOpen: boolean; panelLocked: boolean; panelLowered?: boolean; panelShortcutEnabled?: boolean; panelGeometry?: { effective_ratio: number }; panel: { ratio: number; tint: number; slideInMs: number; slideOutMs: number; desktopDoubleClick: boolean; edge: EdgeVisual['position']; edgeStyle: EdgeStyle }; panelLayouts: Record<string, PanelLayout>; updates: UpdateState;
  panelShortcutRegistered?: boolean;
  diagnostics: { ticks: number; tasks: number; sessions: number; views: number; error: string | null; dataRoot: string; shortcut: string };
}
const action = (action: string, instance?: string) => invoke('window_action', { action, instance });

function WidgetPackageImport({ disabled, onImport, onError }: {
  disabled: boolean;
  onImport: (file?: File) => Promise<boolean>;
  onError: (message: string) => void;
}) {
  const importing = useRef(false);
  const [dragOver, setDragOver] = useState(false);
  const importOnce = async (file?: File) => {
    if (disabled || importing.current) return;
    importing.current = true;
    try { await onImport(file); }
    finally { importing.current = false; }
  };
  const acceptDrag = (event: DragEvent<HTMLDivElement>) => {
    if (!event.dataTransfer.types.includes('Files')) return;
    event.preventDefault();
    const available = !disabled && !importing.current;
    event.dataTransfer.dropEffect = available ? 'copy' : 'none';
    setDragOver(available);
  };
  const drop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setDragOver(false);
    if (disabled || importing.current) return;
    const files = event.dataTransfer.files;
    if (files.length !== 1 || !/\.zip$/i.test(files[0].name)) {
      onError('请一次拖入一个组件 ZIP 文件。');
      return;
    }
    const file = files[0];
    if (file.size > 20 * 1024 * 1024) {
      onError('组件 ZIP 超过 20 MiB。');
      return;
    }
    void importOnce(file);
  };
  return <div className="widget-import" role="group" aria-label="导入组件" data-drag-over={dragOver && !disabled || undefined}
    onDragEnter={acceptDrag} onDragOver={acceptDrag} onDrop={drop}
    onDragLeave={event => {
      if (!(event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget))) setDragOver(false);
    }}>
    <span><Icon name="download" />把组件 ZIP 拖到这里</span>
    <button type="button" disabled={disabled} onClick={() => void importOnce()}>导入本地 ZIP</button>
  </div>;
}

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
  const [dropGroup, setDropGroup] = useState<string | null>();
  const [focusedPackage, setFocusedPackage] = useState<string>();
  const [tab, setTab] = useState<'gallery' | 'library' | 'instances' | 'checks'>(query.get('tab') === 'checks' ? 'checks' : 'library');
  const [notificationHistory, setNotificationHistory] = useState(() => {
    const id = query.get('notificationHistory');
    return id ? { id } : undefined;
  });
  const [localCatalog, setLocalCatalog] = useState<CatalogResponse['catalog']>(null);
  const [updatingAll, setUpdatingAll] = useState(false);
  const browsingLocal = (tab === 'library' || tab === 'instances') && !panel && !edge && !desktop && !detail && !trial;
  useEffect(() => {
    if (!browsingLocal || !isTauri()) return;
    let current = true;
    void invoke<CatalogResponse>('official_catalog', { cachedOnly: true }).then(
      result => {
        const catalog = result?.catalog;
        if (current && catalog) {
          setLocalCatalog(previous => !previous || catalog.revision >= previous.revision ? catalog : previous);
        }
      },
      () => {},
    );
    return () => { current = false; };
  }, [browsingLocal]);
  const [shortcut, setShortcut] = useState<string>();
  const [shortcutModifiers, setShortcutModifiers] = useState('');
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
  const canSaveShortcut = !!shortcutInput && !shortcutModifiers && (!!shortcut || shortcutUnavailable);
  const captureShortcut = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.code === 'Tab' && !event.ctrlKey && !event.altKey && !event.metaKey) return;
    if (event.nativeEvent.isComposing || event.key === 'Process') return;
    event.preventDefault();
    if (event.repeat) return;
    const modifiers = heldShortcutModifiers(event);
    if (!modifiers) {
      if (event.code === 'Escape') {
        setShortcut(undefined);
        event.currentTarget.blur();
      }
      return;
    }
    const key = shortcutMainKey(event);
    setShortcutModifiers(key ? '' : modifiers);
    if (key) setShortcut(modifiers + '+' + key);
  };
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
    const notificationNav = listen<string>('host:notification-history', event => {
      setTab('checks');
      setNotificationHistory({ id: event.payload });
    });
    const geometry = listen<{ effective_ratio: number }>('host:panel-geometry', e => setEffective(e.payload.effective_ratio));
    const offPage = listen<string>('host:detail-page', e => setDetailPage(e.payload));
    const timer = setInterval(refresh, 3000);
    return () => { clearInterval(timer); for (const item of [off, nav, notificationNav, geometry, offPage]) void item.then(fn => fn()); };
  }, [refresh, edge, trial]);
  useEffect(() => {
    if (!isTauri() || !(panel || edge)) return;
    const off = listen<EdgeVisual>('host:panel-edge', e => setEdgeVisual(e.payload));
    return () => { void off.then(unlisten => unlisten()); };
  }, [edge, panel]);
  const run = async <T,>(task: () => Promise<T>, success: string | ((result: T) => string) = '') => {
    setBusy(true); setMessage('');
    try {
      const result = await task();
      setMessage(typeof success === 'function' ? success(result) : success);
      await refresh();
      return true;
    } catch (e) { setMessage(`操作失败：${String(e)}`); return false; } finally { setBusy(false); }
  };
  const importPackage = (file?: File) => run(
    async () => invoke<Package | null>('import_package', file ? await file.arrayBuffer() : undefined),
    result => result ? `已导入“${result.name}”` : '',
  );
  const managePackageInstances = (packageId: string) => {
    setFocusedPackage(packageId);
    setTab('instances');
  };
  const unregister = (packageId: string) =>
    void run(() => invoke('unregister_package', { packageId }), '已卸载组件');
  const version = (id: string) => data?.packages.find(p => p.id === data.instances.find(i => i.id === id)?.packageId)?.version;
  const groups = data?.panelGroups ?? ungrouped;
  const panelInstances = data?.instances.filter(i => i.visible && i.location === 'panel' && (groups.membership[i.id] ?? null) === groups.active) ?? [];
  const changeGroup = (change: GroupChange, success = '') => run(() => invoke('manage_panel_group', { change }), success);
  const groupName = (id: string | null) => groups.groups.find(group => group.id === id)?.name ?? '未分组';
  const dropIntoGroup = (instance: string, group: string | null) =>
    changeGroup({ action: 'move', instance, group }, `已移到“${groupName(group)}”`);
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
      {data && <PanelGroups state={groups} instances={data.instances} busy={busy} message={message.startsWith('操作失败：') ? message : ''} onChange={changeGroup} dropGroup={dropGroup} />}
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
    <section id="panel-group-content" className="panel-group-content" role="tabpanel" aria-label={groupName(groups.active)}>
      {!data ? <div className="empty panel-empty" role="status">正在加载面板…</div> : panelInstances.length ? <PanelCanvas key={groups.active ?? 'ungrouped'} group={groups.active} instances={panelInstances} packages={data.packages} saved={data.panelLayouts} version={version} items={id => controls(id, true)} onError={setMessage} position={data.panelScroll} onScrollPosition={savePanelScroll} enabled={!panelView || panelReady} target={data.panelTarget} onGroupDropChange={setDropGroup} onMoveToGroup={dropIntoGroup} /> : <div className="empty panel-empty"><button className="icon-button" title="添加组件" aria-label="添加组件" onClick={() => void action('show')}><Icon name="plus" width="28" height="28" /></button><p>{groups.active ? '这个分组还没有可见组件' : '从组件库添加到面板'}<br />可用组件工具栏的“移动到分组”按钮调整归属</p></div>}
    </section>
  </main>;
  const catalogById = new Map(localCatalog?.widgets.map(entry => [entry.id, entry]));
  const localPackages = data?.packages.map(pkg => {
    const official = catalogById.get(pkg.id);
    return { ...pkg, category: widgetCategory(pkg.id, official?.category), summary: official?.summary };
  }) ?? [];
  const localById = new Map(localPackages.map(pkg => [pkg.id, pkg]));
  const managedInstances = data?.instances.filter(i => !focusedPackage || i.packageId === focusedPackage).map(instance => {
    const pkg = localById.get(instance.packageId);
    return { ...instance, pkg, name: pkg?.name ?? instance.packageId,
      category: pkg?.category ?? widgetCategory(instance.packageId, catalogById.get(instance.packageId)?.category),
      summary: pkg?.summary };
  }) ?? [];

  return <main className="management-shell"
    onDragOver={event => {
      if (!event.defaultPrevented) {
        event.preventDefault();
        event.dataTransfer.dropEffect = 'none';
      }
    }}
    onDrop={event => event.preventDefault()}
    onPointerDownCapture={e => {
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
      : tab === 'library' ? <><div className="heading"><div><small>添加到面板，按需摆到桌面</small><h1>从一个组件开始</h1><p>添加后出现在侧边面板当前分组，可从组件标题栏放到桌面。同一种组件可添加多份，各自保存配置。</p></div>
        <div className="actions library-heading-actions">
          <button disabled={busy || !localPackages.length} onClick={() => setUpdatingAll(true)}>全部更新</button>
        </div>
      </div>
      <WidgetPackageImport disabled={busy || !data || updatingAll} onImport={importPackage} onError={setMessage} />
      {data && <WidgetBrowser items={localPackages}>
        {pkg => {
          const count = data.instances.filter(instance => instance.packageId === pkg.id).length;
          return <article className="package" key={pkg.id}>
            <div className="package-symbol" data-kind={pkg.id}>▦</div>
            <span className="badge">{pkg.category} · v{pkg.version}</span>
            <h2>{pkg.name}</h2>
            {pkg.summary && <p>{pkg.summary}</p>}
            <div className="package-actions">
              <button className="primary" disabled={busy || data.hiddenAll}
                onClick={() => void run(() => invoke('create_instance', { packageId: pkg.id }), '已添加到侧边面板')}>＋ 添加到面板</button>
              <small>{count} 个实例</small>
            </div>
            <div className="gallery-actions actions">
              {count ? <button disabled={busy} onClick={() => managePackageInstances(pkg.id)}>管理实例</button>
                : <button disabled={busy} onClick={() => unregister(pkg.id)}>卸载</button>}
            </div>
          </article>;
        }}
      </WidgetBrowser>}
      {data?.packages.length === 0 && <div className="empty"><h2>还没有组件</h2><p>从官方广场安装组件，或导入本地 ZIP。</p><button onClick={() => setTab('gallery')}>浏览官方广场</button></div>}
      <p className="muted">本地组件按你信任的代码运行，组件包可独立导入。部分联网组件需自行配置服务或登录。</p></> : tab === 'instances' ? <><div className="heading"><div><small>桌面与面板，互不牵连</small><h1>已添加的组件</h1><p>眼睛按钮仅隐藏视图，垃圾桶按钮需确认后删除实例及其配置与自选，价格警报独立保留。</p></div></div>
      {focusedPackage && <p className="notice">正在管理 {data?.packages.find(p => p.id === focusedPackage)?.name ?? '组件'} 的实例。
        <button onClick={() => setFocusedPackage(undefined)}>显示全部</button></p>}
      {data && !managedInstances.length && <div className="empty"><h2>{focusedPackage ? '这个组件还没有实例' : '桌面还很安静'}</h2><p>到组件库添加组件。</p><button onClick={() => setTab('library')}>浏览组件库</button></div>}
      {data && <WidgetBrowser key={focusedPackage ?? 'all-instances'} items={managedInstances} layout="list">
        {instance => {
          const { id, pkg, visible, pinned, name, category } = instance;
          const inPanel = instance.location === 'panel';
          const locationLocked = busy || data.hiddenAll;
          const layout = data.panelLayouts[id];
          const width = (inPanel ? layout?.width : undefined) ?? instance.width ?? pkg?.defaultSize.width;
          const height = (inPanel ? layout?.height : undefined) ?? instance.height ?? pkg?.defaultSize.height;
          return <article className="instance-row" key={id}>
            <div className="instance-title">
              <h2>{name}</h2>
              <span className="badge">{category} · {inPanel ? '面板' : '桌面'} · {visible ? '已显示' : '已隐藏'}{pinned ? ' · 置顶' : ''}</span>
              <small>实例 {id.slice(0, 8)}{pkg && ' · v' + pkg.version}</small>
            </div>
            <div className="actions">
              <button disabled={locationLocked} onClick={() => void run(() => action(visible ? 'hide' : 'restore', id))}>{visible ? '隐藏' : '显示'}</button>
              <button disabled={locationLocked} onClick={() => void run(() => action(inPanel ? 'float' : 'embed', id))}>{inPanel ? '放回桌面' : '移入面板'}</button>
              {visible && !inPanel && <button disabled={locationLocked} onClick={() => void run(() => action('pin', id))}>{pinned ? '取消置顶' : '置顶'}</button>}
              {pkg?.id === 'local.souldesk.market' && <button disabled={locationLocked || !visible}
                onClick={() => void run(() => action('detail', id))}>自选与警报</button>}
            </div>
            <details>
              <summary>尺寸、排序与删除</summary>
              <form className="size-form" key={`${id}-${instance.location}-${layout?.width}-${layout?.height}-${instance.width}-${instance.height}`} onSubmit={event => {
                event.preventDefault();
                const form = new FormData(event.currentTarget);
                void run(() => invoke('manage_instance', {
                  instance: id, action: 'resize', width: Number(form.get('width')), height: Number(form.get('height')),
                }), '尺寸已保存');
              }}>
                <label>宽度<input name="width" type="number" min={pkg?.minSize.width} max={inPanel ? 16384 : 1600} defaultValue={width} required /></label>
                <label>高度<input name="height" type="number" min={pkg?.minSize.height} max={inPanel ? 16384 : 1200} defaultValue={height} required /></label>
                <button disabled={busy}>保存尺寸</button>
                <button type="button" disabled={busy || data.instances[0]?.id === id} onClick={() => {
                  const ids = data.instances.map(item => item.id);
                  const index = ids.indexOf(id);
                  [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]];
                  void run(() => invoke('reorder_instances', { ids }));
                }}>列表前移</button>
                <button className="danger" type="button" disabled={busy} onClick={() => {
                  if (window.confirm('删除此实例及其配置和自选？已有价格警报与历史会保留，请在警报管理中另行暂停或删除。')) {
                    void run(() => invoke('manage_instance', { instance: id, action: 'remove' }));
                  }
                }}>删除实例</button>
              </form>
            </details>
          </article>;
        }}
      </WidgetBrowser>}</> : <>
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
      <section className="settings-panel"><h2>桌面格子</h2><p>在桌面原地分组文件和快捷方式，保留原路径。</p><div className="actions"><button disabled={busy || !data} onClick={() => void run(() => action('desktop-grid'))}>打开格子管理</button><button disabled={busy || !data} onClick={() => void run(() => action('desktop-grid-restore'))}>恢复 Windows 桌面图标</button></div></section>
      <NotificationSettings historyRequest={notificationHistory} />
      <section className="settings-panel"><h2>组件面板</h2><p>未锁定时，移出面板或失焦 3 秒后收起，点击外部立即收起。锁定时，点击外部会置底，点击挂件切换置顶和置底。快捷键或双击桌面空白处可锁定并展开，或解锁并收起。</p>{panelControls}</section>
      <section className="settings-panel">
        <h2>快捷键与启动</h2>
        {shortcutUnavailable && <p role="alert" className="notice error">面板快捷键 {configuredShortcut} 未能注册。可更换键位，或释放原键后重新保存。</p>}
        <label className="check"><input type="checkbox" checked={shortcutEnabled} disabled={busy || !data} onChange={e => { const enabled = e.target.checked; void run(() => invoke('set_shortcut', { enabled }), '快捷键设置已保存'); }} />启用面板快捷键</label>
        <div className="setting-row">
          <label htmlFor="shortcut">开关辅助面板<small id="shortcut-help">点击输入框，按住 Ctrl、Alt、Shift 或 Win，再按一个主键。按 Esc 取消。当前键位 {configuredShortcut?.replace(/\bSuper\b/gi, 'Win') ?? '—'}</small></label>
          <input id="shortcut" className="shortcut-capture" aria-describedby="shortcut-help" readOnly
            disabled={busy || !data} value={(shortcutModifiers ? shortcutModifiers + '+…' : shortcutInput).replace(/\bSuper\b/gi, 'Win')}
            onKeyDown={captureShortcut} onKeyUp={event => { if (shortcutModifiers) setShortcutModifiers(heldShortcutModifiers(event)); }}
            onBlur={() => setShortcutModifiers('')} />
          <button disabled={busy || !data || !canSaveShortcut} onClick={() => void run(
            () => invoke('set_shortcut', { value: shortcutInput }), '快捷键已保存',
          ).then(saved => { if (saved) setShortcut(undefined); })}>保存</button>
        </div>
        <label className="check"><input type="checkbox" disabled={busy || data?.autostart == null} checked={data?.autostart === true} onChange={e => { const enabled = e.target.checked; void run(() => invoke('set_autostart', { enabled }), '开机启动设置已保存'); }} />开机启动（登录 Windows 后）</label>
        <label className="check"><input type="checkbox" disabled={busy || !data} checked={data?.silentStartup === true} onChange={e => { const silent = e.target.checked; void run(() => invoke('set_autostart', { silent }), '静默启动设置已保存'); }} />静默启动</label>
        <p className="muted">桌面组件和挂件照常恢复。开启静默启动后，管理窗口和面板保持关闭。</p>
        <button onClick={() => void run(() => action('repair'), '已校正可达位置')}>找回屏幕外组件</button>
      </section>
      <UpdateSettings state={data?.updates} refresh={refresh} /><NetworkSettings /><section className="settings-panel"><h2>本地数据与诊断</h2><p>数据目录：{data?.diagnostics.dataRoot ?? '未连接'}</p><p className="muted">{data?.diagnostics.views ?? 0} 个宿主窗口 · {data?.diagnostics.sessions ?? 0} 个组件会话。隐藏组件释放展示，自选与有效规则继续运行。</p><div className="actions"><button disabled={busy} onClick={() => void run(() => invoke('schedule_test', { notification: true, sound: false, delay: 3 }), '已安排通知测试')}>测试通知</button><button disabled={busy} onClick={() => void run(() => invoke('schedule_test', { notification: false, sound: true, delay: 3 }), '已安排声音测试')}>测试声音</button></div></section>
      </>}
      {updatingAll && <UpdateAllWidgets packages={data?.packages ?? []} onCatalog={setLocalCatalog}
        onClose={() => setUpdatingAll(false)} onInstalled={refresh} />}
    </div><footer className="statusbar"><span><i className="dot" /> 本地运行 · 组件隐藏不停止有效规则</span><span>SoulDesk {data?.updates.currentVersion ?? appPackage.version}</span></footer></section>
  </main>;
}
