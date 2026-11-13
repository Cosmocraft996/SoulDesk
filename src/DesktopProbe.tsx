import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from './host';
import { WidgetFrame } from './WidgetFrame';
import './desktop-probe.css';

type Geometry = { left: number; top: number; width: number; height: number; requested_ratio: number; effective_ratio: number };
type WidgetView = { instanceId: string; desktopLabel: string; location: string; visible: boolean };
type WidgetState = { revision: number; instances: WidgetView[]; sessionCount: number; verifiedCount: number; testing: boolean; automatic: { passed?: boolean; error?: string } };
type ProbeState = { geometry: Geometry; windowCount: number; report: string; tintOpacity: number; appearanceEpoch: number; appearance: { material: string }; observations: Record<string, { result: string }>; widgets: WidgetState | null };
const materialNames: Record<string, string> = { 'accent-acrylic': '失焦磨砂候选', 'system-acrylic': '系统磨砂（失焦会回退）', 'transparent-baseline': '透明基线（无模糊）', unknown: '状态不确定，请重新选择' };
const checks = [
  ['desktop', '两个夹具直接显示在桌面'], ['cover', '普通应用能覆盖夹具'],
  ['winD', '手动 Win+D 往返后夹具仍可见'], ['taskSwitcher', '任务栏与 Alt+Tab 无夹具入口'],
  ['input', '显式点击、输入及按钮有效'], ['acrylic', '聚焦及失焦后均保持毛玻璃通透'],
  ['panelGeometry', '铺满可用高度，任务栏可达'], ['pin', '卡片置顶与返回桌面层有效'],
] as const;

