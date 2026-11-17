/** 浏览器权限与组件清单一一对应；隐藏试加载只保留基础沙箱。 */
export function framePermissions(capabilities: readonly string[] = [], preview = false) {
  const granted = new Set(preview ? [] : capabilities);
  const sandbox = ['allow-scripts', 'allow-same-origin', 'allow-forms'];
  const allow: string[] = [];
  if (granted.has('web.embed')) {
    sandbox.push('allow-presentation');
    allow.push('fullscreen');
  }
  if (granted.has('clipboard.read')) allow.push('clipboard-read');
  if (granted.has('clipboard.write')) allow.push('clipboard-write');
  if (granted.has('notifications.island')) allow.push('autoplay');
  if (granted.has('ai.generate') || granted.has('jobs.manage')) allow.push('fullscreen');
  return { sandbox: sandbox.join(' '), allow: allow.length ? [...new Set(allow)].join('; ') : undefined, allowFullScreen: granted.has('web.embed') || granted.has('ai.generate') || granted.has('jobs.manage') };
}
