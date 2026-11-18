import{defineConfig}from'vite';import vue from'@vitejs/plugin-vue';import{fileURLToPath}from'node:url';
import { bundledLicenses } from '../ai-source/vite.config.mjs';
import { entryPlugin } from '../../../scripts/widgetjs-entry.mjs';
export default defineConfig(({ mode }) => {
  const root = fileURLToPath(new URL('.', import.meta.url)), search = mode === 'clipboard-search';
  const source = [
    "import { connectHost } from '../../../sdk/host.js'; import { mountClipboard } from './main.js'; async function start() { const host = await connectHost();",
    search
      ? "const page = new URLSearchParams(location.search).get('page'), variant = page === 'clipboard-ai-mindmap' ? 'mindmap' : page === 'clipboard-ai-translate' ? 'translate' : null; if (variant) { const { mountTool } = await import('./tool-main.js'); await mountTool(host, variant); } else await mountClipboard(host, 'search');"
      : "await mountClipboard(host, 'history');",
    // 先完成入口模块求值，工具分块才能反向复用入口中的公共依赖，避免顶层await形成循环等待。
    "} start().catch(error => { document.getElementById('app').textContent = '组件载入失败：' + error.message; throw error; });",
  ].join('\n');
  return { root, base: './', publicDir: false, plugins: [vue(), bundledLicenses(), entryPlugin(root, source)], build: {
    target: 'es2022', outDir: '../clipboard-bundle', emptyOutDir: false, assetsInlineLimit: 0, cssCodeSplit: false, modulePreload: false,
    rollupOptions: { input: 'virtual:souldesk-widget-entry', output: { entryFileNames: 'app.js', chunkFileNames: '[name]-[hash].js', assetFileNames: asset => asset.names?.some(name => name.endsWith('.css')) ? 'style.css' : '[name][extname]' } },
  } };
});
