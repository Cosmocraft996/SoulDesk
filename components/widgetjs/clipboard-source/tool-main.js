// 仅AI剪贴板的工具窗口载入生成界面，沿用原工具命名空间和宿主授权。
import { mountGenerator } from '../ai-source/main.js';
import { toolHost } from './tool-host.js';

export async function mountTool(host, variant) {
  const { default: view } = variant === 'mindmap'
    ? await import('../ai-source/views/Mindmap.vue')
    : await import('../ai-source/views/Translate.vue');
  const info = host.preview ? null : await host.request('ai.toolInfo');
  return mountGenerator(toolHost(host), variant, view, info?.content);
}
