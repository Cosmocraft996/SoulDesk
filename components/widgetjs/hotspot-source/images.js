// 外部内容图片失败时保留榜单；包内资源失败仍交由宿主检查。
const placeholder = 'data:image/svg+xml;base64,' + btoa('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#e7e9ed"/><text x="32" y="43" text-anchor="middle" font-size="36" fill="#7b8490">?</text></svg>');
export function fallbackRemoteImage(event, status) {
  const image = event.target;
  if (!(image instanceof HTMLImageElement)) return false;
  let url; try { url = new URL(image.src); } catch { return false; }
  if (url.protocol !== 'https:' || url.hostname === 'widget.localhost') return false;
  image.dataset.failedRemote = '1'; image.alt = '图片未能加载';
  status.failed++; image.removeAttribute('srcset'); image.src = placeholder;
  return true;
}
