import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Bell, BellRing, Blend, Eye, List, Monitor, Phone, Volume2, VolumeX, type LucideIcon } from 'lucide-react';
import { invoke } from './host';
import { SettingsGroup, SettingsRow, SettingsSwitch } from './SettingsLayout';
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
  ['call-sound', '模拟来电（播放声音）'], ['market-up', '行情上涨'], ['market-down', '行情下跌'],
] as const;
type Sample = typeof samples[number][0];
type HistoryEntry = { id: string; kind: Sample | 'market-alert'; title: string; message: string; source: string; at?: number; shownAt?: number; triggeredAt?: number };
type History = { shown: HistoryEntry[]; pending: HistoryEntry[]; retentionDays: number };
type HistoryCommand = { type: 'history-retention'; days: number } | { type: 'clear-history' };
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

function IslandLevel({ id, label, icon, value, disabled, save }: {
  id: string;
  label: string;
  icon: LucideIcon;
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
  return <SettingsRow title={label} icon={icon}>
    <input ref={input} id={id} type="range" min="0" max="100" aria-label={label} value={draft ?? value}
      disabled={disabled} onChange={event => setDraft(event.currentTarget.valueAsNumber)} />
    <output htmlFor={id}>{draft ?? value}%</output>
  </SettingsRow>;
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
        if (event.code === 'Tab' && !event.ctrlKey && !event.altKey && !event.metaKey) return;
        if (event.nativeEvent.isComposing || event.key === 'Process') return;
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

export function NotificationSettings({ shortcutsOnly, historyRequest }: { shortcutsOnly: boolean; historyRequest?: { id: string } }) {
  const [preferences, setPreferences] = useState<Preferences>();
  const [controls, setControls] = useState<IslandControls>();
  const [sample, setSample] = useState<Sample>('info');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [history, setHistory] = useState<History>();
  const [historyOpen, setHistoryOpen] = useState(!!historyRequest);
  const [historyError, setHistoryError] = useState('');
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const [retentionDraft, setRetentionDraft] = useState<string>();
  const [historyBusy, setHistoryBusy] = useState(false);
  const [confirmHistoryClear, setConfirmHistoryClear] = useState(false);
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
    if (shortcutsOnly || !historyOpen) return;
    let active = true;
    setHistory(undefined);
    setHistoryError('');
    invoke<History>('notification_settings', { command: { type: 'history' } }).then(
      value => { if (active) setHistory(value); },
      failure => { if (active) setHistoryError(String(failure)); },
    );
    return () => { active = false; };
  }, [shortcutsOnly, historyOpen, historyRefresh, historyRequest]);
  useEffect(() => {
    if (historyOpen && historyRequest && (history || historyError)) {
      (selectedEntry.current ?? historyPanel.current)?.scrollIntoView({ block: 'nearest' });
    }
  }, [historyOpen, historyRequest, history, historyError]);
  useEffect(() => {
    if (shortcutsOnly) {
      previewRequest.current?.abort();
      setPreviewDirection(undefined);
    }
  }, [shortcutsOnly]);

  async function changeHistory(command: HistoryCommand) {
    setHistoryBusy(true);
    setHistoryError('');
    try {
      setHistory(await invoke<History>('notification_settings', { command }));
      if (command.type === 'history-retention') setRetentionDraft(undefined);
      setConfirmHistoryClear(false);
    } catch (failure) {
      setHistoryError(String(failure));
    } finally {
      setHistoryBusy(false);
    }
  }

  const historyDays = Number(retentionDraft ?? history?.retentionDays);
  const validHistoryDays = Number.isInteger(historyDays) && historyDays >= 1 && historyDays <= 4294967295;

  function entries(values: HistoryEntry[]) {
    return <ul className="notification-history">
      {values.map(entry => {
        const selected = entry.id === historyRequest?.id;
        const date = new Date(entry.at ?? entry.shownAt ?? 0);
        return <li key={entry.id} ref={selected ? selectedEntry : undefined} aria-current={selected || undefined}>
          <div><strong>{entry.title || entry.message}</strong><time dateTime={date.toISOString()}>{date.toLocaleString()}</time></div>
          {entry.title && entry.message && <p>{entry.message}</p>}
          {entry.triggeredAt !== undefined && <p>重启补报 · 触发于 {new Date(entry.triggeredAt).toLocaleString('zh-CN', { hour12: false, fractionalSecondDigits: 3, second: '2-digit', minute: '2-digit', hour: '2-digit', year: 'numeric', month: '2-digit', day: '2-digit' })}</p>}
          <small>{entry.source} · {entry.kind === 'market-alert' ? '行情通知' : entry.kind === 'call' ? '来电' : samples.find(([kind]) => kind === entry.kind)?.[1]}</small>
        </li>;
      })}
    </ul>;
  }

  const feedback = error && <p role="alert" className="notice error">{error}</p>;
  if (shortcutsOnly) return <section className="settings-panel" id="notification-shortcuts" tabIndex={-1}>
    <h2>灵动通知快捷键</h2>
    {feedback}
    {preferences ? <>
      <p id="island-shortcut-help" className="muted">点击输入框，录入带 Ctrl、Alt、Shift 或 Win 的组合键。按 Esc 取消。</p>
      <IslandShortcutField control="visibility" label="灵动通知显隐快捷键" shortcut={preferences.shortcuts.visibility}
        disabled={busy} save={value => change({ type: 'shortcut', control: 'visibility', value })} />
      <IslandShortcutField control="mute" label="灵动通知静音快捷键" shortcut={preferences.shortcuts.mute}
        disabled={busy} save={value => change({ type: 'shortcut', control: 'mute', value })} />
    </> : !error && <p>正在读取灵动通知快捷键…</p>}
  </section>;

  return <>
    {feedback}
    <SettingsGroup id="notification-channels" title="通知渠道">
      <SettingsRow title="Windows 系统通知" icon={Monitor} description="在 Windows 通知中心接收消息。">
        <button type="button" disabled={!preferences?.windows || busy}
          onClick={() => void change({ type: 'test', channel: 'windows', sample: 'info' })}>测试系统通知</button>
        <SettingsSwitch label="Windows 系统通知" checked={preferences?.windows ?? true} disabled={!preferences || busy}
          onChange={enabled => void change({ type: 'channel', channel: 'windows', enabled })} />
      </SettingsRow>
      <SettingsRow title="灵动通知" icon={Bell} description="在桌面显示 SoulDesk 通知卡片。">
        <SettingsSwitch label="灵动通知" checked={preferences?.island ?? true} disabled={!preferences || busy}
          onChange={enabled => void change({ type: 'channel', channel: 'island', enabled })} />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup id="notification-island" title="灵动通知">
      <SettingsRow title="显示通知卡片" icon={Eye} description="隐藏卡片后，声音和通知队列继续运行。">
        <SettingsSwitch label="显示通知卡片" checked={controls?.hidden === false} disabled={!controls || busy}
          onChange={() => void change({ type: 'toggle', control: 'visibility' })} />
      </SettingsRow>
      <IslandLevel id="island-opacity" label="灵动通知透明度" icon={Blend}
        value={100 - (preferences?.opacity ?? 100)} disabled={!preferences || busy}
        save={percent => change({ type: 'opacity', percent: 100 - percent })} />
      <IslandLevel id="island-volume" label="灵动通知音量" icon={Volume2}
        value={preferences?.volume ?? 100} disabled={!preferences || busy}
        save={percent => change({ type: 'volume', percent })} />
      <SettingsRow title="静音" icon={VolumeX} description="同时静音行情铃声和模拟来电。">
        <SettingsSwitch label="灵动通知静音" checked={controls?.muted ?? false} disabled={!controls || busy}
          onChange={() => void change({ type: 'toggle', control: 'mute' })} />
      </SettingsRow>
      <SettingsRow title="最大可见条数" icon={List} description="超过上限的通知进入队列。">
        <select id="island-max-visible" aria-label="灵动通知最大可见条数" value={preferences?.maxVisible ?? 9} disabled={!preferences || busy}
          onChange={event => void change({ type: 'max-visible', count: Number(event.target.value) })}>
          {Array.from({ length: 20 }, (_, index) => index + 1).map(count => <option key={count} value={count}>{count} 条</option>)}
        </select>
      </SettingsRow>
      <SettingsRow title="允许模拟来电" icon={Phone} description="允许支持此能力的小组件触发来电。">
        <SettingsSwitch label="允许小组件触发模拟来电" checked={preferences?.calls ?? false} disabled={!preferences || busy}
          onChange={enabled => void change({ type: 'calls', enabled })} />
      </SettingsRow>
      <SettingsRow title="测试灵动通知" icon={BellRing} description="预览提醒、行情和模拟来电等通知。">
        <select id="notification-sample" aria-label="灵动通知样式" value={sample} onChange={event => setSample(event.target.value as Sample)}>
          {samples.map(([id, label]) => <option value={id} key={id}>{label}</option>)}
        </select>
        <button type="button" disabled={!preferences?.island || busy}
          onClick={() => void change({ type: 'test', channel: 'island', sample })}>测试灵动通知</button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup id="notification-sounds" title="行情铃声">
      <SettingsRow title="行情通知铃声" icon={Volume2} description="控制行情上涨和下跌通知的铃声。">
        <SettingsSwitch label="行情通知铃声" checked={preferences?.marketSound ?? true} disabled={!preferences || busy}
          onChange={enabled => void change({ type: 'market-sound', enabled })} />
      </SettingsRow>
      {preferences && marketDirections.map(([direction, label]) => {
      const selected = preferences.marketTones[direction];
      const canPreview = preferences.marketToneGroups.some(group => group.sounds.some(sound => sound.id === selected && sound.available));
      const previewing = previewDirection === direction;
      const field = `market-tone-${direction}`;
      return <SettingsRow title={`${label}铃声`} icon={Volume2} key={direction}>
        <select id={field} className="notification-tone-select" aria-label={`${label}铃声`} value={selected} disabled={busy}
          onChange={event => void change({ type: 'market-tone', direction, id: event.target.value })}>
          {preferences.marketToneGroups.map(group => <optgroup label={group.name} key={group.name}>
            {group.sounds.map(sound => <option value={sound.id} key={sound.id} disabled={!sound.available}>
              {sound.label}{sound.id === preferences.marketToneDefaults[direction] ? '（默认）' : ''}{sound.available ? '' : '（文件缺失）'}
            </option>)}
          </optgroup>)}
        </select>
        <button type="button" disabled={!canPreview || busy} aria-label={`${previewing ? '停止试听' : '试听'}${label}铃声`}
          onClick={() => void previewTone(direction, selected)}>{previewing ? '停止' : '试听'}</button>
      </SettingsRow>;
      })}
    </SettingsGroup>
    <section className="settings-panel" id="notification-history" tabIndex={-1}>
    <h2>灵动通知历史</h2>
    <div className="setting-row">
      <button type="button" disabled={historyBusy} aria-expanded={historyOpen} onClick={() => {
        setConfirmHistoryClear(false);
        setHistoryOpen(!historyOpen);
      }}>{historyOpen ? '收起灵动通知历史' : '查看灵动通知历史'}</button>
      {historyOpen && <button type="button" disabled={historyBusy} onClick={() => setHistoryRefresh(value => value + 1)}>刷新记录</button>}
    </div>
    {historyOpen && <div ref={historyPanel}>
      {historyError && <p role="alert" className="notice error">{historyError}</p>}
      {!history && !historyError && <p role="status">正在读取历史</p>}
      {history && <>
        <form className="setting-row" onSubmit={event => {
          event.preventDefault();
          if (validHistoryDays && !historyBusy && historyDays !== history.retentionDays) {
            void changeHistory({ type: 'history-retention', days: historyDays });
          }
        }}>
          <label htmlFor="island-history-days">保留天数</label>
          <input id="island-history-days" type="number" min="1" max="4294967295" step="1" required
            disabled={historyBusy} value={retentionDraft ?? String(history.retentionDays)}
            onChange={event => setRetentionDraft(event.target.value)} />
          <button type="submit" disabled={historyBusy || !validHistoryDays || historyDays === history.retentionDays}>保存保留天数</button>
        </form>
        <div className="setting-row">
          <span>已显示历史最多保留 1000 条</span>
          {!confirmHistoryClear && <button type="button" disabled={historyBusy || !history.shown.length}
            onClick={() => setConfirmHistoryClear(true)}>清空已显示历史</button>}
        </div>
        {confirmHistoryClear && <div className="setting-row" role="group" aria-label="清空历史确认">
          <span>清空所有已显示的灵动通知历史？</span>
          <button type="button" disabled={historyBusy} onClick={() => void changeHistory({ type: 'clear-history' })}>确认清空</button>
          <button type="button" disabled={historyBusy} onClick={() => setConfirmHistoryClear(false)}>取消</button>
        </div>}
        {(history.pending.length > 0 || historyRequest?.id === 'pending') && <>
          <h3>排队中 · {history.pending.length} 条</h3>
          {history.pending.length > 0 ? entries(history.pending) : <p className="muted">当前没有排队通知</p>}
        </>}
        <h3>已送达</h3>
        {history.shown.length > 0 ? entries(history.shown) : <p className="muted">暂无灵动通知记录</p>}
      </>}
    </div>}
    </section>
  </>;
}
