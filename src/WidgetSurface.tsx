import { useEffect, useState, type PointerEventHandler, type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';
import { invoke } from './host';
import { listen } from '@tauri-apps/api/event';
import type { SnapEdge } from './panel-layout';

export interface SurfaceAction { label: string; icon: IconName; action: () => void; disabled?: boolean; more?: boolean }
const edges: SnapEdge[] = ['left', 'right', 'top', 'bottom'];
export function WidgetSurface({ instance, inPanel = false, snapEdges, items, onDrag, onResize, children, notice, onError }: {
  instance: string; inPanel?: boolean; items: SurfaceAction[];
  snapEdges?: SnapEdge[];
  onDrag: PointerEventHandler<HTMLButtonElement>; onResize: PointerEventHandler<HTMLButtonElement>;
  children: ReactNode; notice?: ReactNode; onError: (message: string) => void;
}) {
  const [menu, setMenu] = useState(false);
  const [desktopEdges, setDesktopEdges] = useState<SnapEdge[]>([]);
  useEffect(() => {
    if (inPanel) return;
    let disposed = false;
    setDesktopEdges([]);
    const stop = listen<SnapEdge[]>('host:snap-edges', event => {
      if (!disposed && Array.isArray(event.payload)) setDesktopEdges(edges.filter(edge => event.payload.includes(edge)));
    });
    void stop.catch(error => { if (!disposed) onError(String(error)); });
    return () => { disposed = true; void stop.then(off => off()).catch(() => {}); };
  }, [inPanel, instance]);
  useEffect(() => {
    if (!inPanel || !menu) return;
    void invoke('window_action', { action: 'menu-open', instance }).catch(e => onError(String(e)));
    return () => { void invoke('window_action', { action: 'menu-close', instance }).catch(() => {}); };
  }, [inPanel, menu, instance]);
  const button = (item: SurfaceAction) => <button className="icon-button" key={item.label} title={item.label} aria-label={item.label} disabled={item.disabled} onClick={() => { setMenu(false); item.action(); }}><Icon name={item.icon} /></button>;
  return <section className={`widget-surface${menu ? ' menu-open' : ''}`}>
    <div className="surface-chrome">
      <button className="drag-handle icon-button" title="拖动组件" aria-label="拖动组件" onPointerDown={onDrag}><Icon name="grip" /></button>
      <div className="surface-toolbar">
        {items.filter(item => !item.more).map(button)}
        <button className="icon-button" title="更多功能" aria-label="更多功能" aria-expanded={menu} onClick={() => setMenu(!menu)}><Icon name="more" /></button>
      </div>
    </div>
    {children}
    {menu && <div className="surface-menu"><div className="surface-actions">
      {items.filter(item => item.more).map(button)}
    </div>{notice}</div>}
    <button className="resize-handle icon-button" title="调整组件尺寸" aria-label="调整组件尺寸" onPointerDown={onResize}><Icon name="resize" /></button>
    {!menu && notice}
    <div className="snap-highlights" aria-hidden="true">
      {edges.map(edge => <span key={edge} data-edge={edge} className={`snap-highlight snap-highlight-${edge}${(inPanel ? snapEdges : desktopEdges)?.includes(edge) ? ' active' : ''}`}><i /></span>)}
    </div>
  </section>;
}
