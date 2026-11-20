import { useEffect, useState } from 'react';
import { invoke } from './host';

type Channel = 'windows' | 'island';
type Preferences = Record<Channel, boolean> & { calls: boolean };
const samples = [
  ['info', '普通信息'], ['success', '成功'], ['warning', '警告'], ['error', '错误'],
  ['reminder', '双按钮提醒'], ['advance-countdown', '倒计时预告'],
  ['countdown', '实时倒计时'], ['call', '模拟来电（静音）'],
] as const;
type Sample = typeof samples[number][0];
type Command =
  | { type: 'channel'; channel: Channel; enabled: boolean }
  | { type: 'calls'; enabled: boolean }
  | { type: 'test'; channel: Channel; sample: Sample };

export function NotificationSettings() {
  const [preferences, setPreferences] = useState<Preferences>();
  const [sample, setSample] = useState<Sample>('info');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    invoke<Preferences>('notification_settings', { command: { type: 'get' } }).then(
      value => { if (active) setPreferences(value); },
      failure => { if (active) setError(String(failure)); },
    );
    return () => { active = false; };
  }, []);

  async function change(command: Command) {
    setBusy(true);
    setError('');
    try {
      setPreferences(await invoke<Preferences>('notification_settings', { command }));
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
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
    {error && <p role="alert" className="notice error">{error}</p>}
  </section>;
}
