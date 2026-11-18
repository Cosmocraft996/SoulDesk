// 构建记录只保存源码/产物指纹，不读取用户配置或运行数据库。
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, writeFileSync, renameSync, mkdirSync, openSync, closeSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';

export const statePath = 'components/widgetjs/build-state.json';
const hash = value => createHash('sha256').update(value).digest('hex');

export function withBuildLock(work, root = '.') {
  mkdirSync(join(root, 'artifacts'), { recursive: true });
  const path = join(root, 'artifacts/widgetjs-build.lock');
  let fd;
  try { fd = openSync(path, 'wx'); }
  catch (error) {
    if (error.code === 'EEXIST') throw new Error(`已有组件构建或上次异常退出留下的锁：${path}。请先确认该构建已结束，再单独移除此锁文件。`);
    throw error;
  }
  try { writeFileSync(fd, JSON.stringify({ pid: process.pid })); return work(); }
  finally { closeSync(fd); unlinkSync(path); }
}

function tree(root, relative) {
  return readdirSync(join(root, relative), { withFileTypes: true }).flatMap(item => {
    const path = `${relative}/${item.name}`;
    if (item.isSymbolicLink()) throw new Error(`组件构建输入不接受符号链接：${path}`);
    if (item.isDirectory()) return tree(root, path);
    return item.isFile() && !/\.test\.[cm]?[jt]sx?$/.test(item.name)
      ? [[path, hash(readFileSync(join(root, path)))]] : [];
  }).sort(([a], [b]) => a.localeCompare(b, 'en'));
}

export function inputGroups(root = '.') {
  const groups = [];
  for (const item of readdirSync(join(root, 'components/widgetjs'), { withFileTypes: true })) {
    if (item.isDirectory() && !item.name.endsWith('-bundle')) {
      const path = `components/widgetjs/${item.name}`;
      groups.push([path, hash(JSON.stringify(tree(root, path)))]);
    }
  }
  const scripts = readdirSync(join(root, 'scripts')).filter(name =>
    /^(?:adapt-widgetjs-.*|build-widgetjs(?:-entries)?|build-excalidraw|widgetjs-hotspots|widgetjs-entry|widgetjs-build-state)\.mjs$/.test(name));
  for (const path of [...scripts.map(name => `scripts/${name}`), 'sdk/host.js', 'sdk/host.d.ts', 'package.json', 'pnpm-lock.yaml', 'pnpm-workspace.yaml', '.gitattributes']) {
    if (existsSync(join(root, path))) groups.push([path, hash(readFileSync(join(root, path)))]);
  }
  return groups.sort(([a], [b]) => a.localeCompare(b, 'en'));
}

export function packageState(files) {
  const manifest = JSON.parse(new TextDecoder().decode(files['manifest.json']));
  if (!files[manifest.entry]) throw new Error(`组件入口缺失：${manifest.id}/${manifest.entry}`);
  const entries = Object.keys(files).sort().map(path => [path, files[path].length, hash(files[path])]);
  return { id: manifest.id, version: manifest.version, digest: hash(JSON.stringify(entries)), files: entries.length };
}

function version(value) {
  if (!/^\d+\.\d+\.\d+$/.test(value)) throw new Error(`组件版本须为三段数字：${value}`);
  return value.split('.').map(BigInt);
}

export function assertPackageVersions(previous, next) {
  const old = new Map(previous.map(item => [item.id, item]));
  const ids = new Set();
  for (const item of next) {
    if (ids.has(item.id)) throw new Error(`重复组件标识：${item.id}`);
    ids.add(item.id);
    const current = version(item.version), before = old.get(item.id);
    if (!before) continue;
    const prior = version(before.version);
    const index = current.findIndex((value, i) => value !== prior[i]);
    if (index >= 0 && current[index] < prior[index]) throw new Error(`拒绝组件降级：${item.id} ${before.version} → ${item.version}`);
    if (index < 0 && item.digest !== before.digest) throw new Error(`组件内容已变化但版本未提升：${item.id} ${item.version}。请提升 manifest.json 中的版本后重新构建；旧包和构建记录未覆盖。`);
  }
  for (const id of old.keys()) if (!ids.has(id)) throw new Error(`已登记组件从构建列表消失：${id}，请先确认移除范围。`);
}

export function readBuildState(root = '.') {
  const state = JSON.parse(readFileSync(join(root, statePath), 'utf8'));
  if (state.schemaVersion !== 1 || !Array.isArray(state.inputs) || !Array.isArray(state.packages)) throw new Error('组件构建记录格式无效，请恢复版本库中的记录');
  return state;
}

export function cacheMatches(state, inputs, packages) {
  const sort = items => [...items].sort((a, b) => a.id.localeCompare(b.id, 'en'));
  return JSON.stringify(state.inputs) === JSON.stringify(inputs)
    && JSON.stringify(sort(state.packages)) === JSON.stringify(sort(packages));
}

export function recordBuildState(inputs, packages, root = '.') {
  const target = join(root, statePath), pending = `${target}.pending`;
  writeFileSync(pending, JSON.stringify({ schemaVersion: 1, inputs, packages: [...packages].sort((a, b) => a.id.localeCompare(b.id, 'en')) }, null, 2) + '\n');
  renameSync(pending, target);
}
