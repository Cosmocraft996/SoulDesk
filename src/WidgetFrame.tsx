import { useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { invoke } from './host';
import { widgetEntryUrl } from './widget-url';
import { framePermissions } from './widget-permissions';

interface Session { token: string; instanceId: string; packageId: string; entry: string; resourcePath: string; version: string; digest: string; verify: boolean; verificationMode?: string; role?: string; preview?: boolean; capabilities?: string[] }
export function WidgetFrame({ instance, probe = false, page, assistantShell = false, jobShell = false }: { instance: string; probe?: boolean; page?: string; assistantShell?: boolean; jobShell?: boolean }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [entry, setEntry] = useState<{ url: string; packageId: string }>();
  const [error, setError] = useState('');
  const [ready, setReady] = useState(false);
  const [revision, setRevision] = useState(0);
  const [permissions, setPermissions] = useState(framePermissions());
  const inPanel = new URLSearchParams(location.search).has('panel');
  useEffect(() => {
    if (probe) return;
    const stop = listen<string>('host:reload-instance', event => {
      if (event.payload !== instance) return;
      // 相同资源地址不会触发iframe导航；先卸载旧页面，再重新握手。
      setEntry(undefined); setReady(false); setError(''); setRevision(value => value + 1);
    });
    const stopError = listen<{ instance: string; message: string }>('host:instance-error', event => {
      if (event.payload.instance === instance) setError(event.payload.message);
    });
    const stopPomodoro = new URLSearchParams(location.search).has('pomodoroWindow') ? listen<unknown>('host:pomodoro-window', event => {
      const iframe = frame.current;
      if (iframe?.contentWindow && iframe.src) iframe.contentWindow.postMessage({ type: 'souldesk:pomodoro-window', surface: event.payload }, new URL(iframe.src).origin);
    }) : undefined;
    return () => { void stop.then(off => off()); void stopError.then(off => off()); void stopPomodoro?.then(off => off()); };
  }, [instance, probe]);
  useEffect(() => {
    const call = <T,>(name: string, args: Record<string, unknown>) => invoke<T>(jobShell ? `job_${name}` : assistantShell ? `assistant_${name}` : probe ? `desktop_widget_${name}` : name, args);
    let disposed = false;
    let session: Session | undefined;
    let channel: MessageChannel | undefined;
    let listener: ((e: MessageEvent) => void) | undefined;
    let updateTimer: ReturnType<typeof setInterval> | undefined;
    const fail = (message: string) => {
      if (disposed) return;
      setError(message);
      if (!probe) void invoke(jobShell ? 'job_surface_failed' : assistantShell ? 'assistant_surface_failed' : 'surface_failed', { instance, message }).catch(() => {});
    };
    const startTimeout = () => setTimeout(() => fail(inPanel ? '组件初始化超过12秒，可重新加载。' : '组件未在 10 秒内完成握手，可重新加载。'), inPanel ? 12_000 : 10_000);
    let timeout = startTimeout();
    void call<Session>('open_session', { instance }).then(s => {
      session = s;
      if (disposed) { void call('close_session', { token: s.token }); return; }
      const resource = widgetEntryUrl(convertFileSrc(s.resourcePath, 'widget'));
      if (s.packageId === 'local.souldesk.cloud-workbench' && s.role === 'details' && page === 'cloud-workbench'
        && !['0.1.0', '0.1.1'].includes(s.version)) {
        resource.pathname = new URL('./console.html', resource).pathname;
      }
      resource.searchParams.set('hostOrigin', location.origin);
      if (s.verify) resource.searchParams.set('verify', s.verificationMode ?? '1');
      if (s.role) resource.searchParams.set('role', s.role);
      if (page) resource.searchParams.set('page', page);
      let handshake = 0;
      listener = async e => {
        if (e.source === frame.current?.contentWindow && e.origin === resource.origin && e.data?.type === 'souldesk:user-pointer' && e.data?.protocol === 1) {
          if (!probe && !new URLSearchParams(location.search).has('instance')) return;
          void invoke(probe ? 'desktop_widget_activate' : 'surface_activate').catch(cause => { if (!disposed) setError(String(cause)); });
          return;
        }
        if (e.source === frame.current?.contentWindow && e.origin === resource.origin && e.data?.type === 'souldesk:failed') {
          const message = String(e.data.message).slice(0, 1000);
          fail(message);
          if (s.preview) void call('trial_failed', { message });
          return;
        }
        if (e.source !== frame.current?.contentWindow || e.origin !== resource.origin || e.data?.type !== 'souldesk:ready' || e.data?.protocol !== 1) return;
        const generation = ++handshake;
        if (inPanel && generation > 1) { clearTimeout(timeout); timeout = startTimeout(); }
        clearInterval(updateTimer);
        channel?.port1.close(); channel?.port2.close();
        setReady(false);
        let active: Session;
        try { active = await call<Session>('open_session', { instance, expectedDigest: s.digest }); }
        catch (cause) { if (!disposed && generation === handshake) fail(String(cause)); return; }
        if (disposed || generation !== handshake) { void call('close_session', { token: active.token }); return; }
        session = active;
        const next = new MessageChannel();
        channel = next;
        const current = () => !disposed && channel === next && session?.token === active.token;
        const subscriptions = new Set<string>();
        let updating = false;
        updateTimer = setInterval(() => {
          if (!current() || updating || !subscriptions.size) return;
          updating = true;
          void Promise.all([...subscriptions].map(async subscriptionId => {
            try {
              const result = await call('widget_request', { token: active.token, method: 'market.poll', args: { subscriptionId } });
              if (current() && subscriptions.has(subscriptionId)) next.port1.postMessage({ protocol: 1, session: active.token, event: 'market:update', result });
            } catch (cause) { if (current()) next.port1.postMessage({ protocol: 1, session: active.token, event: 'market:error', result: { subscriptionId, message: String(cause) } }); }
          })).finally(() => { updating = false; });
        }, 250);
        next.port1.onmessage = async event => {
          const request = event.data;
          if (!current() || request?.protocol !== 1 || request.session !== active.token || typeof request.id !== 'string' || request.id.length > 100 || typeof request.method !== 'string') return;
          const reply = { protocol: 1, session: active.token, id: request.id };
          try {
            const result = await call<{ subscriptionId?: string }>('widget_request', { token: active.token, method: request.method, args: request.args ?? null });
            if (request.method === 'market.subscribe' && result?.subscriptionId) subscriptions.add(result.subscriptionId);
            if (request.method === 'market.unsubscribe' && request.args?.subscriptionId) subscriptions.delete(request.args.subscriptionId);
            if (current()) {
              next.port1.postMessage({ ...reply, result });
              if (inPanel && request.method === 'runtime.ready') {
                clearTimeout(timeout); setReady(true); setError('');
              }
            }
          } catch (cause) {
            if (current()) next.port1.postMessage({ ...reply, error: { code: String(cause).split(':')[0], message: String(cause) } });
          }
        };
        frame.current?.contentWindow?.postMessage({ type: 'souldesk:init', protocol: 1, session: active.token, instanceId: active.instanceId, preview: active.preview === true }, resource.origin, [next.port2]);
        if (!inPanel) { clearTimeout(timeout); setReady(true); setError(''); }
      };
      window.addEventListener('message', listener);
      setPermissions(framePermissions(s.capabilities, s.preview === true));
      setEntry({ url: resource.href, packageId: s.packageId });
    }).catch(cause => { clearTimeout(timeout); fail(String(cause)); });
    return () => {
      disposed = true; clearTimeout(timeout);
      clearInterval(updateTimer);
      if (listener) window.removeEventListener('message', listener);
      channel?.port1.close(); channel?.port2.close();
      if (session) void call('close_session', { token: session.token });
    };
  }, [instance, revision, probe, page, assistantShell, jobShell, inPanel]);
  // 照片和贴纸的透明区域属于组件外观，保留它们与桌面的混合效果。
  const transparentShape = entry?.packageId === 'local.souldesk.widgetjs.photo' || entry?.packageId === 'local.souldesk.widgetjs.sticker';
  return <div className="frame-wrap" data-transparent-shape={transparentShape || undefined}>
    {entry && <iframe ref={frame} title="独立组件" src={entry.url} sandbox={permissions.sandbox} allow={permissions.allow} allowFullScreen={permissions.allowFullScreen} referrerPolicy="no-referrer" />}
    {error ? <div className="frame-message error">{error}<button onClick={() => { setError(''); setReady(false); setEntry(undefined); setRevision(v => v + 1); }}>重新加载</button></div> : !ready && <div className="frame-message">正在连接组件…</div>}
  </div>;
}
