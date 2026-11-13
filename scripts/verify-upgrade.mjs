import { DatabaseSync } from 'node:sqlite';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createInterface } from 'node:readline/promises';

const digest = value => createHash('sha256').update(value).digest('hex');
const fileHash = path => digest(readFileSync(path));

// Tauri打包时将运行时安装类型从UNK写为NSS；其余字节必须完全相同。
export function bundleMarkerEquivalent(build, packaged) {
  if (build.equals(packaged)) return true;
  const source = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
  const target = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS');
  for (let at = build.indexOf(source); at >= 0; at = build.indexOf(source, at + 1)) {
    const expected = Buffer.from(build); target.copy(expected, at);
    if (expected.equals(packaged)) return true;
  }
  return false;
}
export function canResumeUpgrade(previous, installedHash, expectedHash) {
  return previous?.version === '0.4.0' && previous?.before?.exists === true && previous.before.version < 4 &&
    typeof previous.before.database === 'string' && previous.programChecksComplete !== true &&
    installedHash === expectedHash && previous.beforeExecutableSha256 !== expectedHash;
}

// 查询白名单不含警报运行状态或事件历史；配置与组件存储原值只在内存中参与散列。
export function configurationSummary(path) {
  if (!existsSync(path)) return { exists: false };
  const db = new DatabaseSync(path, { readOnly: true, timeout: 5000 });
  try {
    db.exec('BEGIN');
    const integrity = db.prepare('PRAGMA quick_check').get().quick_check;
    if (integrity !== 'ok') throw new Error('正常数据库完整性检查失败，未安装或改写数据');
    const version = db.prepare('PRAGMA user_version').get().user_version;
    if (![1, 2, 3, 4].includes(version)) throw new Error('升级摘要仅支持已有v1～v4库；未修改原数据');
    const queries = {
      app_settings: 'SELECT key,value FROM app_settings ORDER BY key',
      widget_packages: 'SELECT id,manifest,directory,digest FROM widget_packages ORDER BY id',
      widget_instances: `SELECT id,package_id,settings,storage,floating${version >= 2 ? ',position,width,height' : ''}${version >= 4 ? ',location,visible,pinned' : ''} FROM widget_instances ORDER BY id`,
      window_layouts: 'SELECT label,geometry FROM window_layouts ORDER BY label',
      watchlist: 'SELECT instance_id,symbol,position FROM watchlist ORDER BY instance_id,symbol',
      alert_rules: 'SELECT id,revision,definition,enabled,market_key FROM alert_rules ORDER BY id',
    };
    const tables = {};
    for (const [table, query] of Object.entries(queries)) {
      const available = db.prepare('SELECT 1 FROM sqlite_schema WHERE type=\'table\' AND name=?').get(table);
      if (!available && !((version < 3 && ['watchlist', 'alert_rules'].includes(table)) || (version < 2 && table === 'window_layouts'))) throw new Error(`正常数据库缺少表${table}，未继续安装`);
      const rows = available ? db.prepare(query).all() : [];
      tables[table] = { rows: rows.length, sha256: digest(JSON.stringify(rows)) };
    }
    const configurationRows = Object.values(tables).reduce((total, table) => total + table.rows, 0);
    return { exists: true, version, integrity, tables, configurationRows, hasSavedConfiguration: configurationRows > 0, configurationSha256: digest(JSON.stringify(tables)) };
  } finally { db.close(); }
}

function powershell(args) {
  const value = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', ...args], { encoding: 'utf8', windowsHide: true });
  if (value.status !== 0 || value.error) throw new Error(`无法核对普通桌面环境或进程状态；本次停止：${(value.error?.message || value.stderr || `退出码${value.status}`).trim().slice(-1200)}`);
  return value.stdout.trim();
}
const identityScript = fileURLToPath(new URL('./windows-package-identity.ps1', import.meta.url));
const processes = () => JSON.parse(powershell(['-File', identityScript, '-SoulDeskProcesses']));
const assertStopped = () => { if (processes().length) throw new Error('SoulDesk尚未完全退出，未复制或改写正常数据；请从托盘退出后重试'); };
async function waitForTrayExit() {
  const until = Date.now() + 900_000;
  let told = false;
  while (processes().length > 0) {
    if (!told) { console.log('请右键SoulDesk托盘图标并退出；此入口不会结束你的常驻程序。'); told = true; }
    if (Date.now() >= until) throw new Error('等待托盘退出超时，保留报告和全部备份，不自动结束程序');
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
}
async function run(file, args) {
  // 安装器必须可见；隐藏窗口只用于只读PowerShell辅助进程。
  const child = spawn(file, args, { stdio: 'inherit', windowsHide: false });
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(new Error(`子步骤退出码${code}，未判通过`)));
  });
}

