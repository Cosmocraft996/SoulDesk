import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from 'react';
import { invoke } from './host';
import { WidgetFrame } from './WidgetFrame';
import { WidgetSurface } from './WidgetSurface';
import { changeLayout, initialLayouts, type PanelLayout, type PanelWidget, type PanelPackage } from './panel-layout';
import type { IconName } from './Icon';

export function PanelCanvas({ instances, packages, saved, version, items, onError }: {
  instances: PanelWidget[]; packages: PanelPackage[]; saved: Record<string, PanelLayout>;
  version: (id: string) => string | undefined;
  items: (id: string) => { label: string; icon: IconName; action: () => void; disabled?: boolean }[];
  onError: (message: string) => void;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const [available, setAvailable] = useState(400);
  const [draft, setDraft] = useState<Record<string, PanelLayout>>({});
  const [active, setActive] = useState<string>();
  const [saving, setSaving] = useState<string>();
  const [front, setFront] = useState<string>();
  const drag = useRef<{ id: string; pointer: number; x: number; y: number; scroll: number; resize: boolean; start: PanelLayout; current: PanelLayout } | null>(null);
  const layouts = initialLayouts(instances, packages, { ...saved, ...draft }, available);
  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node) return;
    const observer = new ResizeObserver(() => setAvailable(Math.max(0, node.clientWidth - 24)));
    observer.observe(node);
    setAvailable(Math.max(0, node.clientWidth - 24));
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (drag.current || saving) return;
    // 等待后台快照确认，避免松手后短暂跳回旧位置；失败时由保存分支撤销临时布局。
    setDraft(current => {
      const pending = Object.fromEntries(Object.entries(current).filter(([id, g]) => instances.some(i => i.id === id) && !['x', 'y', 'width', 'height'].every(key => saved[id]?.[key as keyof PanelLayout] === g[key as keyof PanelLayout])));
      return Object.keys(pending).length === Object.keys(current).length ? current : pending;
    });
  }, [saved, instances, saving]);
  useEffect(() => () => { void invoke('window_action', { action: 'interaction-end' }).catch(() => {}); }, []);
  const begin = async (event: PointerEvent<HTMLButtonElement>, id: string, resize: boolean) => {
    if (event.button !== 0 || drag.current || saving) return;
    event.preventDefault();
    const target = event.currentTarget, pointer = event.pointerId;
    const start = layouts[id];
    if (!start) return;
    target.setPointerCapture(pointer);
    const state = { id, pointer, x: event.clientX, y: event.clientY, scroll: viewport.current?.scrollTop ?? 0, resize, start, current: start };
    drag.current = state; setFront(id);
    try {
      await invoke('window_action', { action: 'interaction-start' });
      // 快速点击已抬起时不进入拖动，仍释放后台保护。
      if (drag.current === state && target.hasPointerCapture(pointer)) setActive(id);
      else await invoke('window_action', { action: 'interaction-end' });
    } catch (error) { drag.current = null; setActive(undefined); onError(String(error)); void invoke('window_action', { action: 'interaction-end' }).catch(() => {}); }
  };
  const move = (event: PointerEvent) => {
    const state = drag.current;
    if (!state || state.pointer !== event.pointerId) return;
    const pack = packages.find(p => p.id === instances.find(i => i.id === state.id)?.packageId);
    if (!pack) return;
    const dy = event.clientY - state.y + (viewport.current?.scrollTop ?? 0) - state.scroll;
    state.current = changeLayout(state.start, event.clientX - state.x, dy, state.resize, available, pack.minSize);
    setDraft(current => ({ ...current, [state.id]: state.current }));
  };
  const finish = async (event: PointerEvent, canceled = false) => {
    const state = drag.current;
    if (!state || state.pointer !== event.pointerId) return;
    drag.current = null; setActive(undefined); setSaving(state.id);
    try {
      if (!canceled) {
        setDraft(current => ({ ...current, [state.id]: state.current }));
        await invoke('save_panel_layout', { instance: state.id, layout: state.current });
      }
    } catch (error) { canceled = true; onError(`布局保存失败：${String(error)}`); }
    finally {
      if (canceled) setDraft(current => Object.fromEntries(Object.entries(current).filter(([id]) => id !== state.id)));
      setSaving(undefined);
      await invoke('window_action', { action: 'interaction-end' }).catch(error => onError(String(error)));
    }
  };
  const values = Object.values(layouts);
  const height = Math.max(180, ...values.map(g => g.y + g.height + 12));
  const width = Math.max(available, ...values.map(g => g.x + g.width));
  return <div ref={viewport} className={`panel-scroll${active ? ' manipulating' : ''}`} onPointerMove={move} onPointerUp={event => void finish(event)} onPointerCancel={event => void finish(event, true)} onLostPointerCapture={event => void finish(event, true)}>
    <div className="panel-canvas" style={{ height, width }}>
      {instances.map(instance => { const g = layouts[instance.id]; return g && <article className="panel-widget" key={instance.id} onPointerDownCapture={() => setFront(instance.id)} style={{ left: g.x, top: g.y, width: g.width, height: g.height, zIndex: front === instance.id ? 2 : 1 }}>
        <WidgetSurface instance={instance.id} inPanel items={items(instance.id)} onError={onError} onDrag={event => void begin(event, instance.id, false)} onResize={event => void begin(event, instance.id, true)}>
          <WidgetFrame key={version(instance.id)} instance={instance.id} />
        </WidgetSurface>
      </article>; })}
    </div>
  </div>;
}
