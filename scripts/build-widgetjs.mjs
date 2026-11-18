import { readFileSync, readdirSync, mkdirSync, writeFileSync, copyFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { zipSync, unzipSync } from 'fflate';
import { buildExcalidraw } from './build-excalidraw.mjs';
import { spawnSync } from 'node:child_process';
import { hotspotWidgets } from './widgetjs-hotspots.mjs';
import { assertPackageVersions, inputGroups, packageState, readBuildState, recordBuildState, withBuildLock } from './widgetjs-build-state.mjs';
export { hotspotWidgets } from './widgetjs-hotspots.mjs';

export const progressWidgets = ['time-progress', 'wave-progress'];
export const clockWidgets = ['clock', 'flip-clock', 'glitch-clock', 'mickey-clock', 'text-clock'];
export const standaloneWidgets = ['note', 'forex', 'photo', 'sticker', 'metal', 'grid', 'energy-label', 'base-panel', 'water-reminder'];
export const compiledWidgets = ['excalidraw', 'convert', 'gold', 'stock-medium', 'stock-small', 'coin-small', 'coin-medium', 'mindmap', 'server', 'labor', 'deadline', 'calendar', 'birthday-list', 'todo', 'calendar-large', 'pomodoro', 'stock', 'key-stroke'];
export const countdownWidgets = ['countdown', 'countdown-list'];
export const weatherWidgets = ['weather-small', 'weather-medium', 'weather-large'];
export const reminderWidgets = ['dynamic-island','sit-reminder','phone-reminder'];
export const aiWidgets=['ai-mindmap','ai-mermaid','ai-translate','ai-assistant'];
export const clipboardWidgets=['clipboard','clipboard-search'];
export const jobWidgets=['ai-job'];
export const portedWidgets = [...progressWidgets, ...clockWidgets, ...standaloneWidgets, ...hotspotWidgets, ...compiledWidgets, ...countdownWidgets, ...weatherWidgets, ...reminderWidgets,...aiWidgets,...clipboardWidgets,...jobWidgets];
export function widgetFiles(name) {
  if (!portedWidgets.includes(name)) throw new Error('未知的已移植组件：' + name);
  if (name === 'excalidraw') return unzipSync(new Uint8Array(readFileSync('components/widgetjs/excalidraw-bundle/preset.zip')));
  if (aiWidgets.includes(name) || clipboardWidgets.includes(name)) {
    const family = aiWidgets.includes(name) ? 'ai' : 'clipboard';
    return unzipSync(new Uint8Array(readFileSync(`components/widgetjs/${family}-bundle/${name}.zip`)));
  }
  const files = { 'manifest.json': new Uint8Array(readFileSync(`components/widgetjs/${name}/manifest.json`)), 'dist/host.js': new Uint8Array(readFileSync('sdk/host.js')) };
  const common = progressWidgets.includes(name) ? ['components/widgetjs/progress-common'] : clockWidgets.includes(name) ? ['components/widgetjs/clock-bundle'] : hotspotWidgets.includes(name) ? ['components/widgetjs/hotspot-bundle'] : ['photo','sticker'].includes(name) ? ['components/widgetjs/photo-common'] : countdownWidgets.includes(name) ? ['components/widgetjs/countdown-bundle'] : weatherWidgets.includes(name) ? ['components/widgetjs/weather-bundle'] : reminderWidgets.includes(name)?['components/widgetjs/reminder-bundle']:jobWidgets.includes(name)?['components/widgetjs/job-bundle']:compiledWidgets.includes(name) ? [`components/widgetjs/${name}-bundle`] : [];
  if (hotspotWidgets.includes(name)) files['dist/LICENSE.txt'] = new Uint8Array(readFileSync('components/widgetjs/hotspot-source/LICENSE.txt'));
  if (reminderWidgets.includes(name)) files['dist/LICENSE.txt'] = new Uint8Array(readFileSync('components/widgetjs/reminder-source/LICENSE.txt'));
  if(jobWidgets.includes(name))files['dist/UPSTREAM_NOTICES.txt']=new Uint8Array(readFileSync('components/widgetjs/job-source/UPSTREAM_NOTICES.txt'));
  for (const source of [...common, `components/widgetjs/${name}`]) {
    for (const item of readdirSync(source, { withFileTypes: true })) {
      if (item.isFile() && item.name !== 'manifest.json' && !(source.endsWith('-bundle') && item.name === 'index.html') && !item.name.endsWith('.test.js')) files['dist/' + item.name] = new Uint8Array(readFileSync(join(source, item.name)));
    }
  }
  if(['energy-label','base-panel'].includes(name)){
    for(const path of Object.keys(files))if(path.startsWith('dist/')&&path!=='dist/host.js'){
      files[`dist/${name}/${path.slice(5)}`]=files[path];delete files[path];
    }
    files['dist/hardware-common/metrics.js']=new Uint8Array(readFileSync('components/widgetjs/hardware-common/metrics.js'));
    files['dist/hardware-common/verify.js']=new Uint8Array(readFileSync('components/widgetjs/hardware-common/verify.js'));
  }
  return files;
}
export function widgetArchive(name) {
  return zipSync(widgetFiles(name), { level: 9, mtime: new Date('2026-11-14T00:00:00Z') });
}
function buildWidgets() {
  // 先读取旧记录；构建失败或版本冲突时不覆盖旧记录和已交付ZIP。
  const previous = readBuildState();
  for (const name of ['reminders', 'ai', 'assistant', 'clipboard', 'job']) {
    const result = spawnSync(process.execPath, [`scripts/adapt-widgetjs-${name}.mjs`], { stdio: 'inherit', windowsHide: true });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`组件适配失败：${name}`);
  }
  for (const name of ['clock', 'hotspot', 'weather', ...compiledWidgets.filter(name=>name!=='excalidraw'), 'countdown','reminder','job']) {
    const config=`components/widgetjs/${name}-source/vite.config.mjs`;
    const result = spawnSync(process.execPath, ['node_modules/vite/bin/vite.js', 'build', '--config', config], { stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`组件编译失败：${name}`);
  }
  copyFileSync('components/widgetjs/convert-bundle/index.html', 'components/widgetjs/convert/index.html');
  copyFileSync('components/widgetjs/gold-bundle/index.html', 'components/widgetjs/gold/index.html');
  copyFileSync('components/widgetjs/stock-medium-bundle/index.html', 'components/widgetjs/stock-medium/index.html');
  copyFileSync('components/widgetjs/stock-small-bundle/index.html', 'components/widgetjs/stock-small/index.html');
  copyFileSync('components/widgetjs/coin-small-bundle/index.html', 'components/widgetjs/coin-small/index.html');
  copyFileSync('components/widgetjs/coin-medium-bundle/index.html', 'components/widgetjs/coin-medium/index.html');
  copyFileSync('components/widgetjs/mindmap-bundle/index.html', 'components/widgetjs/mindmap/index.html');
  copyFileSync('components/widgetjs/server-bundle/index.html', 'components/widgetjs/server/index.html');
  copyFileSync('components/widgetjs/labor-bundle/index.html', 'components/widgetjs/labor/index.html');
  copyFileSync('components/widgetjs/deadline-bundle/index.html', 'components/widgetjs/deadline/index.html');
  copyFileSync('components/widgetjs/calendar-bundle/index.html', 'components/widgetjs/calendar/index.html');
  copyFileSync('components/widgetjs/birthday-list-bundle/index.html', 'components/widgetjs/birthday-list/index.html');
  copyFileSync('components/widgetjs/todo-bundle/index.html', 'components/widgetjs/todo/index.html');
  copyFileSync('components/widgetjs/calendar-large-bundle/index.html', 'components/widgetjs/calendar-large/index.html');
  copyFileSync('components/widgetjs/pomodoro-bundle/index.html', 'components/widgetjs/pomodoro/index.html');
  copyFileSync('components/widgetjs/stock-bundle/index.html', 'components/widgetjs/stock/index.html');
  copyFileSync('components/widgetjs/key-stroke-bundle/index.html', 'components/widgetjs/key-stroke/index.html');
  const weatherHtml = readFileSync('components/widgetjs/weather-bundle/index.html','utf8');
  for (const name of weatherWidgets) writeFileSync(`components/widgetjs/${name}/index.html`, weatherHtml.replace('<body>', `<body data-variant="${name}">`));
  const countdownHtml=readFileSync('components/widgetjs/countdown-bundle/index.html','utf8');
  copyFileSync('components/widgetjs/countdown-bundle/index.html','components/widgetjs/countdown/index.html');
  writeFileSync('components/widgetjs/countdown-list/index.html',countdownHtml.replace('<body>','<body data-countdown-variant="countdown-list">'));
  const reminderHtml=readFileSync('components/widgetjs/reminder-bundle/index.html','utf8');
  for(const name of reminderWidgets)writeFileSync(`components/widgetjs/${name}/index.html`,reminderHtml.replace('<body>',`<body data-reminder-variant="${name}">`));
  const entries = spawnSync(process.execPath, ['scripts/build-widgetjs-entries.mjs'], { stdio: 'inherit', windowsHide: true });
  if (entries.error) throw entries.error;
  if (entries.status !== 0) throw new Error('组件独立角色包构建失败');
  copyFileSync('components/widgetjs/job-bundle/index.html','components/widgetjs/ai-job/index.html');
  buildExcalidraw();
  const packages = portedWidgets.map(name => packageState(widgetFiles(name)));
  assertPackageVersions(previous.packages, packages);
  mkdirSync('artifacts', { recursive: true });
  for (const name of portedWidgets) {
    const manifest = JSON.parse(new TextDecoder().decode(widgetFiles(name)['manifest.json']));
    const path = `artifacts/${manifest.id}-${manifest.version}.zip`;
    writeFileSync(path, widgetArchive(name));
    console.log(path);
  }
  recordBuildState(inputGroups(), packages);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { withBuildLock(buildWidgets); }
  catch (error) { console.error(`组件构建失败：${error.message}`); process.exitCode = 1; }
}
