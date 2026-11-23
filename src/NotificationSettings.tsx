import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from './host';
import { heldShortcutModifiers, shortcutMainKey } from './shortcut-keys.ts';

type Channel = 'windows' | 'island';
const marketDirections = [['up', '上涨'], ['down', '下跌']] as const;
type MarketDirection = typeof marketDirections[number][0];
type MarketTone = { id: string; label: string; available: boolean };
type IslandControl = 'visibility' | 'mute';
type IslandControls = { hidden: boolean; muted: boolean };
type Shortcut = { value: string; registered: boolean };
type Preferences = Record<Channel, boolean> & {
  calls: boolean;
  marketSound: boolean;
  maxVisible: number;
  opacity: number;
  volume: number;
  controls: IslandControls;
  shortcuts: Record<IslandControl, Shortcut>;
  marketTones: Record<MarketDirection, string>;
  marketToneDefaults: Record<MarketDirection, string>;
  marketToneGroups: { name: string; sounds: MarketTone[] }[];
};
const samples = [
  ['info', '普通信息'], ['success', '成功'], ['warning', '警告'], ['error', '错误'],
  ['reminder', '双按钮提醒'], ['advance-countdown', '倒计时预告'],
  ['countdown', '实时倒计时'], ['call', '模拟来电（静音）'],
] as const;
type Sample = typeof samples[number][0];
type HistoryEntry = { id: string; kind: Sample | 'market-alert'; title: string; message: string; source: string; at?: number; shownAt?: number };
type History = { shown: HistoryEntry[]; pending: HistoryEntry[] };
type Command =
  | { type: 'channel'; channel: Channel; enabled: boolean }
  | { type: 'calls'; enabled: boolean }
  | { type: 'market-sound'; enabled: boolean }
  | { type: 'market-tone'; direction: MarketDirection; id: string }
  | { type: 'max-visible'; count: number }
  | { type: 'opacity'; percent: number }
  | { type: 'volume'; percent: number }
  | { type: 'toggle'; control: IslandControl }
  | { type: 'shortcut'; control: IslandControl; value: string }
  | { type: 'test'; channel: Channel; sample: Sample };

function IslandLevel({ id, label, value, disabled, save }: {
  id: string;
  label: string;
  value: number;
  disabled: boolean;
  save: (percent: number) => Promise<boolean>;
}) {
  const input = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState<number>();
  useEffect(() => {
    const slider = input.current;
    if (!slider) return;
    const commit = () => {
      const percent = slider.valueAsNumber;
      if (percent === value) setDraft(undefined);
      else void save(percent).finally(() => setDraft(undefined));
    };
    // 原生 change 在指针松开时提交，拖动中的 input 只更新百分比读数。
    slider.addEventListener('change', commit);
    return () => slider.removeEventListener('change', commit);
  }, [save, value]);
  return <div className="setting-row island-level">
    <label htmlFor={id}>{label}</label>
    <input ref={input} id={id} type="range" min="0" max="100" value={draft ?? value}
      disabled={disabled} onChange={event => setDraft(event.currentTarget.valueAsNumber)} />
    <output htmlFor={id}>{draft ?? value}%</output>
  </div>;
}

