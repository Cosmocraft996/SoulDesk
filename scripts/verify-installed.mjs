import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash, randomUUID } from 'node:crypto';
import { validateReportContract } from './verification-contract.mjs';
import { stopOwnedChild } from './child-cleanup.mjs';

const args = process.argv.slice(2);
const expectedVersion = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;
const auto = args.includes('--auto');
const g1 = args.includes('--g1');
const g3 = args.includes('--g3');
const g2 = args.includes('--g2') || g3;
const exampleVersion = JSON.parse(readFileSync(new URL('../examples/clock/manifest.json', import.meta.url), 'utf8')).version;
const marketVersion = JSON.parse(readFileSync(new URL('../examples/market/manifest.json', import.meta.url), 'utf8')).version;
const [executable, packagePath = g2 ? `artifacts/local.souldesk.market-${marketVersion}.zip` : `artifacts/local.example.clock-${exampleVersion}.zip`] = args.filter(arg => !['--auto', '--g1', '--g2', '--g3'].includes(arg));
if (!executable || !existsSync(executable) || !existsSync(packagePath)) {
  console.error('用法：pnpm g0:verify <已安装的 SoulDesk.exe> [外部组件包.zip]');
  process.exit(1);
}
let launchContext = { platform: process.platform, packageFamily: null };
if (process.platform === 'win32') {
  const identity = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./windows-package-identity.ps1', import.meta.url))], { encoding: 'utf8', windowsHide: true });
  if (identity.error || identity.status !== 0) {
    console.error('无法核对 Windows 包身份，停止安装版验收。');
    process.exit(1);
  }
  try { launchContext = { ...launchContext, ...JSON.parse(identity.stdout.trim()) }; }
  catch { console.error('Windows 包身份结果无效，停止安装版验收。'); process.exit(1); }
  if (launchContext.packageFamily) {
    console.error(`当前进程继承了 ${launchContext.packageFamily} 的 MSIX 身份，通知及安装路径可能被重定向。请从资源管理器安装，并在独立打开的普通 PowerShell 中运行验收命令。此次未启动测试。`);
    process.exit(1);
  }
}
mkdirSync('artifacts', { recursive: true });
const runId = randomUUID();
const phase = g3 ? 'g3' : g2 ? 'g2' : g1 ? 'g1' : 'g0';
const executableSha256 = createHash('sha256').update(readFileSync(executable)).digest('hex');
if (g3) {
  const expected = JSON.parse(readFileSync('artifacts/delivery-manifest-g3-032.json', 'utf8'));
  if (expected.version !== expectedVersion || expected.executable.sha256 !== executableSha256) throw new Error('安装程序与当前G3交付清单不符，请先安装配套版本');
  if (createHash('sha256').update(readFileSync(packagePath)).digest('hex') !== expected.component.sha256) throw new Error('组件包与当前G3交付清单不符，未启动检查');
}
const reportPath = resolve(`artifacts/${phase}-installed-${runId}.json`);
writeFileSync(resolve(`artifacts/${phase}-launch-${runId}.json`), JSON.stringify({ runId, executable: resolve(executable), executableSha256, ...launchContext }, null, 2));
console.log(`报告：${reportPath}`);
const child = spawn(resolve(executable), ['--g0-smoke', resolve(packagePath), '--g0-report', reportPath, '--g0-run-id', runId, ...(auto ? ['--g0-auto'] : []), ...(g1 ? ['--g0-g1'] : []), ...(g2 ? ['--g0-g2'] : []), ...(g3 ? ['--g0-g3'] : [])], { windowsHide: true, stdio: 'ignore' });
let spawnError;
child.on('error', error => { spawnError = error; });
let lastStage = '';
const deadline = Date.now() + (g2 ? 900_000 : 420_000);
try { while (Date.now() < deadline) {
  if (spawnError) throw new Error(`验证进程无法启动：${spawnError.message}`);
  if (existsSync(reportPath)) {
    let report;
    try { report = JSON.parse(readFileSync(reportPath, 'utf8')); } catch { /* 等待本次完整写入 */ }
    if (report?.runId === runId) {
      validateReportContract(report, phase, expectedVersion);
      if (report.executableSha256 !== executableSha256) throw new Error('报告与当次启动程序散列不符');
      if (report.stage !== lastStage) { lastStage = report.stage; console.log(lastStage); }
      if (report.failure) throw new Error(report.failure);
      if (report.programChecksComplete && (report.stage?.startsWith('程序检查完成') || report.stage?.startsWith('自动检查完成'))) { console.log(`当次${phase.toUpperCase()}程序检查完成，${report.checks.length} 项证据；人工或环境范围：${report.manualPending.join('、')}。阶段验收需结合人工反馈。`); break; }
    }
  }
  if (child.exitCode !== null) throw new Error('验证进程提前退出且没有当次成功报告；请检查是否已有 SoulDesk 正在运行或初始化失败。');
  await new Promise(resolve => setTimeout(resolve, 400));
}
if (Date.now() >= deadline) throw new Error('安装版验收超时；未判通过。请保留当次报告。');
} catch (error) { console.error(error.message); process.exitCode = 1; }
finally { await stopOwnedChild(child); }
