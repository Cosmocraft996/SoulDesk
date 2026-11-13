// 只启动并回收本次隔离进程；不停止正在运行的正常安装版。
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, openSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { stopOwnedChild } from './child-cleanup.mjs';
const executable = resolve(process.argv[2] ?? 'src-tauri/target/release/souldesk.exe');
if (!existsSync(executable)) throw new Error('请先构建正式程序');
mkdirSync('artifacts', { recursive: true });
const report = resolve(`artifacts/desktop-integration-panel-${randomUUID()}.json`);
const output = openSync(`${report}.log`, 'wx');
console.log(`面板隔离检查：${report}`);
const child = spawn(executable, ['--desktop-integration-panel', report], { windowsHide: true, stdio: ['ignore', output, output] });
try {
  await Promise.race([
    new Promise((resolve, reject) => { child.once('exit', code => code === 0 ? resolve() : reject(new Error(`本次隔离进程退出 ${code}`))); child.once('error', reject); }),
    new Promise((_, reject) => { const timeout = setTimeout(() => reject(new Error('面板隔离检查超过60秒，停止本次进程')), 60_000); timeout.unref(); }),
  ]);
  const value = JSON.parse(readFileSync(report, 'utf8'));
  if (value.status !== 'passed' || !value.checks.length || value.checks.some(c => !c.passed)) throw new Error(`面板检查未通过：${value.error ?? value.status}`);
  console.log(`通过 ${value.checks.length} 项：真实组件加载、原生滑动、会话释放、手动展开及桌面布局保持。`);
  console.log('未模拟真实鼠标双击、拖拽；未安装或触碰正常用户库。');
} finally { await stopOwnedChild(child); }
