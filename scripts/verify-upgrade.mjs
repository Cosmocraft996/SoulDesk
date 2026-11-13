import { DatabaseSync } from 'node:sqlite';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const digest = value => createHash('sha256').update(value).digest('hex');
const fileHash = path => digest(readFileSync(path));

// 查询白名单不含警报运行状态或事件历史；配置与组件存储原值只在内存中参与散列。
export function configurationSummary(path) {
  if (!existsSync(path)) return { exists: false };
  const db = new DatabaseSync(path, { readOnly: true, timeout: 5000 });
  try {
    db.exec('BEGIN');
    const integrity = db.prepare('PRAGMA quick_check').get().quick_check;
    if (integrity !== 'ok') throw new Error('正常数据库完整性检查失败，未安装或改写数据');
    const version = db.prepare('PRAGMA user_version').get().user_version;
    if (![2, 3].includes(version)) throw new Error('升级摘要仅支持已有v2/v3库；未修改原数据');
    const queries = {
      app_settings: 'SELECT key,value FROM app_settings ORDER BY key',
      widget_packages: 'SELECT id,manifest,directory,digest FROM widget_packages ORDER BY id',
      widget_instances: 'SELECT id,package_id,settings,storage,floating,position,width,height FROM widget_instances ORDER BY id',
      window_layouts: 'SELECT label,geometry FROM window_layouts ORDER BY label',
      watchlist: 'SELECT instance_id,symbol,position FROM watchlist ORDER BY instance_id,symbol',
      alert_rules: 'SELECT id,revision,definition,enabled,market_key FROM alert_rules ORDER BY id',
    };
    const tables = {};
    for (const [table, query] of Object.entries(queries)) {
      const available = db.prepare('SELECT 1 FROM sqlite_schema WHERE type=\'table\' AND name=?').get(table);
      if (!available && !(version === 2 && ['watchlist', 'alert_rules'].includes(table))) throw new Error(`正常数据库缺少表${table}，未继续安装`);
      const rows = available ? db.prepare(query).all() : [];
      tables[table] = { rows: rows.length, sha256: digest(JSON.stringify(rows)) };
    }
    const configurationRows = Object.values(tables).reduce((total, table) => total + table.rows, 0);
    return { exists: true, version, integrity, tables, configurationRows, hasSavedConfiguration: configurationRows > 0, configurationSha256: digest(JSON.stringify(tables)) };
  } finally { db.close(); }
}

function powershell(args) {
  const value = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', ...args], { encoding: 'utf8', windowsHide: true });
  if (value.status !== 0 || value.error) throw new Error('无法核对普通桌面环境或进程状态；本次停止');
  return value.stdout.trim();
}
async function waitForTrayExit() {
  const count = () => {
    const raw = powershell(['-Command', "@(Get-Process -Name souldesk,souldesk-soak -ErrorAction SilentlyContinue).Count"]);
    if (!/^\d+$/.test(raw)) throw new Error('进程计数不可用，未继续安装');
    const value = Number(raw);
    if (!Number.isInteger(value) || value < 0) throw new Error('进程计数无效，未继续安装');
    return value;
  };
  const until = Date.now() + 180_000;
  let told = false;
  while (count() > 0) {
    if (!told) { console.log('请右键SoulDesk托盘图标并退出；此入口不会结束你的常驻程序。'); told = true; }
    if (Date.now() >= until) throw new Error('等待托盘退出超时，未继续；保留报告后可重新运行');
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
}
async function run(file, args, onOutput) {
  const child = spawn(file, args, { stdio: onOutput ? ['ignore', 'pipe', 'inherit'] : 'inherit', windowsHide: !!onOutput });
  if (onOutput) child.stdout.on('data', data => { process.stdout.write(data); onOutput(data.toString()); });
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(new Error(`子步骤退出码${code}，未判通过`)));
  });
}

