import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, delimiter, resolve } from 'node:path';
import { homedir } from 'node:os';

// 只影响子进程，不写入系统 PATH 或 Rust 默认工具链。
const rustBin = join(homedir(), '.cargo', 'bin');
const env = { ...process.env, PATH: `${rustBin}${delimiter}${process.env.PATH}` };
const [action, ...args] = process.argv.slice(2);
// 本地签名密钥只在构建子进程使用；CI 从仓库 Secrets 注入。
if (action === 'build' && !env.TAURI_SIGNING_PRIVATE_KEY) {
  const key = resolve('artifacts/release-keys/updater.key');
  const password = resolve('artifacts/release-keys/updater.password');
  if (existsSync(key) && existsSync(password)) {
    env.TAURI_SIGNING_PRIVATE_KEY = key;
    env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = readFileSync(password, 'utf8').trim();
  }
}
// 开发入口默认使用项目隔离数据，防止一次本地启动就迁移正常用户库。
if (['dev', 'runtime'].includes(action) && !args.some(arg => arg.startsWith('--desktop-') || arg.startsWith('--g0-'))) {
  const normal = args.indexOf('--normal-data');
  if (normal >= 0) args.splice(normal, 1);
  else {
    const root = resolve('artifacts/desktop-integration-development');
    const marker = join(root, 'isolated-test-data.marker');
    if (!existsSync(root)) { mkdirSync(root, { recursive: true }); writeFileSync(marker, 'SoulDesk isolated development data', { flag: 'wx' }); }
    if (!existsSync(marker)) throw new Error('开发目录缺少隔离标识，未打开正常用户库');
    if (action === 'dev') args.push('--');
    args.push('--desktop-sandbox', root);
  }
}
const cargo = existsSync(join(rustBin, 'cargo.exe')) ? join(rustBin, 'cargo.exe') : 'cargo';
let command, commandArgs;
if (['test', 'fmt', 'network', 'market', 'recovery', 'desktop', 'check:rust', 'runtime'].includes(action)) {
  command = cargo;
  commandArgs = action === 'runtime' ? ['run', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', 'souldesk', '--', ...args]
    : ['network', 'market', 'recovery', 'desktop'].includes(action) ? ['run', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', action === 'desktop' ? 'desktop-probe' : action === 'market' ? 'market-probe' : action === 'recovery' ? 'recovery-probe' : 'network-probe', ...(action === 'desktop' ? ['--features', 'tauri/custom-protocol'] : []), '--', ...args]
    : [action === 'check:rust' ? 'check' : action, '--manifest-path', 'src-tauri/Cargo.toml', ...args];
} else {
  command = process.execPath;
  commandArgs = ['node_modules/@tauri-apps/cli/tauri.js', action, ...args];
}
const result = spawnSync(command, commandArgs, { env, stdio: 'inherit' });
if (result.error) console.error(result.error.message);
process.exit(result.status ?? 1);
