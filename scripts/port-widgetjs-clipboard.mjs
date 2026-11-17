// 只读取固定Git对象，不执行上游脚本；原字节存在时仅核对，不覆盖。
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { createHash } from 'node:crypto';

const repository = 'artifacts/widgetjs-upstream/clipboard';
const revision = '2482fc6b238983f3fe8f2aecf042d92a9b29c9e2';
const target = 'components/widgetjs/clipboard-source';
const listing = spawnSync('git', ['-C', repository, 'ls-tree', '-r', '--name-only', revision, 'src', 'public', 'package.json', 'pnpm-lock.yaml', 'README.md', 'vite.config.ts', 'widget.package.ts', 'index.html', 'LICENSE'], { encoding: 'utf8' });
if (listing.status !== 0) throw Error('无法枚举固定剪切板来源');
const files = [];
for (const source of listing.stdout.trim().split('\n')) {
  const result = spawnSync('git', ['-C', repository, 'show', `${revision}:${source}`], { maxBuffer: 8 * 1024 * 1024 });
  if (result.status !== 0) throw Error('读取固定来源失败：' + source);
  const path = 'original/' + source;
  const out = join(target, path);
  mkdirSync(dirname(out), { recursive: true });
  if (existsSync(out)) {
    if (!readFileSync(out).equals(result.stdout)) throw Error('来源字节不同，未覆盖：' + out);
  } else writeFileSync(out, result.stdout, { flag: 'wx' });
  files.push({ source, path, bytes: result.stdout.length, sha256: createHash('sha256').update(result.stdout).digest('hex') });
}
const audit = { repository: 'https://github.com/rtugeek/clipboard', revision, license: '该固定树未提供许可证，package.json未声明；保留事实，不补造授权。用户已明确自用并授权移植。', files };
writeFileSync(join(target, 'SOURCE_FILES.json'), JSON.stringify(audit, null, 2) + '\n');
console.log(JSON.stringify({ revision, files: files.length, bytes: files.reduce((sum, file) => sum + file.bytes, 0) }));
