import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
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

const validColor = (color: string | null | undefined): color is string => !!color && /^#[0-9a-f]{6}$/i.test(color);
function colorStyle(color: string): CSSProperties {
  const safe = validColor(color) ? color.toUpperCase() : palette.neutral;
  const rgb = [1, 3, 5].map(offset => parseInt(safe.slice(offset, offset + 2), 16));
  // 固定深色底保证黑白等自定义颜色的文字可读性，底色不受桌面壁纸影响。
  const tint = (amount: number) => `rgb(${rgb.map(value => Math.round(34 * (1 - amount) + value * amount)).join(', ')})`;
  return {
    '--group-color': safe, '--group-bg': tint(.14), '--group-hover': tint(.22), '--group-active': tint(.30),
    '--group-border': `rgb(${rgb.map(value => Math.round(value * .65 + 255 * .35)).join(', ')})`,
  } as CSSProperties;
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

export function PanelGroups({ state, instances, busy, message, onChange }: {
  state: PanelGroupsState; instances: { id: string; location: string }[]; busy: boolean; message: string; onChange: Change;
}) {
  const [managing, setManaging] = useState(false);
  const [name, setName] = useState('');
  const [editing, setEditing] = useState<{ id: string; name: string }>();
  const [removing, setRemoving] = useState<string>();
  const [coloring, setColoring] = useState<{ id: string | null }>();
  const count = (id: string | null) => instances.filter(i => i.location === 'panel' && (state.membership[i.id] ?? null) === id).length;
  const choices = [{ id: null, name: '未分组', autoColor: palette.neutral, color: state.ungroupedColor }, ...state.groups.map((group, index) => ({ ...group, autoColor: validColor(group.autoColor) ? group.autoColor : palette.presets[index % palette.presets.length].color }))];
  const editColor = (id: string | null) => { setColoring({ id }); setEditing(undefined); setRemoving(undefined); };
  const colorEditor = (id: string | null) => {
    const group = choices.find(group => group.id === id)!;
    return coloring?.id === id && <ColorEditor key={id ?? 'ungrouped'} name={group.name} automatic={group.autoColor} saved={group.color ?? null} busy={busy} onCancel={() => setColoring(undefined)} onSave={color => onChange({ action: 'color', group: id, color }).then(ok => { if (ok) setColoring(undefined); return ok; })} />;
  };
  const reorder = (id: string, direction: number) => {
    const ids = state.groups.map(g => g.id), at = ids.indexOf(id);
    [ids[at], ids[at + direction]] = [ids[at + direction], ids[at]];
    void onChange({ action: 'reorder', ids });
  };
  const close = () => { setManaging(false); setEditing(undefined); setRemoving(undefined); setColoring(undefined); };
  return <>
    <div className="panel-group-bar">
      <div className="panel-group-tabs" role="tablist" aria-label="组件分组">
        {choices.map(group => <button key={group.id ?? 'ungrouped'} role="tab" style={colorStyle(group.color ?? group.autoColor)} aria-selected={state.active === group.id} aria-controls="panel-group-content" title={group.name} disabled={busy} onClick={() => { if (state.active !== group.id) void onChange({ action: 'select', group: group.id }); }}>
          <span>{group.name}</span><small>{count(group.id)}</small>
        </button>)}
      </div>
      <button className="icon-button" title="管理分组" aria-label="管理分组" disabled={busy} onClick={() => setManaging(true)}><Icon name="groups" /></button>
    </div>
    {managing && <GroupDialog title="管理分组" busy={busy} message={message} onClose={close}>
      <p>每个组件只属于一个组。“未分组”固定保留，分组顺序可调整。</p>
      <ul className="panel-group-list">
        <li><div className="panel-group-row"><span className="panel-group-name">未分组<small>{count(null)} 个组件</small></span><button disabled={busy} onClick={() => editColor(null)}>配色</button></div>{colorEditor(null)}</li>
        {state.groups.map((group, index) => <li key={group.id}>
          {editing?.id === group.id ? <form className="panel-group-rename" onSubmit={e => { e.preventDefault(); void onChange({ action: 'rename', id: group.id, name: editing.name }).then(ok => { if (ok) setEditing(undefined); }); }}>
            <input aria-label="新的分组名称" autoFocus maxLength={40} required value={editing.name} onChange={e => setEditing({ id: group.id, name: e.target.value })} />
            <button disabled={busy || !editing.name.trim()}>保存</button><button type="button" disabled={busy} onClick={() => setEditing(undefined)}>取消</button>
          </form> : <div className="panel-group-row">
            <span className="panel-group-name" title={group.name}>{group.name}<small>{count(group.id)} 个组件</small></span>
            <button className="icon-button" aria-label={`前移分组 ${group.name}`} title="前移" disabled={busy || index === 0} onClick={() => reorder(group.id, -1)}><Icon name="up" /></button>
            <button className="icon-button" aria-label={`后移分组 ${group.name}`} title="后移" disabled={busy || index === state.groups.length - 1} onClick={() => reorder(group.id, 1)}><Icon name="down" /></button>
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
