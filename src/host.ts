import { invoke as tauriInvoke, type InvokeArgs } from '@tauri-apps/api/core';

declare global {
  interface Window {
    readonly __SOULDESK_HOST_TOKEN__?: string;
    readonly __SOULDESK_APP_THEME__?: string;
  }
}

// 令牌只注入宿主顶层页面，不传给组件或写入数据库。
export function invoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  return tauriInvoke<T>(command, args, { headers: { 'x-souldesk-host': window.__SOULDESK_HOST_TOKEN__ ?? '' } });
}