export function DesktopProbe({ kind }: { kind: string }) {
  const [now, setNow] = useState(new Date());
  const [state, setState] = useState<ProbeState>();
  const [ratio, setRatio] = useState(40);
  const [tint, setTint] = useState(0.12);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [confirmFinish, setConfirmFinish] = useState(false);
  const [text, setText] = useState('留一点空间给此刻。');
  const [observations, setObservations] = useState<Record<string, string>>({});
  const [widgets, setWidgets] = useState<WidgetState | null>(null);
  const label = `desktop-probe-${kind}`;
  const mounted = widgets?.instances.filter(view => view.visible && (view.location === 'panel' ? kind === 'panel' : view.desktopLabel === label)) ?? [];
  const locked = busy || widgets?.testing === true;
  function updateWidgets(next: WidgetState) {
    setWidgets(previous => !previous || next.revision >= previous.revision ? next : previous);
  }
  async function action(name: string) {
    setError(''); setBusy(true);
    try {
      if (name === 'rebuild-panel') await invoke('desktop_probe_set_tint', { opacity: tint });
      const next = await invoke<ProbeState | null>('desktop_probe_action', { action: name });
      if (next) { setState(next); if (next.widgets) updateWidgets(next.widgets); setRatio(Math.round(next.geometry.requested_ratio * 100)); setTint(next.tintOpacity); setObservations(Object.fromEntries(Object.entries(next.observations).map(([key, value]) => [key, value.result]))); }
    } catch (cause) {
      setError(String(cause));
      if (['transparent-baseline', 'restore-material', 'persistent-material'].includes(name)) {
        setState(previous => previous ? { ...previous, appearance: { material: 'unknown' } } : previous);
      }
    }
    finally { setBusy(false); }
  }
  async function saveTint(opacity: number) {
    try {
      const saved = await invoke<{ appearanceEpoch: number; changed: boolean }>('desktop_probe_set_tint', { opacity });
      setState(previous => previous ? { ...previous, appearanceEpoch: Math.max(previous.appearanceEpoch, saved.appearanceEpoch) } : previous);
      if (saved.changed) setObservations(previous => ({ ...previous, acrylic: 'unverified' }));
      setMessage('本次背景遮罩已记录；不会修改正常应用设置。');
    } catch (cause) { setError(String(cause)); }
  }
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<WidgetState>('desktop-probe:widgets', event => { if (!disposed) updateWidgets(event.payload); }).then(stop => {
      if (disposed) { stop(); return; }
      unlisten = stop;
      return invoke<ProbeState>('desktop_probe_action', { action: 'ready' }).then(next => {
        if (disposed) return;
        setState(next);
        if (next.widgets) updateWidgets(next.widgets);
        setRatio(Math.round(next.geometry.requested_ratio * 100));
        setTint(next.tintOpacity);
        setObservations(Object.fromEntries(Object.entries(next.observations).map(([key, value]) => [key, value.result])));
      });
    }).catch(cause => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; unlisten?.(); };
  }, [kind]);
  useEffect(() => {
    if (kind !== 'clock' || widgets) return;
    const timer = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(timer);
  }, [kind, widgets !== null]);
  useEffect(() => {
    if (!widgets) return;
    // 提交发生在 React 已卸载旧 iframe、执行其清理之后。
    void invoke('desktop_widget_view', { revision: widgets.revision, instances: mounted.map(view => view.instanceId).sort() }).catch(cause => {
      if (!String(cause).includes('STALE_VIEW')) setError(String(cause));
    });
  }, [widgets?.revision, kind]);
  useEffect(() => {
    if (kind !== 'panel') return;
    const timer = setTimeout(() => {
      const background = (element: Element | null) => element ? getComputedStyle(element).backgroundColor : null;
      void invoke('desktop_probe_action', { action: 'surface', surface: {
        html: background(document.documentElement), body: background(document.body), root: background(document.getElementById('root')),
        panel: background(document.querySelector('.probe-panel')), requestedTint: tint,
      } }).catch(cause => setError(String(cause)));
    }, 300);
    return () => clearTimeout(timer);
  }, [kind, tint]);

  if (kind !== 'panel' && widgets) return <main className="probe-real-desktop">
    <header><span>原 ZIP · {kind === 'clock' ? '此刻时钟' : '示例卡片'}</span><button disabled={locked} onClick={() => void action('open-panel')}>打开侧栏 ↗</button></header>
    {mounted.map(view => <WidgetFrame key={view.instanceId} instance={view.instanceId} probe />)}
    {error && <p role="alert" className="probe-error">{error}</p>}
  </main>;
  if (kind !== 'panel') return <main className={`probe-fixture ${kind}`}>
    <header><span>{kind === 'clock' ? '此刻' : '给自己的一句话'}</span><span className="probe-tag">平台夹具</span></header>
    {kind === 'clock' ? <><time>{now.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit' })}<span>{now.getSeconds().toString().padStart(2, '0')}</span></time><p>{now.toLocaleDateString('zh-CN', { month: 'long', day: 'numeric', weekday: 'long' })}</p></>
      : <><textarea aria-label="卡片输入测试" value={text} onChange={e => setText(e.target.value)} /><p>桌面上的一小块安静。</p></>}
    <footer><span>外观验证 · 非组件包</span><button onClick={() => void action('open-panel')}>打开侧栏 ↗</button></footer>
    {error && <div role="alert" className="probe-error">{error}</div>}
  </main>;

  return <main className="probe-panel" style={{ backgroundColor: `rgba(28,28,28,${tint})` }}>
    <header className="probe-panel-heading"><div><span className="probe-tag">SOULDESK · 隔离验证</span><h1>桌面，留给组件。</h1><p>独立桌面层 · 全高磨砂侧栏</p></div><button disabled={busy} onClick={() => void action('hide-panel')}>收起</button></header>
    <div className="probe-panel-content">
      <section className="probe-width"><label htmlFor="panel-ratio">面板宽度 <strong>{ratio}%</strong></label><input id="panel-ratio" type="range" min="1" max="100" step="1" value={ratio} onChange={e => setRatio(Number(e.target.value))} />
        <div className="probe-row"><span>有效宽度 {state ? `${(state.geometry.effective_ratio * 100).toFixed(1)}% · ${state.geometry.width} × ${state.geometry.height}` : '正在读取…'}</span><button disabled={busy} onClick={() => {
          setBusy(true); setError('');
          void invoke<Geometry>('desktop_probe_set_ratio', { ratio: ratio / 100 }).then(geometry => {
            setState(previous => previous ? { ...previous, geometry } : previous); setMessage('比例已保存到本次隔离目录。');
          }).catch(cause => setError(String(cause))).finally(() => setBusy(false));
        }}>应用并保存</button></div>
      </section>
      <section className="probe-width"><label htmlFor="panel-tint">背景遮罩 <strong>{Math.round(tint * 100)}%</strong></label>
        <input id="panel-tint" type="range" min="0" max="80" step="1" value={Math.round(tint * 100)} disabled={busy}
          onChange={e => setTint(Number(e.target.value) / 100)}
          onPointerUp={e => void saveTint(Number(e.currentTarget.value) / 100)}
          onKeyUp={e => void saveTint(Number(e.currentTarget.value) / 100)}
          onBlur={e => void saveTint(Number(e.currentTarget.value) / 100)} />
        <p className="probe-note">越低越通透。此滑条只用于本次材质校准，文字和卡片保持不透明。</p>
        <p role="status" className="probe-material">当前材质：<strong>{state ? materialNames[state.appearance.material] ?? '未知' : '正在读取…'}</strong><br />外观代次：{state?.appearanceEpoch ?? '…'} · 遮罩 {Math.round(tint * 100)}%</p>
        <div className="probe-actions"><button disabled={busy} aria-pressed={state?.appearance.material === 'transparent-baseline'} onClick={() => void action('transparent-baseline')}>检查透明基线（无模糊）</button><button disabled={busy} aria-pressed={state?.appearance.material === 'system-acrylic'} onClick={() => void action('restore-material')}>恢复系统磨砂</button><button disabled={busy} aria-pressed={state?.appearance.material === 'accent-acrylic'} onClick={() => void action('persistent-material')}>验证失焦磨砂</button></div>
      </section>
      {widgets ? <section className="probe-real-widgets"><h2>原包与实际消息桥</h2>
        <p className="probe-note">{widgets.testing ? '正在核对隐藏释放、侧栏往返和受控失败恢复…' : widgets.automatic.passed ? '程序检查已通过，等待实际交互确认。' : widgets.automatic.error ?? '等待检查结果…'}</p>
        <p className="probe-note">已验证原包 {widgets.verifiedCount}/2 · 有效会话 {widgets.sessionCount} · 同一实例只保留一个主展示</p>
        <div className="probe-actions"><button disabled={locked} onClick={() => void action('widget-move-clock')}>时钟：桌面 ↔ 侧栏</button><button disabled={locked} onClick={() => void action('widget-move-card')}>卡片：桌面 ↔ 侧栏</button><button disabled={locked} onClick={() => void action('widget-test-failure')}>验证卡片切换失败恢复</button></div>
        {mounted.map(view => <article key={view.instanceId} className="probe-real-slot"><WidgetFrame instance={view.instanceId} probe /></article>)}
      </section> : <div className="probe-samples"><article className="probe-mini-clock"><span>此刻</span><strong>独立时钟</strong><p>桌面层的薄荷绿夹具</p></article><article className="probe-mini-card"><span>一句话</span><strong>独立卡片</strong><p>可点击和输入的暖色夹具</p></article></div>}
      <section><h2>窗口与恢复</h2><div className="probe-actions"><button disabled={locked} onClick={() => void action('hide-fixtures')}>{widgets ? '隐藏桌面组件' : '隐藏夹具'}</button><button disabled={locked} onClick={() => void action('restore-fixtures')}>无焦点恢复</button><button disabled={locked || Boolean(widgets && !widgets.instances.some(view => view.desktopLabel === 'desktop-probe-card' && view.location === 'desktop' && view.visible))} onClick={() => void action('toggle-pin')}>切换卡片置顶</button><button disabled={locked} onClick={() => void action('rebuild-panel')}>按保存比例重建侧栏</button><button disabled={locked} onClick={() => void action('snapshot')}>记录原生状态</button></div></section>
      <section><h2>实际观察</h2><p className="probe-note">{widgets ? '确认原包交互及跨应用输入焦点。键入检查须先在另一应用输入框中放置光标，再直接点击桌面组件；只出现光标不算通过。' : '磨砂要同时确认聚焦和失焦两种状态。未确认项保持“未验证”；调用成功不会自动勾选。'}</p><div className="probe-checks">{(widgets ? [['widgets', '原包保存、切换和恢复后内容正常'], ['widgetInput', '从其他应用输入框直接切入后，按键只进入组件']] : checks).map(([key, label]) => <label key={key}><span>{label}</span><select aria-label={label} disabled={locked} value={observations[key] ?? 'unverified'} onChange={e => {
        const result = e.target.value;
        setBusy(true);
        void invoke('desktop_probe_observe', { key, result, expectedEpoch: state?.appearanceEpoch, expectedTint: tint }).then(() => {
          setObservations(previous => ({ ...previous, [key]: result })); setMessage('观察结果已记录。');
        }).catch(cause => setError(String(cause))).finally(() => setBusy(false));
      }}><option value="unverified">未验证</option><option value="pass">通过</option><option value="fail">失败</option></select></label>)}</div></section>
      {error && <p role="alert" className="probe-error">{error}</p>}
      {message && <p role="status" className="probe-message">{message}</p>}
      <details><summary>本次记录</summary><p className="probe-report">{state?.report}</p><p>本次窗口 {state?.windowCount ?? '…'} 个；不打开正常数据库；不加载行情；无全局快捷键。30 分钟后自动结束。</p></details>
    </div>
    <footer className="probe-panel-footer"><span>{confirmFinish ? '未完成项会保留为未验证。' : widgets ? '原 ZIP 与 SDK · 隔离测试库' : '只验证平台能力 · 真实组件另验'}</span>{confirmFinish ? <div><button onClick={() => setConfirmFinish(false)}>继续检查</button> <button disabled={busy} onClick={() => void action('finish')}>确认结束</button></div> : <button disabled={busy} onClick={() => setConfirmFinish(true)}>结束本次验证</button>}</footer>
  </main>;
}
