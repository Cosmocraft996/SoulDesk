import { readFileSync, mkdirSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { zipSync } from 'fflate';
import { createHash } from 'node:crypto';

// 此脚本独立于 Vite/宿主构建；产物不被打入安装器。
mkdirSync('artifacts', { recursive: true });
for (const source of process.argv.length > 2 ? process.argv.slice(2) : ['examples/clock', 'examples/template']) {
  const manifest = JSON.parse(readFileSync(join(source, 'manifest.json'), 'utf8'));
  const files = { 'manifest.json': new Uint8Array(readFileSync(join(source, 'manifest.json'))), 'dist/host.js': new Uint8Array(readFileSync('sdk/host.js')) };
  for (const file of readdirSync(source, { withFileTypes: true })) {
    if (file.isFile() && /\.(html|css|js)$/.test(file.name) && !file.name.endsWith('.test.js') && file.name !== 'host.js') files[`dist/${file.name}`] = new Uint8Array(readFileSync(join(source, file.name)));
  }
  const bytes = zipSync(files, { level: 9, mtime: new Date('2026-11-12T00:00:00Z') });
  const target = `artifacts/${manifest.id}-${manifest.version}.zip`;
  writeFileSync(target, bytes);
  console.log(`${target}  SHA256=${createHash('sha256').update(bytes).digest('hex')}`);
}
