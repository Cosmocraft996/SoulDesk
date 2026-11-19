import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from 'react';
import { invoke } from './host';
import { WidgetFrame } from './WidgetFrame';
import { WidgetSurface, type SurfaceAction } from './WidgetSurface';
import { changeLayout, initialLayouts, type PanelLayout, type PanelWidget, type PanelPackage, type SnapEdge, type SnapMatch } from './panel-layout';

export interface PanelScroll { left: number; top: number }

export function PanelCanvas({ group, instances, packages, saved, version, items, onError, position, onScrollPosition, enabled = true, target }: {
  group: string | null;
  instances: PanelWidget[]; packages: PanelPackage[]; saved: Record<string, PanelLayout>;
  version: (id: string) => string | undefined;
  items: (id: string) => SurfaceAction[];
  onError: (message: string) => void;
  position?: PanelScroll;
  onScrollPosition?: (group: string | null, position: PanelScroll) => void;
  enabled?: boolean;
  target?: string | null;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const [measuredWidth, setMeasuredWidth] = useState<number>();
  const [viewportHeight, setViewportHeight] = useState(0);
  const available = measuredWidth ?? 400;
  const [loaded, setLoaded] = useState(() => new Set<string>());
  const lastScroll = useRef<PanelScroll | undefined>(undefined);
  const [draft, setDraft] = useState<Record<string, PanelLayout>>({});
  const [active, setActive] = useState<string>();
  const [saving, setSaving] = useState<string>();
  const [front, setFront] = useState<string>();
  const [snapEdges, setSnapEdges] = useState<Record<string, SnapEdge[]>>({});
  const drag = useRef<{ id: string; pointer: number; x: number; y: number; scroll: number; resize: boolean; start: PanelLayout; current: PanelLayout } | null>(null);
  const layouts = initialLayouts(instances, packages, { ...saved, ...draft }, available);
  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node) return;
    const measure = () => {
      setMeasuredWidth(Math.max(0, node.clientWidth - 24));
      setViewportHeight(node.clientHeight);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    measure();
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node || measuredWidth === undefined || lastScroll.current) return;
    node.scrollLeft = position?.left ?? 0;
    node.scrollTop = position?.top ?? 0;
    lastScroll.current = { left: node.scrollLeft, top: node.scrollTop };
  }, [measuredWidth, position]);
  useEffect(() => {
    const node = viewport.current;
    if (!enabled || !node || measuredWidth === undefined) return;
    const observer = new IntersectionObserver(entries => {
      const visible: string[] = [];
      for (const entry of entries) {
        const id = (entry.target as HTMLElement).dataset.panelInstance;
        if (entry.isIntersecting && id) {
          observer.unobserve(entry.target);
          visible.push(id);
        }
      }
      if (visible.length) setLoaded(current => new Set([...current, ...visible]));
    }, { root: node, rootMargin: '180px' });
    for (const card of node.querySelectorAll<HTMLElement>('[data-panel-instance]')) {
      observer.observe(card);
    }
    return () => observer.disconnect();
  }, [enabled, measuredWidth, instances]);
  useEffect(() => {
    if (enabled && target) setLoaded(current => current.has(target) ? current : new Set([...current, target]));
  }, [enabled, target]);
  useEffect(() => {
    if (drag.current || saving) return;
    // 等待后台快照确认，避免松手后短暂跳回旧位置；失败时由保存分支撤销临时布局。
    setDraft(current => {
      const pending = Object.fromEntries(Object.entries(current).filter(([id, g]) => instances.some(i => i.id === id) && !['x', 'y', 'width', 'height'].every(key => saved[id]?.[key as keyof PanelLayout] === g[key as keyof PanelLayout])));
      return Object.keys(pending).length === Object.keys(current).length ? current : pending;
    });
  }, [saved, instances, saving]);
  const begin = (event: PointerEvent<HTMLButtonElement>, id: string, resize: boolean) => {
    if (event.button !== 0 || drag.current || saving) return;
    event.preventDefault();
    const target = event.currentTarget, pointer = event.pointerId;
    const start = layouts[id];
    if (!start) return;
    target.setPointerCapture(pointer);
    drag.current = { id, pointer, x: event.clientX, y: event.clientY, scroll: viewport.current?.scrollTop ?? 0, resize, start, current: start };
    setFront(id);
    setActive(id);
  };
  const move = (event: PointerEvent) => {
    const state = drag.current;
    if (!state || state.pointer !== event.pointerId) return;
    const pack = packages.find(p => p.id === instances.find(i => i.id === state.id)?.packageId);
    if (!pack) return;
    const dy = event.clientY - state.y + (viewport.current?.scrollTop ?? 0) - state.scroll;
    const node = viewport.current;
    const peers = Object.entries(layouts).filter(([id]) => id !== state.id);
    const matches: SnapMatch[] = [];
    state.current = changeLayout(state.start, event.clientX - state.x, dy, state.resize, available, pack.minSize,
      peers.map(([, layout]) => layout),
      node ? { top: node.scrollTop, bottom: node.scrollTop + Math.max(0, node.clientHeight - 24) } : undefined, matches);
    const edges: Record<string, SnapEdge[]> = {};
    for (const match of matches) {
      (edges[state.id] ??= []).push(match.edge);
      if (match.peer !== undefined) (edges[peers[match.peer][0]] ??= []).push(match.peerEdge);
    }
    setSnapEdges(edges);
    setDraft(current => ({ ...current, [state.id]: state.current }));
  };
  const finish = async (event: PointerEvent, canceled = false) => {
    const state = drag.current;
    if (!state || state.pointer !== event.pointerId) return;
    drag.current = null; setActive(undefined); setSnapEdges({}); setSaving(state.id);
    try {
      if (!canceled) {
        setDraft(current => ({ ...current, [state.id]: state.current }));
        await invoke('save_panel_layout', { instance: state.id, layout: state.current });
      }
    } catch (error) { canceled = true; onError(`布局保存失败：${String(error)}`); }
    finally {
      if (canceled) setDraft(current => Object.fromEntries(Object.entries(current).filter(([id]) => id !== state.id)));
      setSaving(undefined);
    }
  };
  const values = Object.values(layouts);
  const height = Math.max(180, ...values.map(g => g.y + g.height + 12)) + viewportHeight * 0.5;
  const width = Math.max(available, ...values.map(g => g.x + g.width));
  return <div ref={viewport} className={`panel-scroll${active ? ' manipulating' : ''}`} onScroll={event => {
    const prev = lastScroll.current;
    const { scrollLeft: left, scrollTop: top } = event.currentTarget;
    if (prev && (prev.left !== left || prev.top !== top)) {
      lastScroll.current = { left, top };
      onScrollPosition?.(group, { left, top });
    }
  }} onPointerMove={move} onPointerUp={event => void finish(event)} onPointerCancel={event => void finish(event, true)} onLostPointerCapture={event => void finish(event, true)}>
    <div className="panel-canvas" style={{ height, width }}>
      {instances.map(instance => { const g = layouts[instance.id]; return g && <article className="panel-widget" key={instance.id} data-panel-instance={instance.id} onPointerDownCapture={() => setFront(instance.id)} style={{ left: g.x, top: g.y, width: g.width, height: g.height, zIndex: front === instance.id ? 2 : 1 }}>
        <WidgetSurface instance={instance.id} inPanel snapEdges={snapEdges[instance.id]} items={items(instance.id)} onError={onError} onDrag={event => void begin(event, instance.id, false)} onResize={event => void begin(event, instance.id, true)}>
          {enabled && (loaded.has(instance.id) || target === instance.id) ? <WidgetFrame key={version(instance.id)} instance={instance.id} /> : <div className="frame-wrap"><div className="frame-message">等待加载…</div></div>}
        </WidgetSurface>
      </article>; })}
    </div>
  </div>;
}
