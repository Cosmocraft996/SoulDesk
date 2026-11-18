// 只启动并回收明确的隔离子进程；正常用户数据和已运行的安装版保持独立。
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, openSync, readFileSync, closeSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomUUID, createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { stopOwnedChild } from './child-cleanup.mjs';
const executable = resolve(process.argv[2] ?? 'src-tauri/target/debug/souldesk.exe');
const selectedPackage = process.argv[4];
const contentUpgradeFrom = ['content-upgrade', 'audit-upgrade'].includes(selectedPackage) && process.argv[5] ? resolve(process.argv[5]) : null;
if (['content-upgrade', 'audit-upgrade'].includes(selectedPackage) && !contentUpgradeFrom) throw new Error('缺少升级前的标准ZIP目录');
if (selectedPackage && !['assets', 'content-upgrade', 'audit-upgrade', 'convert-files', 'preview-all', 'time-progress', 'wave-progress', 'clock', 'flip-clock', 'glitch-clock', 'mickey-clock', 'text-clock', 'deadline', 'calendar', 'countdown', 'countdown-list', 'birthday-list', 'todo', 'calendar-large', 'pomodoro', 'note', 'forex', 'energy-label', 'base-panel', 'convert', 'photo', 'sticker', 'water-reminder', 'labor', 'server', 'hotspots', 'coins'].includes(selectedPackage)) throw new Error('只接受已有受控表单检查的组件或全部隐藏加载检查');
if (!existsSync(executable)) throw new Error('请先构建桌面调试程序');
mkdirSync('artifacts', { recursive: true });
const report = resolve(process.argv[3] ?? `artifacts/desktop-integration-widgetjs-${randomUUID()}.json`);
if (existsSync(report) || existsSync(report.replace(/\.json$/, ''))) throw new Error('不覆盖已有隔离报告或测试库');
const launch = { executable, sha256: createHash('sha256').update(readFileSync(executable)).digest('hex'), report, selectedPackage: selectedPackage ?? 'first-eight', launcherPid: process.pid, phases: [], status: 'starting', normalDataOpened: false };
const record = () => writeFileSync(report + '.runner.json', JSON.stringify(launch, null, 2));
record();
const identity = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./windows-package-identity.ps1', import.meta.url))], { encoding: 'utf8', windowsHide: true });
try {
  if (identity.status !== 0) throw new Error('无法读取启动环境的包身份');
  launch.packageFamily = JSON.parse(identity.stdout).packageFamily;
  if (launch.packageFamily !== null) throw new Error('启动环境继承外部MSIX身份，未启动隔离程序');
} catch (error) { launch.status = 'failed'; launch.error = error.message; record(); throw error; }
launch.status = 'running'; record();
const fixture = selectedPackage === 'server' ? await (await import('./widgetjs-server-fixture.mjs')).startServerFixture(report) : null;
console.log('组件移植隔离验收：' + report);
try {
for (const phase of ['widgetjs', 'widgetjs-resume']) {
  const output = openSync(`${report}.${phase}.log`, 'wx');
  const args = ['--desktop-integration-' + phase, report];
  if (selectedPackage) args.push('--widgetjs-check-package=' + selectedPackage);
  if (contentUpgradeFrom) args.push((selectedPackage === 'audit-upgrade' ? '--widgetjs-audit-upgrade-from=' : '--widgetjs-content-upgrade-from=') + contentUpgradeFrom);
  if (fixture) args.push('--widgetjs-server-url=' + fixture.url);
  const child = spawn(executable, args, { windowsHide: true, stdio: ['ignore', output, output] });
  const result = { phase, pid: child.pid, status: 'running' }; launch.phases.push(result); record();
  let timeout;
  try {
    await Promise.race([
      new Promise((resolve, reject) => { child.once('exit', code => code === 0 ? resolve() : reject(new Error('隔离进程退出：' + code))); child.once('error', reject); }),
      new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('组件隔离检查超时')), 180000); timeout.unref(); }),
    ]);
    const value = JSON.parse(readFileSync(report, 'utf8'));
    if (value.status !== (phase === 'widgetjs' ? 'restart-ready' : 'passed') || !value.checks.length || value.checks.some(c => !c.passed)) throw new Error(value.error ?? '验收未通过');
    console.log(`${phase}：${value.checks.length}项通过`);
    result.status = 'passed'; result.exitCode = child.exitCode; result.checks = value.checks.length; record();
  } catch (error) {
    result.status = 'failed'; result.exitCode = child.exitCode; launch.status = 'failed'; launch.error = error.message; record();
    const value = existsSync(report) ? JSON.parse(readFileSync(report, 'utf8')) : {};
    throw new Error(`${error.message}；${value.error ?? ''}；报告：${report}`);
  } finally {
    clearTimeout(timeout);
    try { await stopOwnedChild(child); }
    finally { result.finalExitCode = child.exitCode; result.finalSignal = child.signalCode; record(); closeSync(output); }
  }
}
} finally { await fixture?.close(); }
launch.status = 'passed'; record();
console.log(contentUpgradeFrom ? '旧版标准ZIP导入、显式升级、历史资源策略及实例数据重启保持已检查。' : selectedPackage === 'assets' ? '资源细分真实WebView允许/拒绝、五款隐藏加载及登记重启已检查；人工视觉及普通安装未验。' : selectedPackage === 'preview-all' ? '全部预置隐藏试加载及登记重启检查完成；功能、交互、视觉和普通安装未验。' : selectedPackage === 'convert-files' ? '单实例原生文件选择、五种格式解码、磁盘导出及重启已检查；HEIC、双实例和完整视觉未验。' : '真实宿主/消息桥/表单保存/双实例/面板切换/进程重启已检查；人工视觉及普通安装未验。');
