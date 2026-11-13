import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { zipSync, unzipSync } from 'fflate';
const version = JSON.parse(readFileSync('examples/clock/manifest.json', 'utf8')).version;
const source = unzipSync(readFileSync(`artifacts/local.example.clock-${version}.zip`));
mkdirSync('artifacts/g1-fixtures', { recursive: true });
function build(name, change) {
  const files = { ...source };
  const manifest = JSON.parse(new TextDecoder().decode(files['manifest.json']));
  manifest.version = name === 'upgrade' ? '0.2.0' : '0.3.0';
  manifest.settingsSchema.properties.upgradeNote = { type: 'string', default: '兼容升级' };
  change(manifest, files);
  files['manifest.json'] = new TextEncoder().encode(JSON.stringify(manifest));
  writeFileSync(`artifacts/g1-fixtures/${name}.zip`, zipSync(files, { level: 9, mtime: new Date('2026-11-12T00:00:00Z') }));
}
build('upgrade', manifest => { manifest.settingsSchema.properties.upgradeNote = { type: 'string', default: '兼容升级' }; });
build('runtime-failure', (_, files) => { files['dist/clock.js'] = new TextEncoder().encode("throw new Error('G1 受控脚本失败');"); });
build('incompatible', manifest => { manifest.settingsSchema.properties.title = { type: 'integer', default: 1 }; });
build('unknown-capability', manifest => { manifest.capabilities.push('shell.exec'); });
build('bad-path', (_, files) => { files['../escape.txt'] = new TextEncoder().encode('拒绝越界'); });
console.log('artifacts/g1-fixtures/：兼容升级、脚本失败、配置不兼容、未知能力及坏路径包已独立构建');
