// 六种角色分别构建；直接封装完整标准包，防止独立包和Rust预置漏带动态分块。
import { build } from 'vite';
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import { zipSync } from 'fflate';
import { aiWidgets, clipboardWidgets } from './build-widgetjs.mjs';

const reports = [];
for (const name of [...aiWidgets, ...clipboardWidgets]) {
  const family = aiWidgets.includes(name) ? 'ai' : 'clipboard';
  const result = await build({ configFile: `components/widgetjs/${family}-source/vite.config.mjs`, mode: name, build: { write: false }, logLevel: 'warn' });
  if (Array.isArray(result) || !result.output) throw new Error(`组件构建输出不唯一：${name}`);
  const files = {};
  for (const item of result.output) files[`dist/${item.fileName}`] = Buffer.from(item.type === 'chunk' ? item.code : item.source);
  for (const item of readdirSync(`components/widgetjs/${name}`, { withFileTypes: true })) {
    if (item.isFile()) files[item.name === 'manifest.json' ? item.name : `dist/${item.name}`] = readFileSync(`components/widgetjs/${name}/${item.name}`);
  }
  files['dist/host.js'] = readFileSync('sdk/host.js');
  files[`dist/${family === 'ai' ? 'LICENSE.txt' : 'LICENSE-AI.txt'}`] = readFileSync('components/widgetjs/ai-source/LICENSE.txt');
  const entry = result.output.find(item => item.type === 'chunk' && item.isEntry);
  const initial = new Set();
  function visit(file) { if (initial.has(file)) return; initial.add(file); for (const dependency of result.output.find(item => item.fileName === file)?.imports ?? []) visit(dependency); }
  visit(entry.fileName);
  reports.push({ name, initialScripts: [...initial].map(file => `dist/${file}`), chunks: result.output.filter(item => item.type === 'chunk').map(item => ({ file: item.fileName, imports: item.imports, dynamicImports: item.dynamicImports, modules: Object.keys(item.modules).map(path => path.replaceAll('\\', '/').replace(process.cwd().replaceAll('\\', '/') + '/', '')) })) });
  writeFileSync(`components/widgetjs/${family}-bundle/${name}.zip`, zipSync(files, { level: 9, mtime: new Date('2026-11-14T00:00:00Z') }));
  console.log(`独立角色包构建完成：${name}`);
}
mkdirSync('artifacts', { recursive: true });
writeFileSync('artifacts/widgetjs-entry-build.json', JSON.stringify(reports, null, 2) + '\n');
