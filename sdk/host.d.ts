export interface HostConnection {
  instanceId: string;
  preview: boolean;
  request<T = unknown>(method: string, args?: unknown): Promise<T>;
  ready(): Promise<unknown>;
  subscribe(keys: unknown[], callback: (snapshot: unknown) => void): Promise<() => Promise<void>>;
}
export function connectHost(): Promise<HostConnection>;
