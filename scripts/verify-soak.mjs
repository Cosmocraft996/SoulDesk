import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash, randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stopOwnedChild } from './child-cleanup.mjs';
const seconds = Number(process.argv[2] ?? 1800);
if (!Number.isInteger(seconds) || seconds < 120 || seconds > 86400) throw new Error('时长须为120～86400秒；不足24小时只作短检查');
const expected = JSON.parse(readFileSync('artifacts/delivery-manifest-g3-031.json', 'utf8'));
const executable = resolve(expected.soakExecutable.path);
const digest = createHash('sha256').update(readFileSync(executable)).digest('hex');
if (digest !== expected.soakExecutable.sha256) throw new Error('检查程序散列不符，请保留原文件并重新构建');
const identity = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./windows-package-identity.ps1', import.meta.url))], { encoding: 'utf8', windowsHide: true });
if (identity.status !== 0) throw new Error('无法核对Windows包身份');
const context = JSON.parse(identity.stdout.trim());
if (context.packageFamily) throw new Error('请在资源管理器运行此入口，避免继承外部MSIX身份；本次未启动长期检查');
const runId = randomUUID(), reportPath = resolve(`artifacts/g3-soak-${runId}.json`);
writeFileSync(`artifacts/g3-launch-${runId}.json`, JSON.stringify({ runId, seconds, executable, sha256: digest, ...context }, null, 2));
console.log(`报告：${reportPath}\n初始集成检查约1分钟，其后后台采样${seconds}秒，不发送通知声音；不要同时运行另一实例。`);
const child = spawn(executable, ['--g0-smoke', resolve('artifacts/local.souldesk.market-0.1.0.zip'), '--g0-report', reportPath, '--g0-run-id', runId, '--g0-runtime', '--g0-g2', '--g0-auto', '--g0-soak', String(seconds)], { windowsHide: true, stdio: 'ignore' });
let spawnError;
child.on('error', error => { spawnError = error; });
let previous = '', previousCount = -1;
const deadline = Date.now() + (seconds + 900) * 1000;
try { while (Date.now() < deadline) {
  if (spawnError) throw spawnError;
  let report;
  if (existsSync(reportPath)) { try { report = JSON.parse(readFileSync(reportPath, 'utf8')); } catch { /* 等待一次完整写入 */ } }
  if (report?.runId === runId) {
    if (report.version !== expected.version || report.executableSha256 !== digest) throw new Error('报告版本或程序散列不符');
    if (report.stage !== previous) { previous = report.stage; console.log(previous); }
    if (report.sampleCount !== previousCount && report.sampleCount && report.sampleCount % 120 === 0) { previousCount = report.sampleCount; console.log(`已采样${report.elapsedSeconds}秒，当前私有内存${Math.round(report.latestSample.process.privateBytes / 1048576)}MiB`); }
    if (report.failure) { console.error(report.failure); process.exitCode = 1; break; }
    if (report.soakFinished) {
      if (seconds === 86400 && report.soakPassed !== true) throw new Error('24小时检查未通过，需按报告复核');
      if (seconds < 86400 && report.shortCheckPassed !== true) throw new Error('短采样检查未通过');
      console.log(seconds === 86400 ? '24小时零视图后台检查完成；仍需核对完整首版验收记录。' : '短时采样完成，不能作为24小时通过证据。');
      break;
    }
  }
  if (child.exitCode !== null) throw new Error('测试进程提前退出，未取得完整成功报告');
  await new Promise(resolve => setTimeout(resolve, 1000));
}
if (Date.now() >= deadline) { console.error('长期检查超时；保留报告，不判通过。'); process.exitCode = 1; }
} finally { await stopOwnedChild(child); }
