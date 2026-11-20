import { useCallback, useEffect, useRef, useState, type CSSProperties, type PointerEvent, type ReactNode } from 'react';
import { Icon } from './Icon';
import palette from './panel-colors.json';

export interface PanelGroupsState {
  groups: { id: string; name: string; autoColor?: string; color?: string | null }[];
  active: string | null;
  membership: Record<string, string>;
  ungroupedColor?: string | null;
}
export type GroupChange =
  | { action: 'create'; name: string }
  | { action: 'rename'; id: string; name: string }
  | { action: 'color'; group: string | null; color: string | null }
  | { action: 'remove'; id: string }
  | { action: 'reorder'; ids: string[] }
  | { action: 'select'; group: string | null }
  | { action: 'move'; instance: string; group: string | null };
type Change = (change: GroupChange) => Promise<boolean>;
export const ungrouped: PanelGroupsState = { groups: [], active: null, membership: {} };

interface GroupDrag {
  id: string;
  pointer: number;
  startX: number;
  startY: number;
  grabX: number;
  grabY: number;
  width: number;
  distance: number;
  x: number;
  y: number;
  moved: boolean;
  frame?: number;
}

interface GroupPreview {
  id: string;
  index?: number;
  left: number;
  top: number;
  width: number;
  distance: number;
  settling?: boolean;
}

function dropIndex(strip: HTMLDivElement, drag: GroupDrag, x: number, y: number) {
  const bounds = strip.getBoundingClientRect();
  if (x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom) return;
  const center = x - drag.grabX + drag.width / 2 - bounds.left + strip.scrollLeft;
  let index = 0;
  for (const tab of strip.querySelectorAll<HTMLElement>('[data-panel-group-id]')) {
    if (tab.dataset.panelGroupId === drag.id) continue;
    // 排序按未平移的布局判断，避免相邻标签让位后反复改变目标位置。
    if (center < tab.offsetLeft + tab.offsetWidth / 2) break;
    index++;
  }
  return index;
}

export function panelGroupDropAt(x: number, y: number) {
  // 指针捕获会把事件发给拖动手柄，按实际位置查找可见的分组标签。
  const hit = document.elementFromPoint(x, y);
  const strip = hit?.closest<HTMLDivElement>('.panel-group-tabs');
  if (!strip) return;
  const tab = hit?.closest<HTMLButtonElement>('button.panel-group-tab');
  const group = tab && !tab.disabled && !strip.hasAttribute('data-reordering')
    ? tab.dataset.panelGroupId ?? null
    : undefined;
  return { strip, group };
}

export function scrollGroupTabs(strip: HTMLDivElement, x: number) {
  const { left, right, width } = strip.getBoundingClientRect();
  const edge = Math.min(24, width / 3);
  const step = x < left + edge ? -6 : x > right - edge ? 6 : 0;
  if (!step) return false;
  const before = strip.scrollLeft;
  strip.scrollLeft += step;
  return strip.scrollLeft !== before;
}

const validColor = (color: string | null | undefined): color is string => !!color && /^#[0-9a-f]{6}$/i.test(color);
function colorStyle(color: string): CSSProperties {
  const safe = validColor(color) ? color.toUpperCase() : palette.neutral;
  return { '--group-color': safe } as CSSProperties;
}

