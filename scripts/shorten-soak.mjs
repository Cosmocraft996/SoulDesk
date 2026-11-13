// 按用户调整收束已有运行；保留原报告，另存缩短后的实际验收结果。
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { resolve, dirname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
async function readReport(path) {
  for (let attempt = 0; ; attempt++) { try { return JSON.parse(readFileSync(path, 'utf8')); } catch (error) { if (attempt >= 4) throw error; await new Promise(resolve => setTimeout(resolve, 100)); } }
}

function summary(values) {
  const sorted = [...values].sort((a, b) => a - b), middle = Math.floor(sorted.length / 2);
  return { count: values.length, median: sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2, max: sorted.at(-1), first: values[0], last: values.at(-1), growth: values.at(-1) - values[0] };
}
export function assessAdjustedRun(report, rows, seconds = 1800) {
  if (report.phase !== 'g3-soak' || report.failure || report.elapsedSeconds < seconds || rows.at(-1)?.elapsedSeconds < seconds || rows[0]?.elapsedSeconds !== 0 || rows.length < Math.floor(seconds / 30)) throw new Error('尚无完整30分钟采样证据');
  if (report.healthObservationGapsOver3s !== 0 || report.samplingGapsOver90s !== 0) throw new Error('观察存在缺口，不能因缩短目标而忽略');
  const events = rows[0].events;
  for (const [i, row] of rows.entries()) {
    if ((i > 0 && (row.elapsedSeconds <= rows[i - 1].elapsedSeconds || row.elapsedSeconds - rows[i - 1].elapsedSeconds > 90)) || row.events !== events || row.queueOverflows !== 0 || row.streamCount !== 62 || ['views', 'sessions', 'temporarySubscriptions', 'windowTasks', 'trials'].some(key => row[key] !== 0)) throw new Error('资源、事件或采样序列边界不符');
  }
  if (rows.at(-1).healthy !== true) throw new Error('收束时必要行情尚未恢复');
  const warmed = rows.filter(r => r.elapsedSeconds >= 300);
  if (!warmed.length || warmed.some(r => !Number.isFinite(r.process.privateBytes) || !Number.isFinite(r.process.workingSetBytes) || !Number.isFinite(r.process.cpuPercentOneCore))) throw new Error('预热后的资源采样无效');
  const memory = summary(warmed.map(r => r.process.privateBytes));
  const review = memory.growth > 64 * 1048576 && memory.growth > memory.first * 0.25;
  return { passed: !review, effectiveTargetSeconds: seconds, observedSeconds: report.elapsedSeconds, full24Hours: false, resourceReviewRequired: review, sampleCount: rows.length, eventCount: events, privateBytesSummary: memory, workingSetSummary: summary(warmed.map(r => r.process.workingSetBytes)), cpuPercentOneCoreSummary: summary(warmed.map(r => r.process.cpuPercentOneCore)) };
}

async function main() {
  const path = resolve(process.argv[2] ?? '');
  const artifacts = resolve('artifacts');
  if (!path.startsWith(artifacts + sep) || dirname(path) !== artifacts) throw new Error('只收束当前项目artifacts下明确的测试报告');
  let report = await readReport(path);
  const runId = report.runId;
  if (!/^[0-9a-f-]{36}$/.test(runId) || report.phase !== 'g3-soak' || report.soakSeconds !== 86400) throw new Error('目标不是本次原24小时测试');
  const launch = JSON.parse(readFileSync(resolve(artifacts, `g3-launch-${runId}.json`), 'utf8'));
  if (launch.packageFamily !== null || launch.sha256 !== report.executableSha256 || launch.executable !== report.executable) throw new Error('原始启动身份或程序不一致');
  const output = resolve(artifacts, `g3-adjusted-${runId}.json`);
  const base = { phase: 'g3-adjusted-soak', runId, originalReport: path, originalTargetSeconds: 86400, effectiveTargetSeconds: 1800, full24Hours: false, reason: '用户将本轮验收改为30分钟，长期运行由用户日常使用时自行验证', originalReportPreserved: true };
  const save = extra => writeFileSync(output, JSON.stringify({ ...base, ...extra }, null, 2) + '\n');
  save({ passed: false, stage: '沿用已有样本，等待累计30分钟' });
  console.log(`结果：${output}\n当前${report.elapsedSeconds}秒，累计1800秒后只结束该测试进程。`);
  let nextProgress = 0;
  while (true) {
    report = await readReport(path);
    if (report.runId !== runId || report.executableSha256 !== launch.sha256) throw new Error('测试身份发生变化，停止自动处理');
    if (report.failure) { save({ passed: false, stage: '原始运行已失败', failure: report.failure }); throw new Error(report.failure); }
    if (report.elapsedSeconds >= 1800 || Date.now() >= report.soakStarted + 1830000) break;
    if (report.elapsedSeconds >= nextProgress) { console.log(`已采样${report.elapsedSeconds}秒，行情健康=${report.latestSample?.healthy}，队列溢出=${report.latestSample?.queueOverflows}`); nextProgress = report.elapsedSeconds + 300; }
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  const pid = report.latestSample?.process?.pid;
  if (!Number.isInteger(pid) || pid <= 0) throw new Error('缺少本次明确进程标识');
  const stop = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./stop-soak-process.ps1', import.meta.url)), '-ReportPath', path, '-ExpectedRunId', runId, '-ExpectedPid', String(pid), '-ExpectedSha256', launch.sha256], { encoding: 'utf8', windowsHide: true });
  if (stop.status !== 0) { save({ passed: false, stage: '进程身份核对或停止失败', failure: stop.stderr.trim() }); throw new Error(stop.stderr.trim() || '测试进程未收束'); }
  report = JSON.parse(readFileSync(path, 'utf8'));
  const samplePath = resolve(report.sampleFile);
  if (samplePath !== path.replace(/\.json$/, '.samples.jsonl')) throw new Error('采样文件不属于本次报告');
  const rows = readFileSync(samplePath, 'utf8').trim().split(/\r?\n/).map(JSON.parse);
  try {
    const result = assessAdjustedRun(report, rows);
    save({ ...result, stage: result.passed ? '按用户调整完成30分钟验收' : '30分钟采样完成，资源增长需复核', finished: Date.now(), stopMethod: '核对程序路径、运行标识、启动时间及散列后，仅结束该隔离测试进程', originalReportSha256: createHash('sha256').update(readFileSync(path)).digest('hex'), terminalNote: '旧24小时启动器可能提示提前退出；本次由用户调整目标，正式结果以此补充报告为准' });
    console.log(result.passed ? '30分钟验收通过；未完成、也不宣称24小时验证。' : '资源增长需复核，未判通过。');
    if (!result.passed) process.exitCode = 1;
  } catch (error) { save({ passed: false, stage: '已按用户时长要求停止，采样证据未通过', finished: Date.now(), failure: String(error) }); throw error; }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
