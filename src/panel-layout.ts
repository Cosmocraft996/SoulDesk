export interface PanelLayout { x: number; y: number; width: number; height: number }
export interface PanelWidget { id: string; packageId: string; width: number | null; height: number | null }
export interface PanelPackage { id: string; defaultSize: { width: number; height: number }; minSize: { width: number; height: number } }
export function fitLayout(layout: PanelLayout, available: number, min: { width: number; height: number }): PanelLayout {
  // 极窄工作区保留组件最小宽度，画布提供横向滚动，不把组件压坏。
  const width = Math.max(min.width, Math.min(layout.width, Math.max(min.width, available)));
  return { x: Math.max(0, Math.min(layout.x, Math.max(0, available - width))), y: Math.max(0, layout.y), width, height: Math.max(min.height, layout.height) };
}
export function initialLayouts(instances: PanelWidget[], packages: PanelPackage[], saved: Record<string, PanelLayout>, available: number): Record<string, PanelLayout> {
  const result: Record<string, PanelLayout> = {};
  let nextY = 0;
  for (const instance of instances) {
    const pack = packages.find(p => p.id === instance.packageId);
    if (!pack) continue;
    const old = saved[instance.id];
    const layout = old ?? { x: 0, y: nextY, width: instance.width ?? pack.defaultSize.width, height: instance.height ?? pack.defaultSize.height };
    result[instance.id] = fitLayout(layout, available, pack.minSize);
    nextY = Math.max(nextY, result[instance.id].y + result[instance.id].height + 12);
  }
  return result;
}
export function changeLayout(start: PanelLayout, dx: number, dy: number, resize: boolean, available: number, min: { width: number; height: number }): PanelLayout {
  const changed = resize
    ? { ...start, width: Math.min(16_384, Math.max(min.width, start.width + dx)), height: Math.min(16_384, Math.max(min.height, start.height + dy)) }
    : { ...start, x: start.x + dx, y: Math.min(100_000, start.y + dy) };
  return fitLayout(changed, available, min);
}