function ColorEditor({ name, automatic, saved, busy, onSave, onCancel }: {
  name: string; automatic: string; saved: string | null; busy: boolean;
  onSave: (color: string | null) => Promise<boolean>; onCancel: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(saved);
  const current = draft ?? automatic;
  return <form className="panel-color-editor" aria-label={`设置${name}颜色`} onSubmit={e => { e.preventDefault(); if (draft === null || validColor(draft)) void onSave(draft); }}>
    <div className="panel-color-preview" style={colorStyle(current)}><span className="panel-color-label">{name}</span><small>{draft === null ? '自动配色' : '自定义颜色'}</small></div>
    <div className="panel-color-presets" role="group" aria-label="iOS风格预设颜色">
      <button type="button" className="panel-color-auto" aria-pressed={draft === null} disabled={busy} onClick={() => setDraft(null)}>自动配色</button>
      {palette.presets.map(preset => <button type="button" key={preset.color} className="panel-color-swatch" style={{ '--swatch': preset.color } as CSSProperties} title={preset.name} aria-label={`使用${preset.name}`} aria-pressed={draft?.toUpperCase() === preset.color} disabled={busy} onClick={() => setDraft(preset.color)} />)}
    </div>
    <div className="panel-color-custom"><label>自定义<input type="color" aria-label="自定义分组颜色" disabled={busy} value={validColor(current) ? current : automatic} onChange={e => setDraft(e.target.value)} /></label><input aria-label="十六进制颜色" maxLength={7} pattern="#[0-9A-Fa-f]{6}" required disabled={busy} value={current} onChange={e => setDraft(e.target.value)} /></div>
    <div className="actions"><button type="button" disabled={busy} onClick={onCancel}>取消</button><button disabled={busy || draft !== null && !validColor(draft)}>保存颜色</button></div>
  </form>;
}

function GroupDialog({ title, busy, message, onClose, children }: {
  title: string; busy: boolean; message: string; onClose: () => void; children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const node = dialog.current!;
    node.showModal();
    return () => node.close();
  }, []);
  return <dialog ref={dialog} className="panel-group-dialog" aria-label={title} onCancel={e => { e.preventDefault(); if (!busy) onClose(); }}>
    <header><h2>{title}</h2><button className="icon-button" aria-label="关闭分组窗口" title="关闭" disabled={busy} onClick={onClose}><Icon name="close" /></button></header>
    {children}
    {message && <p role="alert" className="error">{message}</p>}
  </dialog>;
}

