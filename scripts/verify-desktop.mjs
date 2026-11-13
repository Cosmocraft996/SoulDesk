// 运行正式程序本体的隔离集成，不安装、不覆盖正常数据，不复跑旧平台探针。
import { spawn } from 'node:child_process';
import { existsSync, readFileSync, mkdirSync, openSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';
const parameters = process.argv.slice(2);
const surface = parameters[0] === '--surface';
const executable = resolve((surface ? parameters[2] : parameters[0]) ?? 'src-tauri/target/release/souldesk.exe');
if (!existsSync(executable)) throw new Error('正式程序不存在，请先构建');
mkdirSync('artifacts', { recursive: true });
const report = surface ? resolve(parameters[1] ?? '') : resolve(`artifacts/desktop-integration-${randomUUID()}.json`);
const read = () => { try { return JSON.parse(readFileSync(report, 'utf8')); } catch { return undefined; } };
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function launch(flag) {
  const phase = flag.endsWith('surface') ? `surface-${randomUUID()}` : flag.endsWith('resume') ? 'resume' : 'first';
  const output = openSync(`${report}.${phase}.log`, 'wx');
  const child = spawn(executable, [flag, report], { cwd: process.cwd(), windowsHide: true, stdio: ['ignore', output, output] });
  let exit; child.once('exit', code => { exit = code; }); child.once('error', error => { console.error(error); exit = 1; });
  let previous = '', count = 0, sawRunning = false;
  const initialChecks = read()?.checks?.length ?? 0;
  for (let n = 0; n < 360; n++) {
    const value = read();
    const current = `${value?.status ?? 'starting'} / ${value?.checks?.length ?? 0}`;
    if (current !== previous) { console.log(current); previous = current; }
    if (value?.status === 'running') sawRunning = true;
    if (value?.status === 'failed') throw new Error(`${value.error}\n报告：${report}\n隔离程序保留，便于定位。PID=${child.pid}`);
    if ((flag.endsWith('resume') || surface) && value?.status === 'automatic-complete-manual-observation-pending' && (sawRunning || value.checks.length > initialChecks)) {
      console.log(`正式集成自动部分完成，人工观察窗口正在运行。PID=${child.pid}；请观察后从托盘退出。`);
      // 保持启动器存活，避免桌面工具的作业对象在命令结束时回收图形子进程。
      await new Promise(resolve => child.once('exit', resolve)); return;
    }
    if (exit !== undefined) { if (exit !== 0 || value?.status !== 'restart-ready') throw new Error(`程序退出 ${exit}，报告：${report}`); return; }
    count++; await delay(1000);
  }
  throw new Error(`等待超过${count}秒，未继续自动重试。PID=${child.pid}，报告：${report}`);
}
console.log(`隔离报告：${report}`);
if (surface) await launch('--desktop-integration-surface');
else { await launch('--desktop-integration'); await launch('--desktop-integration-resume'); }
console.log(`报告：${report}`);
