import { spawnSync } from 'node:child_process';
import { portedWidgets, widgetFiles } from './build-widgetjs.mjs';
import { cacheMatches, inputGroups, packageState, readBuildState } from './widgetjs-build-state.mjs';

try {
  const state = readBuildState();
  let packages;
  try { packages = portedWidgets.map(name => packageState(widgetFiles(name))); }
  catch (error) { if (process.argv.includes('--check')) throw error; }
  if (packages && cacheMatches(state, inputGroups(), packages)) {
    console.log(`组件构建缓存有效：${packages.length}项，源码和包内容均匹配。`);
  } else {
    if (process.argv.includes('--check')) throw new Error('组件源码或构建资源已变化，请执行 pnpm widgetjs:build；内容变化须提升包版本。');
    console.log('组件构建缓存已过期，先重建并校验版本，再构建主程序。');
    const result = spawnSync(process.execPath, ['scripts/build-widgetjs.mjs'], { stdio: 'inherit', windowsHide: true });
    if (result.error) throw result.error;
    if (result.status !== 0) process.exit(result.status ?? 1);
    const current = portedWidgets.map(name => packageState(widgetFiles(name)));
    if (!cacheMatches(readBuildState(), inputGroups(), current)) throw new Error('重建后组件记录仍不一致，已停止主程序构建。');
  }
} catch (error) {
  console.error(`组件构建校验失败：${error.message}`);
  process.exitCode = 1;
}