// 只保留数据库及组件资源；不复制浏览器资料、日志或整个用户目录。
function inventory(root) {
  const files = [];
  function visit(path) {
    if (!existsSync(path)) return;
    const stat = lstatSync(path);
    if (stat.isSymbolicLink()) throw new Error('备份范围包含链接，已停止；不跟随到其他用户目录');
    if (stat.isDirectory()) for (const name of readdirSync(path).sort()) visit(join(path, name));
    else if (stat.isFile()) files.push({ path: relative(root, path), bytes: stat.size, sha256: fileHash(path) });
  }
  for (const name of ['workbench.sqlite', 'workbench.sqlite-wal', 'workbench.sqlite-shm', 'widgets']) visit(join(root, name));
  return files;
}
function copyFiles(source, target, files) {
  mkdirSync(target, { recursive: true });
  for (const file of files) {
    const to = join(target, file.path);
    mkdirSync(dirname(to), { recursive: true });
    copyFileSync(join(source, file.path), to);
    if (fileHash(to) !== file.sha256) throw new Error('停写副本散列校验失败，未继续安装');
  }
}
function contentDigest(path) {
  const db = new DatabaseSync(path, { readOnly: true });
  try {
    const quote = name => `"${name.replaceAll('"', '""')}"`;
    const tables = db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all();
    return digest(JSON.stringify(tables.map(({ name }) => [name, db.prepare(`SELECT * FROM ${quote(name)}`).all().map(row => JSON.stringify(row)).sort()])));
  } finally { db.close(); }
}
export function snapshotStoppedData(source, target) {
  const files = inventory(source);
  const raw = join(target, 'stopped-data');
  copyFiles(source, raw, files);
  if (JSON.stringify(files) !== JSON.stringify(inventory(source))) throw new Error('备份期间源文件发生变化，未继续安装');
  if (!existsSync(join(raw, 'workbench.sqlite'))) {
    if (files.some(file => file.path.startsWith('workbench.sqlite-'))) throw new Error('仅有数据库侧文件，缺少主文件，未继续安装');
    return { exists: false, files, raw };
  }
  // SQLite仅打开第二份工作副本；原库和原始停写副本保持逐文件可校验。
  const working = join(target, 'sqlite-working-copy');
  copyFiles(raw, working, files.filter(file => file.path.startsWith('workbench.sqlite')));
  const database = join(target, 'database.sqlite');
  const db = new DatabaseSync(join(working, 'workbench.sqlite'), { readOnly: true, timeout: 5000 });
  try { db.prepare('VACUUM INTO ?').run(database); } finally { db.close(); }
  const check = new DatabaseSync(database, { readOnly: true });
  try { if (check.prepare('PRAGMA integrity_check').get().integrity_check !== 'ok') throw new Error('一致备份完整性校验失败'); } finally { check.close(); }
  const summary = configurationSummary(database);
  if (contentDigest(join(working, 'workbench.sqlite')) !== contentDigest(database)) throw new Error('一致备份内容与停写副本不同');
  return { ...summary, files, raw, database, databaseSha256: fileHash(database), contentSha256: contentDigest(database) };
}

