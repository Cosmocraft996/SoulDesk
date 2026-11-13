import { useEffect, useState, type PointerEventHandler, type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';
import { invoke } from './host';

interface Item { label: string; icon: IconName; action: () => void; disabled?: boolean }
export function WidgetSurface({ instance, inPanel = false, items, onDrag, onResize, children, notice, onError }: {
  instance: string; inPanel?: boolean; items: Item[];
  onDrag: PointerEventHandler<HTMLButtonElement>; onResize: PointerEventHandler<HTMLButtonElement>;
  children: ReactNode; notice?: ReactNode; onError: (message: string) => void;
}) {
  const [menu, setMenu] = useState(false);
  useEffect(() => {
    if (!inPanel || !menu) return;
    void invoke('window_action', { action: 'menu-open', instance }).catch(e => onError(String(e)));
    return () => { void invoke('window_action', { action: 'menu-close', instance }).catch(() => {}); };
  }, [inPanel, menu, instance]);
  return <section className={`widget-surface${menu ? ' menu-open' : ''}`}>
    <div className="surface-chrome">
      <button className="drag-handle icon-button" title="拖动组件" aria-label="拖动组件" onPointerDown={onDrag}><Icon name="grip" /></button>
      <button className="icon-button" title="组件菜单" aria-label="组件菜单" aria-expanded={menu} onClick={() => setMenu(!menu)}><Icon name="more" /></button>
    </div>
    {children}
    {menu && <div className="surface-menu"><div className="surface-actions">
      {items.map(item => <button className="icon-button" key={item.label} title={item.label} aria-label={item.label} disabled={item.disabled} onClick={item.action}><Icon name={item.icon} /></button>)}
      <button className="icon-button" title="关闭菜单" aria-label="关闭菜单" onClick={() => setMenu(false)}><Icon name="close" /></button>
    </div>{notice}</div>}
    <button className="resize-handle icon-button" title="调整组件尺寸" aria-label="调整组件尺寸" onPointerDown={onResize}><Icon name="resize" /></button>
    {!menu && notice}
  </section>;
}