function IslandShortcutField({ control, label, shortcut, disabled, save }: {
  control: IslandControl;
  label: string;
  shortcut: Shortcut;
  disabled: boolean;
  save: (value: string) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState<string>();
  const value = draft ?? shortcut.value;
  return <div className="setting-row">
    <label htmlFor={`island-shortcut-${control}`}>{label}
      {!shortcut.registered && <small>当前快捷键未能注册，可改键或重试。</small>}
    </label>
    <input id={`island-shortcut-${control}`} className="shortcut-capture" readOnly
      aria-describedby="island-shortcut-help" disabled={disabled} value={value.replace(/\bSuper\b/gi, 'Win')}
      onKeyDown={event => {
        event.preventDefault();
        if (event.repeat) return;
        const modifiers = heldShortcutModifiers(event);
        if (!modifiers && event.key === 'Escape') {
          setDraft(undefined);
          event.currentTarget.blur();
          return;
        }
        const key = shortcutMainKey(event);
        if (modifiers && key) setDraft(modifiers + '+' + key);
      }} />
    <button type="button" disabled={disabled || (shortcut.registered && value === shortcut.value)}
      onClick={() => void save(value).then(saved => { if (saved) setDraft(undefined); })}>保存</button>
  </div>;
}

export function NotificationSettings({ historyRequest }: { historyRequest?: { id: string } }) {
  const [preferences, setPreferences] = useState<Preferences>();
  const [controls, setControls] = useState<IslandControls>();
  const [sample, setSample] = useState<Sample>('info');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [history, setHistory] = useState<History>();
  const [historyOpen, setHistoryOpen] = useState(!!historyRequest);
  const [historyError, setHistoryError] = useState('');
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const historyPanel = useRef<HTMLDivElement>(null);
  const selectedEntry = useRef<HTMLLIElement>(null);
  const previewRequest = useRef<AbortController>(null);
  const previewAudio = useRef<HTMLAudioElement>(null);
  const previewVolume = useRef(1);
  const volume = (preferences?.volume ?? 100) / 100;
  const [previewDirection, setPreviewDirection] = useState<MarketDirection>();
  useEffect(() => {
    previewVolume.current = volume;
    if (previewAudio.current) previewAudio.current.volume = volume;
  }, [volume]);
  useEffect(() => {
    let active = true;
    const off = listen<IslandControls>('host:island-controls', ({ payload }) => {
      if (active) setControls(payload);
    });
    off.then(() => invoke<Preferences>('notification_settings', { command: { type: 'get' } })).then(
      value => {
        if (!active) return;
        setPreferences(value);
        setControls(current => current ?? value.controls);
      },
      failure => { if (active) setError(String(failure)); },
    );
    return () => {
      active = false;
      void off.then(unlisten => unlisten()).catch(() => undefined);
      previewRequest.current?.abort();
    };
  }, []);

  function stopPreview() {
    previewRequest.current?.abort();
    previewRequest.current = null;
    setPreviewDirection(undefined);
  }

  async function previewTone(direction: MarketDirection, id: string) {
    const wasPlaying = previewDirection === direction;
    stopPreview();
    if (wasPlaying) return;
    setError('');
    setPreviewDirection(direction);
    const request = new AbortController();
    previewRequest.current = request;
    const failed = (failure: unknown) => {
      if (request.signal.aborted) return;
      setError(`无法试听铃声：${String(failure)}`);
      stopPreview();
    };
    try {
      const encoded = await invoke<string>('notification_settings', { command: { type: 'preview-market-tone', id } });
      if (request.signal.aborted) return;
      const bytes = Uint8Array.from(atob(encoded), value => value.charCodeAt(0));
      const url = URL.createObjectURL(new Blob([bytes], { type: 'audio/mpeg' }));
      const audio = new Audio(url);
      audio.volume = previewVolume.current;
      previewAudio.current = audio;
      request.signal.addEventListener('abort', () => {
        if (previewAudio.current === audio) previewAudio.current = null;
        audio.pause();
        audio.removeAttribute('src');
        URL.revokeObjectURL(url);
      }, { once: true });
      audio.addEventListener('ended', () => { if (!request.signal.aborted) stopPreview(); }, { once: true });
      audio.addEventListener('error', () => failed(audio.error?.message || '无法播放音频'), { once: true });
      await audio.play();
    } catch (failure) {
      failed(failure);
    }
  }

  async function change(command: Command) {
    if (command.type === 'market-tone') stopPreview();
    setBusy(true);
    setError('');
    try {
      setPreferences(await invoke<Preferences>('notification_settings', { command }));
      return true;
    } catch (failure) {
      setError(String(failure));
      return false;
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (historyRequest) setHistoryOpen(true);
  }, [historyRequest]);
  useEffect(() => {
    if (!historyOpen) return;
    let active = true;
    setHistory(undefined);
    setHistoryError('');
    invoke<History>('notification_settings', { command: { type: 'history' } }).then(
      value => { if (active) setHistory(value); },
      failure => { if (active) setHistoryError(String(failure)); },
    );
    return () => { active = false; };
  }, [historyOpen, historyRefresh, historyRequest]);
  useEffect(() => {
    if (historyOpen && historyRequest && (history || historyError)) {
      (selectedEntry.current ?? historyPanel.current)?.scrollIntoView({ block: 'nearest' });
    }
  }, [historyOpen, historyRequest, history, historyError]);

  function entries(values: HistoryEntry[]) {
    return <ul className="notification-history">
      {values.map(entry => {
        const selected = entry.id === historyRequest?.id;
        const date = new Date(entry.at ?? entry.shownAt ?? 0);
        return <li key={entry.id} ref={selected ? selectedEntry : undefined} aria-current={selected || undefined}>
          <div><strong>{entry.title || entry.message}</strong><time dateTime={date.toISOString()}>{date.toLocaleString()}</time></div>
          {entry.title && entry.message && <p>{entry.message}</p>}
          <small>{entry.source} · {entry.kind === 'market-alert' ? '行情通知' : entry.kind === 'call' ? '来电' : samples.find(([kind]) => kind === entry.kind)?.[1]}</small>
        </li>;
      })}
    </ul>;
  }

  return <section className="settings-panel">
    <h2>通知</h2>
    <div className="setting-row">
      <label className="check"><input type="checkbox" checked={preferences?.windows ?? true} disabled={!preferences || busy}
        onChange={event => void change({ type: 'channel', channel: 'windows', enabled: event.target.checked })} />Windows 系统通知</label>
      <button type="button" disabled={!preferences?.windows || busy}
        onClick={() => void change({ type: 'test', channel: 'windows', sample: 'info' })}>测试系统通知</button>
    </div>
    <label className="check"><input type="checkbox" checked={preferences?.island ?? true} disabled={!preferences || busy}
      onChange={event => void change({ type: 'channel', channel: 'island', enabled: event.target.checked })} />灵动通知</label>
    <IslandLevel id="island-opacity" label="灵动通知透明度"
      value={100 - (preferences?.opacity ?? 100)} disabled={!preferences || busy}
      save={percent => change({ type: 'opacity', percent: 100 - percent })} />
    <IslandLevel id="island-volume" label="灵动通知音量"
      value={preferences?.volume ?? 100} disabled={!preferences || busy}
      save={percent => change({ type: 'volume', percent })} />
    <div className="setting-row">
      <button type="button" disabled={!controls || busy} onClick={() => void change({ type: 'toggle', control: 'visibility' })}>
        {controls?.hidden ? '显示灵动通知' : '隐藏灵动通知'}
      </button>
      <button type="button" disabled={!controls || busy} onClick={() => void change({ type: 'toggle', control: 'mute' })}>
        {controls?.muted ? '解除灵动通知静音' : '静音灵动通知'}
      </button>
    </div>
    {preferences && <>
      <p id="island-shortcut-help" className="muted">点击输入框，录入带 Ctrl、Alt、Shift 或 Win 的组合键。按 Esc 取消。</p>
      <IslandShortcutField control="visibility" label="灵动通知显隐快捷键" shortcut={preferences.shortcuts.visibility}
        disabled={busy} save={value => change({ type: 'shortcut', control: 'visibility', value })} />
      <IslandShortcutField control="mute" label="灵动通知静音快捷键" shortcut={preferences.shortcuts.mute}
        disabled={busy} save={value => change({ type: 'shortcut', control: 'mute', value })} />
    </>}
    <div className="setting-row">
      <label htmlFor="island-max-visible">灵动通知最大可见条数</label>
      <select id="island-max-visible" value={preferences?.maxVisible ?? 9} disabled={!preferences || busy}
        onChange={event => void change({ type: 'max-visible', count: Number(event.target.value) })}>
        {Array.from({ length: 20 }, (_, index) => index + 1).map(count => <option key={count} value={count}>{count} 条</option>)}
      </select>
    </div>
    <label className="check"><input type="checkbox" checked={preferences?.marketSound ?? true} disabled={!preferences || busy}
      onChange={event => void change({ type: 'market-sound', enabled: event.target.checked })} />行情通知铃声</label>
    {preferences && marketDirections.map(([direction, label]) => {
      const selected = preferences.marketTones[direction];
      const canPreview = preferences.marketToneGroups.some(group => group.sounds.some(sound => sound.id === selected && sound.available));
      const previewing = previewDirection === direction;
      const field = `market-tone-${direction}`;
      return <div className="setting-row notification-tone" key={direction}>
        <label htmlFor={field}>{label}铃声</label>
        <select id={field} value={selected} disabled={busy}
          onChange={event => void change({ type: 'market-tone', direction, id: event.target.value })}>
          {preferences.marketToneGroups.map(group => <optgroup label={group.name} key={group.name}>
            {group.sounds.map(sound => <option value={sound.id} key={sound.id} disabled={!sound.available}>
              {sound.label}{sound.id === preferences.marketToneDefaults[direction] ? '（默认）' : ''}{sound.available ? '' : '（文件缺失）'}
            </option>)}
          </optgroup>)}
        </select>
        <button type="button" disabled={!canPreview || busy} aria-label={`${previewing ? '停止试听' : '试听'}${label}铃声`}
          onClick={() => void previewTone(direction, selected)}>{previewing ? '停止' : '试听'}</button>
      </div>;
    })}
    <label className="check"><input type="checkbox" checked={preferences?.calls ?? false} disabled={!preferences || busy}
      onChange={event => void change({ type: 'calls', enabled: event.target.checked })} />允许小组件触发模拟来电</label>
    <div className="setting-row">
      <label htmlFor="notification-sample">灵动通知样式</label>
      <select id="notification-sample" value={sample} onChange={event => setSample(event.target.value as Sample)}>
        {samples.map(([id, label]) => <option value={id} key={id}>{label}</option>)}
      </select>
      <button type="button" disabled={!preferences?.island || busy}
        onClick={() => void change({ type: 'test', channel: 'island', sample })}>测试灵动通知</button>
    </div>
    <div className="setting-row">
      <button type="button" aria-expanded={historyOpen} onClick={() => setHistoryOpen(!historyOpen)}>{historyOpen ? '收起灵动通知历史' : '查看灵动通知历史'}</button>
      {historyOpen && <button type="button" onClick={() => setHistoryRefresh(value => value + 1)}>刷新记录</button>}
    </div>
    {historyOpen && <div ref={historyPanel}>{historyError
      ? <p role="alert" className="notice error">{historyError}</p>
      : history === undefined ? <p role="status">正在读取历史</p>
      : <>
        {(history.pending.length > 0 || historyRequest?.id === 'pending') && <>
          <h3>排队中 · {history.pending.length} 条</h3>
          {history.pending.length > 0 ? entries(history.pending) : <p className="muted">当前没有排队通知</p>}
        </>}
        <h3>已送达</h3>
        {history.shown.length > 0 ? entries(history.shown) : <p className="muted">暂无灵动通知记录</p>}
      </>}
    </div>}
    {error && <p role="alert" className="notice error">{error}</p>}
  </section>;
}
