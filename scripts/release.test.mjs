import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { verifyRelease, verifySignedArtifact, releaseRepository } from './release.mjs';
const config = { version: '0.5.0', identifier: 'local.souldesk.desktop', bundle: { createUpdaterArtifacts: true, targets: ['nsis'] }, plugins: { updater: { pubkey: 'test-public-key', requireSignedVersion: true, endpoints: [`https://github.com/${releaseRepository}/releases/latest/download/latest.json`] } } };
test('打包拒绝版本错配、错误标签、其他仓库更新源和未签名构建', () => {
  assert.equal(verifyRelease('0.5.0', '0.5.0', config, 'refs/tags/v0.5.0'), 'v0.5.0');
  assert.throws(() => verifyRelease('0.5.1', '0.5.0', config));
  assert.throws(() => verifyRelease('0.5.0', '0.5.0', config, 'refs/tags/v0.4.0'));
  const wrongSource = structuredClone(config); wrongSource.plugins.updater.endpoints[0] = 'https://github.com/Cosmocraft996/yyds-SoulDesk-releases/releases/latest/download/latest.json';
  assert.throws(() => verifyRelease('0.5.0', '0.5.0', wrongSource));
  const unsigned = structuredClone(config); unsigned.plugins.updater.pubkey = '';
  assert.throws(() => verifyRelease('0.5.0', '0.5.0', unsigned));
});
test('官方命令行生成的真实签名可验证，篡改安装内容、签名和版本均被拒绝', () => {
  const root = new URL('../src-tauri/tests/fixtures/', import.meta.url);
  const payload = readFileSync(new URL('update-payload.txt', root));
  const pub = readFileSync(new URL('updater.pub', root), 'utf8');
  const signature = readFileSync(new URL('update-payload.txt.sig', root), 'utf8');
  assert.match(verifySignedArtifact(payload, pub, signature, '0.5.0'), /^[a-f0-9]{64}$/);
  assert.throws(() => verifySignedArtifact(Buffer.concat([payload, Buffer.from('tampered')]), pub, signature, '0.5.0'));
  assert.throws(() => verifySignedArtifact(payload, pub, signature, '0.6.0'));
  const changed = Buffer.from(Buffer.from(signature.trim(), 'base64').toString().replace('version:0.5.0', 'version:0.6.0')).toString('base64');
  assert.throws(() => verifySignedArtifact(payload, pub, changed, '0.6.0'));
});
