import { invoke as tauriInvoke } from '@tauri-apps/api/core';

declare global { interface Window { readonly __SOULDESK_HOST_TOKEN__?: string } }

// 令牌只注入宿主顶层页面，不传给组件或写入数据库。
export function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(command, args, { headers: { 'x-souldesk-host': window.__SOULDESK_HOST_TOKEN__ ?? '' } });
}