// 迁移新增hidden_all及新列不等于配置丢失；事件递送/规则运行状态允许正常推进。
export function compareSavedData(beforePath, afterPath) {
  const before = new DatabaseSync(beforePath, { readOnly: true });
  const after = new DatabaseSync(afterPath, { readOnly: true });
  try {
    const checks = {};
    for (const [table, columns] of Object.entries({
      app_settings: 'key,value', widget_packages: 'id,manifest,directory,digest',
      widget_instances: 'id,package_id,settings,storage,floating,position,width,height,location,visible,pinned',
      watchlist: 'instance_id,symbol,position', alert_rules: 'id,revision,definition,enabled,market_key',
      widget_versions: 'package_id,digest,manifest,directory,created',
      widget_upgrade_snapshots: 'package_id,from_digest,to_digest,instance_id,settings,storage,created',
      alert_events: 'id,rule_id,revision,sequence,event,created', test_events: 'id,created',
    })) {
      if (!before.prepare("SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?").get(table)) continue;
      const available = new Set(before.prepare(`PRAGMA table_info(${table})`).all().map(row => row.name));
      const selected = columns.split(',').filter(column => available.has(column)).join(',');
      const previous = before.prepare(`SELECT ${selected} FROM ${table}`).all().map(row => JSON.stringify(row));
      const current = after.prepare(`SELECT ${selected} FROM ${table}`).all().map(row => JSON.stringify(row));
      const counts = new Map();
      for (const row of current) counts.set(row, (counts.get(row) ?? 0) + 1);
      const retained = previous.every(row => { const count = counts.get(row) ?? 0; counts.set(row, count - 1); return count > 0; });
      const allowAdditional = ['app_settings', 'alert_events', 'test_events'].includes(table);
      checks[table] = { beforeRows: previous.length, afterRows: current.length, retained: retained && (allowAdditional || previous.length === current.length) };
    }
    let completedRules = 0, completedRulesPreserved = true;
    if (before.prepare("SELECT 1 FROM sqlite_schema WHERE name='alert_rules'").get()) {
      for (const rule of before.prepare('SELECT id,state FROM alert_rules').all()) {
        if (JSON.parse(rule.state).completed !== true) continue;
        completedRules++;
        const current = after.prepare('SELECT state FROM alert_rules WHERE id=?').get(rule.id);
        if (!current || JSON.parse(current.state).completed !== true) completedRulesPreserved = false;
        const previousEvents = before.prepare('SELECT count(*) AS count FROM alert_events WHERE rule_id=?').get(rule.id).count;
        if (after.prepare('SELECT count(*) AS count FROM alert_events WHERE rule_id=?').get(rule.id).count !== previousEvents) completedRulesPreserved = false;
      }
    }
    return { preserved: Object.values(checks).every(check => check.retained) && completedRulesPreserved, tables: checks, completedRules, completedRulesPreserved };
  } finally { before.close(); after.close(); }
}

async function startNormal(exe, expectedHash) {
  let live = processes();
  if (!live.length) {
    const child = spawn(exe, [], { windowsHide: true, stdio: 'ignore' });
    let failure; child.once('error', error => { failure = error; });
    for (let attempt = 0; attempt < 20 && !live.length; attempt++) {
      if (failure) throw failure;
      await new Promise(resolve => setTimeout(resolve, 500)); live = processes();
      if (child.exitCode !== null && !live.length) throw new Error('正常程序启动后提前退出，保留数据和报告');
    }
  }
  if (live.length !== 1 || live[0].packageFamily !== null || resolve(live[0].executable).toLowerCase() !== exe.toLowerCase() || /--(?:g0-|desktop-)/.test(live[0].commandLine) || fileHash(live[0].executable) !== expectedHash) throw new Error('实际运行路径、散列、隔离参数或MSIX身份不符，未判普通安装通过');
  return { ...live[0], executableSha256: expectedHash, physicalExecutable: realpathSync.native(exe) };
}

