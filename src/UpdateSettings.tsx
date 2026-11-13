import { useState } from 'react';
import { invoke } from './host';
import { Icon } from './Icon';

export interface UpdateState {
  currentVersion: string; autoCheck: boolean; phase: string; version: string | null; notes: string | null;
  downloaded: number; total: number | null; checkedAt: number | null; error: string | null; backup: string | null;
}
export function UpdateSettings({ state, refresh }: { state?: UpdateState; refresh: () => Promise<void> }) {
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const working = busy || ['checking', 'downloading', 'installing'].includes(state?.phase ?? '');
  const run = async (action: string, enabled?: boolean) => {
    setBusy(true); setError('');
    try { await invoke('app_update', { action, enabled, version: state?.version }); }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); await refresh(); }
  };
  const labels: Record<string, string> = { idle: '尚未检查更新', checking: '正在检查更新…', latest: '当前已是最新版本', available: '发现新版本', downloading: '正在下载并校验…', downloaded: '下载完成，签名校验通过', installing: '正在备份数据并启动安装…', error: '更新未完成，可重新检查' };
  return <section className="settings-panel update-settings"><div className="setting-row"><label>应用更新<small>当前版本 {state?.currentVersion ?? '—'}{state?.version ? ` · 可更新至 ${state.version}` : ''}</small></label><button disabled={working || !state} onClick={() => void run('check')}><Icon name="reload" />检查更新</button></div>
    <label className="check"><input type="checkbox" disabled={working || !state} checked={state?.autoCheck ?? true} onChange={event => void run('auto-check', event.target.checked)} />自动检查更新<small>启动后及每6小时检查；安装前由你确认</small></label>
    <p role="status">{labels[state?.phase ?? 'idle']}{state?.checkedAt ? ` · 上次检查 ${new Date(state.checkedAt).toLocaleString('zh-CN')}` : ''}</p>
    {state?.notes && <details><summary>更新说明 · {state.version}</summary><pre className="release-notes">{state.notes}</pre></details>}
    {state?.phase === 'downloading' && <div className="update-progress"><progress max={state.total ?? undefined} value={state.total ? state.downloaded : undefined} /><small>{(state.downloaded / 1024 / 1024).toFixed(1)} MB{state.total ? ` / ${(state.total / 1024 / 1024).toFixed(1)} MB` : ''}</small></div>}
    {(error || state?.error) && <p role="alert" className="error">{error || state?.error}</p>}
    <div className="actions">{state?.phase === 'available' && <button disabled={working} onClick={() => void run('download')}><Icon name="download" />下载 {state.version}</button>}
      {state?.phase === 'downloaded' && <button className="primary" disabled={working} onClick={() => { if (window.confirm(`安装 ${state.version}？将先保存布局并备份数据，随后退出 SoulDesk 完成更新。安装期间价格警报会暂停。`)) void run('install'); }}>安装并重启</button>}
    </div>
    {state?.backup && <p className="muted">更新前备份：{state.backup}</p>}
  </section>;
}
