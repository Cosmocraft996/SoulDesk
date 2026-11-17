export interface PanelLayout { x: number; y: number; width: number; height: number }
export type SnapEdge = 'left' | 'right' | 'top' | 'bottom';
export interface SnapMatch { edge: SnapEdge; peer?: number; peerEdge: SnapEdge }
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
function nearest(value: number, candidates: (SnapMatch & { value: number })[], min: number, max: number, matches: SnapMatch[]): number {
  let result = value, distance = Infinity;
  let selected: SnapMatch | undefined;
  for (const candidate of candidates) {
    const delta = Math.abs(candidate.value - value);
    if (Number.isFinite(candidate.value) && candidate.value >= min && candidate.value <= max && delta <= 8 && delta < distance) {
      result = candidate.value; distance = delta;
      selected = { edge: candidate.edge, peer: candidate.peer, peerEdge: candidate.peerEdge };
    }
  }
  if (selected) matches.push(selected);
  return result;
}
export function changeLayout(start: PanelLayout, dx: number, dy: number, resize: boolean, available: number, min: { width: number; height: number }, peers: PanelLayout[] = [], viewport?: { top: number; bottom: number }, matches: SnapMatch[] = []): PanelLayout {
  matches.length = 0;
  if (dx === 0 && dy === 0) return { ...start };
  type Target = { at: number; peer?: number; edge: SnapEdge };
  const xs: Target[] = [{ at: 0, edge: 'left' }, { at: available, edge: 'right' }, ...peers.flatMap((g, peer): Target[] => [{ at: g.x, peer, edge: 'left' }, { at: g.x + g.width, peer, edge: 'right' }])];
  const ys: Target[] = [{ at: 0, edge: 'top' }, ...(viewport ? [{ at: viewport.top, edge: 'top' } as Target, { at: viewport.bottom, edge: 'bottom' } as Target] : []), ...peers.flatMap((g, peer): Target[] => [{ at: g.y, peer, edge: 'top' }, { at: g.y + g.height, peer, edge: 'bottom' }])];
  const candidate = (target: Target, edge: SnapEdge, value: number) => ({ value, edge, peer: target.peer, peerEdge: target.edge });
  if (resize) {
    // 固定左上角，右边到达面板边界后不再推动整个组件。
    const maxWidth = Math.max(min.width, Math.min(16_384, available - start.x));
    const width = Math.min(maxWidth, Math.max(min.width, start.width + dx));
    const height = Math.min(16_384, Math.max(min.height, start.height + dy));
    return { ...start,
      width: nearest(width, xs.map(t => candidate(t, 'right', t.at - start.x)), min.width, maxWidth, matches),
      height: nearest(height, ys.map(t => candidate(t, 'bottom', t.at - start.y)), min.height, 16_384, matches),
    };
  }
  const changed = fitLayout({ ...start, x: start.x + dx, y: Math.min(100_000, start.y + dy) }, available, min);
  return { ...changed,
    x: nearest(changed.x, xs.flatMap(t => [candidate(t, 'left', t.at), candidate(t, 'right', t.at - changed.width)]), 0, Math.max(0, available - changed.width), matches),
    y: nearest(changed.y, ys.flatMap(t => [candidate(t, 'top', t.at), candidate(t, 'bottom', t.at - changed.height)]), 0, 100_000, matches),
  };
}
