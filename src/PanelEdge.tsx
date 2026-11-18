/// <reference types="vite/client" />
import { useRef, type PointerEvent } from 'react';
import { invoke } from './host';
import { Icon } from './Icon';
import paw from './assets/panel-edge/paw.png';
import rocket from './assets/panel-edge/rocket.png';
import basketball from './assets/panel-edge/basketball.png';
import jordan from './assets/panel-edge/jordan.png';
import shinobu from './assets/panel-edge/shinobu.png';

export const edgeStyles = {
  classic: { label: '经典箭头', width: 12, height: 100 },
  frost: { label: '磨砂胶囊', width: 22, height: 58 },
  glow: { label: '青色微光', width: 14, height: 68 },
  paw: { label: '猫爪挂件', width: 42, height: 48, image: paw },
  rocket: { label: '火箭', width: 42, height: 48, image: rocket },
  basketball: { label: '篮球', width: 42, height: 48, image: basketball },
  jordan: { label: '飞人乔丹', width: 42, height: 48, image: jordan },
  shinobu: { label: '蝴蝶忍', width: 42, height: 48, image: shinobu },
};
export type EdgeStyle = keyof typeof edgeStyles;
export type EdgePosition = { side: 'left' | 'right'; offset: number };
export type EdgeVisual = { style: EdgeStyle; position: EdgePosition };
export const defaultEdge: EdgeVisual = { style: 'classic', position: { side: 'right', offset: .5 } };
export function readEdgeStyle(value: string | null): EdgeStyle {
  return value && Object.hasOwn(edgeStyles, value) ? value as EdgeStyle : 'classic';
}

function EdgeArtwork({ style }: { style: EdgeStyle }) {
  const item = edgeStyles[style];
  return <span className="panel-edge-art" data-edge-style={style} aria-hidden="true">
    {'image' in item
      ? <img src={item.image} alt="" draggable={false} />
      : <Icon name="collapse" width="12" height="18" />}
  </span>;
}

export function EdgeStylePicker({ value, onChange }: { value: EdgeStyle; onChange: (style: EdgeStyle) => void }) {
  return <fieldset className="edge-style-picker">
    <legend>吸边挂件</legend>
    <div className="edge-style-options">
      {(Object.keys(edgeStyles) as EdgeStyle[]).map(style => <label key={style}>
        <input type="radio" name="panel-edge-style" value={style} checked={style === value} onChange={() => onChange(style)} />
        <span className="edge-style-preview"><EdgeArtwork style={style} /></span>
        <span>{edgeStyles[style].label}</span>
      </label>)}
    </div>
    <small>按住挂件拖到屏幕左侧或右侧，松开自动保存。</small>
  </fieldset>;
}

export function PanelEdge({ visual, expanded, onToggle, onError }: {
  visual: EdgeVisual; expanded: boolean; onToggle: () => void; onError: (message: string) => void;
}) {
  const gesture = useRef<{ pointer: number; x: number; y: number; moved: boolean; result: Promise<boolean> } | undefined>(undefined);
  const label = expanded ? '收起组件面板' : '展开组件面板';
  const start = (event: PointerEvent<HTMLButtonElement>) => {
    if (!event.isPrimary || event.button !== 0) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    const result = invoke<boolean>('drag_panel_edge').catch(error => {
      onError(`移动挂件失败：${String(error)}`);
      return true;
    });
    gesture.current = { pointer: event.pointerId, x: event.screenX, y: event.screenY, moved: false, result };
  };
  const track = (event: PointerEvent<HTMLButtonElement>) => {
    const press = gesture.current;
    if (press?.pointer === event.pointerId && event.buttons &&
      Math.max(Math.abs(event.screenX - press.x), Math.abs(event.screenY - press.y)) >= 5) {
      press.moved = true;
    }
  };
  return <button
    type="button"
    className={`edge${expanded ? ' panel-edge-toggle' : ''}`}
    data-edge-side={visual.position.side}
    title={`${label} · 按住可拖动`}
    aria-label={label}
    aria-expanded={expanded}
    onPointerDown={start}
    onPointerMove={track}
    onPointerCancel={() => { if (gesture.current) gesture.current.moved = true; }}
    onClick={event => {
      const press = gesture.current;
      if (event.detail === 0 || !press) { onToggle(); return; }
      gesture.current = undefined;
      void press.result.then(dragged => { if (!dragged && !press.moved) onToggle(); });
    }}
  ><EdgeArtwork style={visual.style} /></button>;
}