async function main() {
  if (process.platform !== 'win32') throw new Error('此入口仅用于Windows普通桌面');
  const context = JSON.parse(powershell(['-File', identityScript]));
  if (context.packageFamily) throw new Error('请从独立资源管理器双击scripts\\verify-upgrade.cmd；当前继承外部MSIX身份，未安装或读取正常数据');
  process.chdir(fileURLToPath(new URL('../', import.meta.url)));
  const manifest = JSON.parse(readFileSync('artifacts/delivery-manifest-desktop-040.json', 'utf8'));
  const candidate = manifest.files.find(file => file.path === 'artifacts/SoulDesk_0.4.0_x64-setup.exe');
  const buildExecutable = manifest.files.find(file => file.path === 'src-tauri/target/release/souldesk.exe');
  const executable = manifest.installedExecutable;
  if (manifest.version !== '0.4.0' || manifest.schemaVersion !== 4 || !candidate || !buildExecutable || !executable || fileHash(candidate.path) !== candidate.sha256 || fileHash(buildExecutable.path) !== buildExecutable.sha256 || fileHash(executable.path) !== executable.sha256 || !bundleMarkerEquivalent(readFileSync(buildExecutable.path), readFileSync(executable.path))) throw new Error('0.4.0候选、安装器内程序或清单不符，未继续安装');
  const registration = JSON.parse(powershell(['-Command', "$entry=Get-ItemProperty -LiteralPath 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\SoulDesk' -ErrorAction SilentlyContinue; if($entry){[pscustomobject]@{version=$entry.DisplayVersion;directory=$entry.InstallLocation} | ConvertTo-Json -Compress}else{'null'}"]));
  const installedDirectory = registration?.directory?.replace(/^"|"$/g, '') || join(process.env.LOCALAPPDATA, 'SoulDesk');
  const exe = resolve(installedDirectory, 'souldesk.exe');
  const dataRoot = resolve(process.env.LOCALAPPDATA, 'local.souldesk.desktop');
  const previousPointer = existsSync('artifacts/desktop-upgrade-current.json') ? JSON.parse(readFileSync('artifacts/desktop-upgrade-current.json', 'utf8')) : null;
  const previous = previousPointer && existsSync(previousPointer.reportPath) ? JSON.parse(readFileSync(previousPointer.reportPath, 'utf8')) : null;
  const resuming = canResumeUpgrade(previous, existsSync(exe) ? fileHash(exe) : null, executable.sha256);
  if (process.argv.includes('--resume-only') && !resuming) throw new Error('续验入口未找到匹配的安装程序及v3备份；已停止，不调用安装器');
  const runId = randomUUID(), folder = resolve(`artifacts/desktop-upgrade-${runId}`), reportPath = `${folder}.json`;
  mkdirSync(folder);
  const report = { runId, version: manifest.version, context, registration, started: Date.now(), stage: 'preparing', passed: false, installed: false, realDowngradePerformed: false, dataRoot, physicalDataRoot: existsSync(dataRoot) ? realpathSync.native(dataRoot) : null, scope: '普通安装、正常库升级和重启；不重跑历史G3探针；完整G3及真实回退仍单列' };
  const save = () => writeFileSync(reportPath, JSON.stringify(report, null, 2));
  const input = createInterface({ input: process.stdin, output: process.stdout });
  save(); writeFileSync('artifacts/desktop-upgrade-current.json', JSON.stringify({ runId, reportPath }, null, 2));
  console.log(`报告：${reportPath}`);
  try {
    if (resuming) {
      if (previous.context.packageFamily !== null || resolve(previous.dataRoot).toLowerCase() !== dataRoot.toLowerCase() || fileHash(previous.before.database) !== previous.before.databaseSha256 || contentDigest(previous.before.database) !== previous.before.contentSha256) throw new Error('原普通升级报告或安装前备份不匹配，未恢复验收');
      report.resumesReport = previousPointer.reportPath;
      report.before = previous.before;
      report.beforeExecutableSha256 = previous.beforeExecutableSha256;
      report.beforeProgramFiles = previous.beforeProgramFiles;
      report.beforeExecutablePhysicalPath = previous.beforeExecutablePhysicalPath;
      console.log('已识别安装器内的0.4.0程序，沿用原v3备份继续验收，不调用安装器。');
    } else {
    assertStopped();
    report.before = snapshotStoppedData(dataRoot, join(folder, 'before'));
    report.beforeExecutableSha256 = existsSync(exe) ? fileHash(exe) : null;
    if (report.beforeExecutableSha256) {
      const program = join(folder, 'before', 'program'); mkdirSync(program);
      report.beforeProgramFiles = [];
      for (const name of ['souldesk.exe', 'notification-icon.png', 'uninstall.exe']) if (existsSync(join(dirname(exe), name))) {
        const sha256 = fileHash(join(dirname(exe), name)); copyFileSync(join(dirname(exe), name), join(program, name));
        if (fileHash(join(program, name)) !== sha256) throw new Error('旧程序副本校验失败');
        report.beforeProgramFiles.push({ path: join(program, name), sha256 });
      }
      report.beforeExecutablePhysicalPath = realpathSync.native(exe);
    }
    assertStopped(); report.stage = 'backup-verified-awaiting-user-install'; save();
    console.log(`停写副本和一致备份已校验：${join(folder, 'before')}\n旧库：${report.before.exists ? `v${report.before.version}，配置${report.before.configurationRows}条` : '不存在'}。`);
    if (process.argv.includes('--prepare-only')) return;
    if ((await input.question('输入“安装”继续普通升级；输入其他内容则仅保留备份、不安装：')).trim() !== '安装') { report.stage = 'prepared-not-installed'; save(); return; }
    assertStopped();
    if (JSON.stringify(inventory(dataRoot)) !== JSON.stringify(report.before.files) || (existsSync(exe) ? fileHash(exe) : null) !== report.beforeExecutableSha256 || fileHash(candidate.path) !== candidate.sha256) throw new Error('确认期间源数据、旧程序或候选发生变化，未启动安装');
    report.stage = 'installer-running'; save();
    console.log('请按默认目录完成安装。首次启动先不要添加组件、修改设置或升级组件包，加载后从托盘退出。');
    // 现有安装使用NSIS更新模式，避开卸载页及其“删除应用数据”选项；不使用静默参数。
    await run(resolve(candidate.path), report.beforeExecutableSha256 ? ['/UPDATE'] : []);
    }
    if (!existsSync(exe) || fileHash(exe) !== executable.sha256) throw new Error('安装后的程序散列不符，可能取消安装或选择了其他目录');
    report.installed = true; report.installerReplacementVerified = report.beforeExecutableSha256 !== null && report.beforeExecutableSha256 !== executable.sha256;
    if (resuming && previous.firstRun?.executableSha256 === executable.sha256 && previous.firstRun.packageFamily === null && processes().length === 0) {
      report.firstRun = previous.firstRun; report.normalExitObserved = true;
      console.log('已沿用之前的普通运行身份记录，当前程序已退出，直接核对停写数据。');
    } else {
      if (process.argv.includes('--data-only')) throw new Error('只读数据入口要求程序先从托盘退出，且已有普通运行身份记录；不会自动启动程序');
      report.firstRun = await startNormal(exe, executable.sha256); report.stage = 'first-start-awaiting-tray-exit'; save();
      console.log('已核对普通运行身份和程序散列。请先不要修改设置；当前组件加载后从托盘退出，接着核对真实迁移数据。');
      await waitForTrayExit(); report.normalExitObserved = true;
    }
    report.after = snapshotStoppedData(dataRoot, join(folder, 'after-first-start'));
    if (report.after.version !== 4) throw new Error('正常库尚未迁移至v4');
    report.migration = report.before.exists ? compareSavedData(report.before.database, report.after.database) : null;
    const resourceFiles = report.before.files.filter(file => file.path.startsWith(`widgets/`) || file.path.startsWith(`widgets\\`));
    report.resources = { beforeFiles: resourceFiles.length, preserved: resourceFiles.every(file => report.after.files.some(current => current.path === file.path && current.sha256 === file.sha256)) };
    report.nonEmptyConfigurationPreservationVerified = report.before.hasSavedConfiguration === true && report.migration?.preserved === true;
    if (report.migration && !report.migration.preserved) throw new Error('已有配置、规则完成状态或历史保持检查失败；保留文件，不自动回退');
    if (!report.resources.preserved) throw new Error('已有组件资源文件发生变化；保留副本，需复核后继续');
    report.migrationBackups = [];
    if (report.before.exists && report.before.version < 4) {
      for (const name of readdirSync(join(dataRoot, 'backups')).filter(name => name.startsWith(`before-v${report.before.version}-`) && name.endsWith('.sqlite'))) {
        const path = join(dataRoot, 'backups', name);
        const summary = configurationSummary(path);
        if (summary.version === report.before.version && contentDigest(path) === report.before.contentSha256) report.migrationBackups.push({ path, sha256: fileHash(path), version: summary.version, integrity: summary.integrity, contentMatchesBefore: true });
      }
      if (!report.migrationBackups.length) throw new Error('未找到内容与升级前匹配且校验通过的迁移前自动备份');
    }
    report.migrationChecksComplete = true;
    report.manualPending = ['普通应用覆盖及返回关系', '侧栏卡片输入与文字保存恢复', '宽度比例和遮罩手动调节及正常重启恢复'];
    report.stage = 'normal-data-verified-manual-observation-pending'; report.finished = Date.now(); save();
    console.log(`程序身份、v3→v4、迁移前备份及原配置保持核对完成。未自动反复启动软件，三组人工观察和正常重启恢复另行确认。\n报告：${reportPath}`);
  } catch (error) { report.failure = error.message; report.stage = 'stopped-with-evidence'; save(); throw error; }
  finally { input.close(); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
