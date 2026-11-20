export type NotificationChannel = 'windows' | 'island';
export type NotificationRequest = { requestId: string; channel?: NotificationChannel } & (
  | { kind?: 'info' | 'success' | 'warning' | 'error' | 'reminder'; title: string; body?: string }
  | { kind: 'advance-countdown' | 'countdown'; title: string; body?: string; targetAt: number }
  | { kind: 'call'; preset: 'squidward' | 'tea' | 'pill' | 'study' }
);
export type NotificationReceipt =
  | { status: 'accepted'; id: string; channel: NotificationChannel }
  | { status: 'suppressed' };

export interface HostConnection {
  instanceId: string;
  preview: boolean;
  request(method: 'notifications.send', args: NotificationRequest): Promise<NotificationReceipt>;
  request<T = unknown>(method: string, args?: unknown): Promise<T>;
  ready(): Promise<unknown>;
  subscribe(keys: unknown[], callback: (snapshot: unknown) => void): Promise<() => Promise<void>>;
}
export function connectHost(): Promise<HostConnection>;
