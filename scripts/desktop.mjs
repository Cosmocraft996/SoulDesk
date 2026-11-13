import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join, delimiter } from 'node:path';
import { homedir } from 'node:os';

// 只影响子进程，不写入系统 PATH 或 Rust 默认工具链。
const rustBin = join(homedir(), '.cargo', 'bin');
const env = { ...process.env, PATH: `${rustBin}${delimiter}${process.env.PATH}` };
const [action, ...args] = process.argv.slice(2);
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
