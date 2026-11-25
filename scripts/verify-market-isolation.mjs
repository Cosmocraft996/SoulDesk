import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { stopOwnedChild } from './child-cleanup.mjs';

const executable = resolve(process.argv[2] ?? 'src-tauri/target/release/souldesk.exe');
if (!existsSync(executable)) throw new Error('请先构建正式程序');

mkdirSync('artifacts', { recursive: true });
const root = resolve(`artifacts/desktop-integration-market-isolation-${randomUUID()}`);
const report = `${root}.json`;
const output = openSync(`${report}.log`, 'wx');
console.log(`行情进程隔离验收 ${report}`);
const desktop = spawn(executable, ['--desktop-integration-market-isolation', report], {
  windowsHide: true,
  stdio: ['ignore', output, output],
});

try {
  const code = await Promise.race([
    new Promise((resolve, reject) => {
      desktop.once('exit', resolve);
      desktop.once('error', reject);
    }),
    new Promise((_, reject) => {
      setTimeout(() => reject(new Error('进程隔离验收超过 60 秒')), 60_000).unref();
    }),
  ]);
  const value = existsSync(report) ? JSON.parse(readFileSync(report, 'utf8')) : null;
  if (code !== 0 || value?.status !== 'passed' || !value.recovery || !value.workerCleanup) {
    throw new Error(value?.error ?? `测试程序退出 ${code}，未生成通过报告`);
  }
  console.log(`通过。主界面阻塞期间，独立通知进程在触线后 ${value.evidence.showLatencyMs} 毫秒显示灵动通知。`);
  const recoveryDelay = value.recovery.shown.at - value.recovery.source.triggeredAt;
  console.log(`通过。行情核心意外退出后 ${recoveryDelay} 毫秒内显示补报（自原触发时刻计），标记价缺口仅记录。`);
} finally {
  if (existsSync(join(root, 'isolated-test-data.marker'))) {
    writeFileSync(join(root, 'stop-isolation.marker'), '');
  }
  await stopOwnedChild(desktop);
}
