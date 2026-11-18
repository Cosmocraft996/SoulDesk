// 按角色选择固定入口；所有依赖仍由Vite打入组件包，不使用宿主公共依赖或远程脚本。
import { resolve } from 'node:path';
import { normalizePath } from 'vite';

export function entryPlugin(root, source) {
  const entry = 'virtual:souldesk-widget-entry', resolved = '\0' + entry;
  return {
    name: '组件固定角色入口',
    resolveId(id, importer) {
      if (id === entry) return resolved;
      if (importer === resolved && id.startsWith('.')) return normalizePath(resolve(root, id));
    },
    load(id) { if (id === resolved) return source; },
  };
}