export function PanelGroups({ state, instances, busy, message, onChange, dropGroup }: {
  state: PanelGroupsState; instances: { id: string; location: string }[]; busy: boolean; message: string; onChange: Change;
  dropGroup?: string | null;
}) {
  const [managing, setManaging] = useState(false);
  const [name, setName] = useState('');
  const [editing, setEditing] = useState<{ id: string; name: string }>();
  const [removing, setRemoving] = useState<string>();
  const [coloring, setColoring] = useState<{ id: string | null }>();
  const tabs = useRef<HTMLDivElement>(null);
  const drag = useRef<GroupDrag | null>(null);
  const suppressClick = useRef(false);
  const [preview, setPreview] = useState<GroupPreview>();
  const stopDrag = useCallback(() => {
    if (!drag.current) return;
    if (drag.current.frame !== undefined) window.cancelAnimationFrame(drag.current.frame);
    drag.current = null;
    setPreview(undefined);
  }, []);
  useEffect(() => {
    window.addEventListener('blur', stopDrag);
    return () => {
      window.removeEventListener('blur', stopDrag);
      if (drag.current?.frame !== undefined) window.cancelAnimationFrame(drag.current.frame);
    };
  }, [stopDrag]);

  const reorder = (id: string, index: number) => {
    if (busy || index < 0 || index >= state.groups.length) return;
    const ids = state.groups.map(group => group.id);
    const from = ids.indexOf(id);
    if (from < 0 || index === from) return;
    ids.splice(from, 1);
    ids.splice(index, 0, id);
    return onChange({ action: 'reorder', ids });
  };
  const startDrag = (event: PointerEvent<HTMLButtonElement>, id: string | null) => {
    const strip = tabs.current;
    if (!strip || id === null || busy || preview || state.groups.length < 2 || !event.isPrimary || event.button !== 0 || drag.current) return;
    const { clientX: x, clientY: y, pointerId: pointer } = event;
    const { left, top, width } = event.currentTarget.getBoundingClientRect();
    const gap = parseFloat(window.getComputedStyle(strip).columnGap) || 0;
    event.currentTarget.setPointerCapture(pointer);
    drag.current = { id, pointer, startX: x, startY: y, grabX: x - left, grabY: y - top, width, distance: width + gap, x, y, moved: false };
  };
  const updatePreview = (current: GroupDrag) => {
    current.frame = undefined;
    const strip = tabs.current;
    if (!strip || drag.current !== current) return;
    const index = dropIndex(strip, current, current.x, current.y);
    setPreview({ id: current.id, index, left: current.x - current.grabX, top: current.y - current.grabY, width: current.width, distance: current.distance });
    if (index !== undefined && scrollGroupTabs(strip, current.x)) {
      current.frame = window.requestAnimationFrame(() => updatePreview(current));
    }
  };
  const moveDrag = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    current.x = event.clientX;
    current.y = event.clientY;
    if (!current.moved && Math.hypot(current.x - current.startX, current.y - current.startY) < 5) return;
    current.moved = true;
    suppressClick.current = true;
    if (current.frame === undefined) updatePreview(current);
  };
  const finishDrag = (event: PointerEvent<HTMLDivElement>, canceled = false) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    stopDrag();
    if (canceled || !current.moved) return;
    const strip = event.currentTarget;
    const index = dropIndex(strip, current, event.clientX, event.clientY);
    if (index === undefined) return;
    const target = strip.querySelectorAll<HTMLElement>('[data-panel-group-id]')[index];
    if (!target) return;
    const saving = reorder(current.id, index);
    if (!saving) return;
    const from = state.groups.findIndex(group => group.id === current.id);
    const left = strip.getBoundingClientRect().left + target.offsetLeft - strip.scrollLeft;
    const top = target.getBoundingClientRect().top;
    const rightOffset = index > from ? target.offsetWidth - current.width : 0;
    setPreview({ id: current.id, index, left: left + rightOffset, top, width: current.width, distance: current.distance, settling: true });
    void saving.finally(() => setPreview(undefined));
  };

  const count = (id: string | null) => instances.filter(i => i.location === 'panel' && (state.membership[i.id] ?? null) === id).length;
  const choices = [{ id: null, name: '未分组', autoColor: palette.neutral, color: state.ungroupedColor }, ...state.groups.map((group, index) => ({ ...group, autoColor: validColor(group.autoColor) ? group.autoColor : palette.presets[index % palette.presets.length].color }))];
  const editColor = (id: string | null) => { setColoring({ id }); setEditing(undefined); setRemoving(undefined); };
  const colorEditor = (id: string | null) => {
    const group = choices.find(group => group.id === id)!;
    return coloring?.id === id && <ColorEditor key={id ?? 'ungrouped'} name={group.name} automatic={group.autoColor} saved={group.color ?? null} busy={busy} onCancel={() => setColoring(undefined)} onSave={color => onChange({ action: 'color', group: id, color }).then(ok => { if (ok) setColoring(undefined); return ok; })} />;
  };
  const close = () => { setManaging(false); setEditing(undefined); setRemoving(undefined); setColoring(undefined); };
  const fromIndex = state.groups.findIndex(group => group.id === preview?.id);
  const toIndex = preview?.index ?? fromIndex;
  const distance = preview?.distance ?? 0;
  const floating = preview && choices.find(group => group.id === preview.id);
  return <>
    <div className="panel-group-bar">
      <div ref={tabs} className="panel-group-tabs" role="tablist" aria-label="组件分组" data-reordering={(preview && (!preview.settling || fromIndex !== toIndex)) || undefined} onPointerDownCapture={e => { if (e.button === 0 && !drag.current) suppressClick.current = false; }}
        onPointerMove={moveDrag} onPointerUp={e => finishDrag(e)} onPointerCancel={e => finishDrag(e, true)} onLostPointerCapture={e => finishDrag(e, true)}
        onKeyDown={e => { if (e.key === 'Escape' && drag.current) { e.preventDefault(); stopDrag(); } }}
        onClickCapture={e => { if (suppressClick.current && e.detail > 0) { suppressClick.current = false; e.stopPropagation(); } }}>
        {choices.map((group, index) => {
          const rank = index - 1;
          const shift = fromIndex < rank && rank <= toIndex ? -distance : toIndex <= rank && rank < fromIndex ? distance : 0;
          return <button key={group.id ?? 'ungrouped'} className="panel-group-tab" role="tab" style={{ ...colorStyle(group.color ?? group.autoColor), transform: shift ? `translateX(${shift}px)` : undefined }} aria-selected={state.active === group.id} aria-controls="panel-group-content" title={group.name} disabled={busy}
            data-panel-group-id={group.id ?? undefined} data-sortable={(group.id !== null && state.groups.length > 1 && !busy) || undefined}
            data-dragging={preview?.id === group.id || undefined} data-drop-target={(!busy && !preview && dropGroup === group.id) || undefined}
            onPointerDown={e => startDrag(e, group.id)} onClick={() => { if (state.active !== group.id) void onChange({ action: 'select', group: group.id }); }}>
            <span>{group.name}</span><small>{count(group.id)}</small>
          </button>;
        })}
      </div>
      {preview && floating && <div className="panel-group-tab panel-group-floating" aria-hidden="true" data-settling={preview.settling} data-active={state.active === floating.id} style={{
        ...colorStyle(floating.color ?? floating.autoColor), width: preview.width, transform: `translate3d(${preview.left}px, ${preview.top}px, 0)`,
      }}><span>{floating.name}</span><small>{count(floating.id)}</small></div>}
      <button className="icon-button" title="管理分组" aria-label="管理分组" disabled={busy} onClick={() => setManaging(true)}><Icon name="groups" /></button>
    </div>
    {managing && <GroupDialog title="管理分组" busy={busy} message={message} onClose={close}>
      <p>每个组件只属于一个组。拖动顶部标签或使用前移、后移按钮排序。“未分组”固定在首位。</p>
      <ul className="panel-group-list">
        <li><div className="panel-group-row"><span className="panel-group-name">未分组<small>{count(null)} 个组件</small></span><button disabled={busy} onClick={() => editColor(null)}>配色</button></div>{colorEditor(null)}</li>
        {state.groups.map((group, index) => <li key={group.id}>
          {editing?.id === group.id ? <form className="panel-group-rename" onSubmit={e => { e.preventDefault(); void onChange({ action: 'rename', id: group.id, name: editing.name }).then(ok => { if (ok) setEditing(undefined); }); }}>
            <input aria-label="新的分组名称" autoFocus maxLength={40} required value={editing.name} onChange={e => setEditing({ id: group.id, name: e.target.value })} />
            <button disabled={busy || !editing.name.trim()}>保存</button><button type="button" disabled={busy} onClick={() => setEditing(undefined)}>取消</button>
          </form> : <div className="panel-group-row">
            <span className="panel-group-name" title={group.name}>{group.name}<small>{count(group.id)} 个组件</small></span>
            <button className="icon-button" aria-label={`前移分组 ${group.name}`} title="前移" disabled={busy || index === 0} onClick={() => void reorder(group.id, index - 1)}><Icon name="up" /></button>
            <button className="icon-button" aria-label={`后移分组 ${group.name}`} title="后移" disabled={busy || index === state.groups.length - 1} onClick={() => void reorder(group.id, index + 1)}><Icon name="down" /></button>
            <button disabled={busy} onClick={() => editColor(group.id)}>配色</button>
            <button disabled={busy} onClick={() => { setEditing({ id: group.id, name: group.name }); setRemoving(undefined); setColoring(undefined); }}>改名</button>
            <button className="danger" disabled={busy} onClick={() => { setRemoving(group.id); setEditing(undefined); setColoring(undefined); }}>删除</button>
          </div>}
          {colorEditor(group.id)}
          {removing === group.id && <div className="panel-group-remove"><p>删除“{group.name}”？组内组件将回到“未分组”，配置和内容保留。</p><div className="actions"><button disabled={busy} onClick={() => setRemoving(undefined)}>取消</button><button className="danger" disabled={busy} onClick={() => void onChange({ action: 'remove', id: group.id }).then(ok => { if (ok) setRemoving(undefined); })}>确认删除分组</button></div></div>}
        </li>)}
      </ul>
      <form className="panel-group-create" onSubmit={e => { e.preventDefault(); void onChange({ action: 'create', name }).then(ok => { if (ok) setName(''); }); }}>
        <label htmlFor="panel-group-name">新分组</label><div><input id="panel-group-name" placeholder="例如：工作、行情、生活" maxLength={40} required value={name} onChange={e => setName(e.target.value)} /><button disabled={busy || !name.trim()}><Icon name="plus" width="16" height="16" />新建</button></div>
      </form>
    </GroupDialog>}
  </>;
}

export function MovePanelGroupDialog({ instance, name, state, busy, message, onChange, onClose }: {
  instance: string; name: string; state: PanelGroupsState; busy: boolean; message: string; onChange: Change; onClose: () => void;
}) {
  const [group, setGroup] = useState(state.membership[instance] ?? '');
  return <GroupDialog title="移动组件到分组" busy={busy} message={message} onClose={onClose}>
    <form className="panel-group-move" onSubmit={e => { e.preventDefault(); void onChange({ action: 'move', instance, group: group || null }).then(ok => { if (ok) onClose(); }); }}>
      <label htmlFor="panel-group-destination">将“{name}”移动到</label>
      <select id="panel-group-destination" autoFocus value={group} onChange={e => setGroup(e.target.value)}>
        <option value="">未分组</option>{state.groups.map(g => <option key={g.id} value={g.id}>{g.name}</option>)}
      </select>
      <div className="actions"><button type="button" disabled={busy} onClick={onClose}>取消</button><button disabled={busy}>移动</button></div>
    </form>
  </GroupDialog>;
}