async function main() {
  if (process.platform !== 'win32') throw new Error('此入口仅用于Windows普通桌面');
  const context = JSON.parse(powershell(['-File', fileURLToPath(new URL('./windows-package-identity.ps1', import.meta.url))]));
  if (context.packageFamily) throw new Error('请在资源管理器双击verify-upgrade.cmd；当前继承外部MSIX身份，本次未安装或读取正常数据');
  const root = fileURLToPath(new URL('../', import.meta.url));
  process.chdir(root);
  const manifest = JSON.parse(readFileSync('artifacts/delivery-manifest-g3-032.json', 'utf8'));
  const installer = resolve(manifest.installer.path);
  if (fileHash(installer) !== manifest.installer.sha256) throw new Error('安装包散列与清单不符，未启动安装');
  const exe = join(process.env.LOCALAPPDATA, 'SoulDesk', 'souldesk.exe');
  const identifier = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).identifier;
  // 与Rust setup中的app_local_data_dir一致；不使用Roaming目录。
  const database = join(process.env.LOCALAPPDATA, identifier, 'workbench.sqlite');
  const runId = randomUUID(), reportPath = resolve(`artifacts/g3-upgrade-${runId}.json`);
  const report = { runId, version: manifest.version, context, started: Date.now(), passed: false, scope: '正常数据只读摘要；安装器实际升级；G3业务探针使用隔离数据，非长期运行或旧安装器回退验收' };
  const save = () => writeFileSync(reportPath, JSON.stringify(report, null, 2));
  save(); console.log(`升级报告：${reportPath}`);
  try {
    await waitForTrayExit();
    report.beforeExecutableSha256 = existsSync(exe) ? fileHash(exe) : null;
    report.before = configurationSummary(database); save();
    console.log('升级前摘要已保存（只含数量和散列）。现在打开安装器，请按默认选项完成安装。');
    await run(installer, []);
    console.log('安装结束。若程序已在托盘常驻，请先退出；随后自动进行约三分钟的视图检查。');
    await waitForTrayExit();
    if (fileHash(exe) !== manifest.executable.sha256) throw new Error('安装后的程序散列不符，可能取消安装或选了其他安装目录');
    report.afterExecutableSha256 = fileHash(exe);
    report.after = configurationSummary(database);
    report.existingDataPresent = report.before.exists;
    report.configurationPreserved = report.before.exists ? report.after.exists && report.before.configurationSha256 === report.after.configurationSha256 : null;
    report.nonEmptyConfigurationPreservationVerified = report.before.hasSavedConfiguration === true && report.configurationPreserved === true;
    if (report.before.exists && !report.configurationPreserved) throw new Error('正常配置的前后摘要不同；保留现有文件，不自动回退或重置，请复核报告');
    report.installerReplacementVerified = report.beforeExecutableSha256 !== null && report.beforeExecutableSha256 !== report.afterExecutableSha256;
    save();
    let output = '';
    await run(process.execPath, ['scripts/verify-installed.mjs', exe, '--g3'], text => { output += text; });
    const match = output.match(/g3-installed-([a-f0-9-]{36})\.json/);
    if (!match) throw new Error('缺少当次G3报告路径，未判通过');
    const g3Path = resolve('artifacts', basename(`g3-installed-${match[1]}.json`));
    const g3 = JSON.parse(readFileSync(g3Path, 'utf8'));
    if (g3.runId !== match[1] || g3.g3ChecksComplete !== true || g3.failure) throw new Error('当次G3完成证据不符');
    report.g3Report = g3Path;
    report.finished = Date.now(); report.passed = true; save();
    console.log(report.nonEmptyConfigurationPreservationVerified ? '安装升级及视图专项完成，已有非空配置摘要保持。' : report.before.exists ? '安装及视图专项完成；安装前是空库，不宣称验证了非空配置保持。' : '安装及视图专项完成；安装前没有正常数据库，不宣称验证了已有配置保持。');
  } catch (error) { report.failure = error.message; save(); throw error; }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
