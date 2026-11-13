import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { zipSync, unzipSync } from 'fflate';

// 发布包只收录显式清单，不扫描用户数据或诊断目录。
process.chdir(fileURLToPath(new URL('../', import.meta.url)));
const git = (...args) => {
  const result = spawnSync('git', args, { encoding: 'utf8' });
  if (result.status !== 0) throw new Error('Git 状态读取失败，请先完成源码提交。');
  return result.stdout.trim();
};
const sourceCommit = git('rev-parse', 'HEAD');
if (git('status', '--porcelain')) throw new Error('源码工作区尚未提交，停止打包。');
const version = '0.3.3-desktop-preview.1';
const name = `SoulDesk_${version}_windows-x64`;
const directory = 'artifacts/releases';
mkdirSync(directory, { recursive: true });
const targets = [`${directory}/${name}.zip`, `${directory}/SHA256SUMS.txt`, `${directory}/desktop-preview-manifest.json`];
if (targets.some(path => existsSync(path))) throw new Error('发布产物已存在；请核对后使用新的版本，禁止覆盖既有发布。');
const bytes = text => Buffer.from(text, 'utf8');
const files = {
  'desktop-probe.exe': readFileSync('src-tauri/target/release/desktop-probe.exe'),
  '启动桌面验证.cmd': bytes('@echo off\r\nchcp 65001 >nul\r\ncd /d "%~dp0"\r\n"%~dp0desktop-probe.exe" --widgets\r\nif errorlevel 1 (\r\n  echo 验证程序未正常完成，请查看 artifacts 中的报告。\r\n  pause\r\n)\r\n'),
  'README.md': readFileSync('docs/10-桌面验证预览版发布说明.md'),
  'artifacts/local.example.clock-0.1.1.zip': readFileSync('artifacts/local.example.clock-0.1.1.zip'),
  'artifacts/local.example.card-0.1.0.zip': readFileSync('artifacts/local.example.card-0.1.0.zip'),
  'third-party/window-vibrancy-0.8.1-MIT.txt': readFileSync('docs/licenses/window-vibrancy-0.8.1-MIT.txt'),
};
const sha256 = data => createHash('sha256').update(data).digest('hex');
const manifest = {
  version, sourceCommit, kind: 'desktop-validation-preview', platform: 'windows-x64',
  normalApplicationInstallerIncluded: false, normalDatabaseIncluded: false,
  files: Object.entries(files).map(([path, data]) => ({ path, bytes: data.length, sha256: sha256(data) })),
};
files['bundle-manifest.json'] = bytes(JSON.stringify(manifest, null, 2) + '\n');
const archive = zipSync(Object.fromEntries(Object.entries(files).map(([path, data]) => [`SoulDesk-desktop-preview/${path}`, data])), { level: 6 });
const unpacked = unzipSync(archive);
if (Object.keys(unpacked).length !== Object.keys(files).length) throw new Error('发布包文件数量不符。');
for (const [path, data] of Object.entries(files)) {
  if (sha256(unpacked[`SoulDesk-desktop-preview/${path}`]) !== sha256(data)) throw new Error(`发布包完整性检查失败：${path}`);
}
const archiveManifest = { ...manifest, archive: { name: `${name}.zip`, bytes: archive.length, sha256: sha256(archive) } };
writeFileSync(targets[0], archive, { flag: 'wx' });
writeFileSync(targets[1], `${sha256(archive)}  ${name}.zip\n`, { flag: 'wx' });
writeFileSync(targets[2], JSON.stringify(archiveManifest, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ sourceCommit, paths: targets, archiveBytes: archive.length, archiveSha256: sha256(archive), verifiedFiles: Object.keys(files).length }, null, 2));
