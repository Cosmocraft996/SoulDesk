import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createUpdateManifest, verifyRelease, verifyReleaseDraft, verifySignedArtifact, releaseRepository } from './release.mjs';
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

const installerName = 'SoulDesk_0.5.0_x64-setup.exe';
const installerUrl = `https://github.com/${releaseRepository}/releases/download/v0.5.0/${installerName}`;
const installerApiUrl = `https://api.github.com/repos/${releaseRepository}/releases/assets/15`;
const signature = readFileSync(new URL('../src-tauri/tests/fixtures/update-payload.txt.sig', import.meta.url));
const platform = { url: installerUrl, signature: signature.toString('utf8') };
const validManifest = {
  version: '0.5.0',
  notes: '测试发行说明',
  pub_date: '2026-11-19T00:00:00.000Z',
  platforms: { 'windows-x86_64': platform, 'windows-x86_64-nsis': platform },
};

function draftFixture(manifest = validManifest) {
  const manifestBytes = Buffer.from(JSON.stringify(manifest));
  const hash = bytes => createHash('sha256').update(bytes).digest('hex');
  const installerDigest = hash(readFileSync(new URL('../src-tauri/tests/fixtures/update-payload.txt', import.meta.url)));
  const assets = [
    { id: 15, name: installerName, digest: `sha256:${installerDigest}`, browser_download_url: installerUrl },
    { id: 16, name: `${installerName}.sig`, digest: `sha256:${hash(signature)}` },
    { id: 17, name: 'latest.json', digest: `sha256:${hash(manifestBytes)}` },
  ].map(asset => ({ ...asset, state: 'uploaded' }));
  return {
    draft: { tag_name: 'v0.5.0', draft: true, prerelease: false, assets },
    build: { version: '0.5.0', installerName, installerDigest, signature, manifestBytes },
  };
}

test('正式版草稿必须包含与本次构建一致的三项附件', () => {
  const { draft, build } = draftFixture();
  verifyReleaseDraft(draft, build);
  for (const fields of [{ tag_name: 'v0.5.1' }, { draft: false }, { prerelease: true }]) {
    assert.throws(() => verifyReleaseDraft({ ...draft, ...fields }, build), /本版本的正式版草稿/);
  }
  for (const asset of draft.assets) {
    const missing = draft.assets.filter(item => item !== asset);
    assert.throws(() => verifyReleaseDraft({ ...draft, assets: missing }, build), /草稿附件缺失/);
    const changed = [...missing, { ...asset, digest: 'sha256:changed' }];
    assert.throws(() => verifyReleaseDraft({ ...draft, assets: changed }, build), /草稿附件缺失/);
  }
  const pending = draft.assets.map(asset => ({ ...asset, state: 'starter' }));
  assert.throws(() => verifyReleaseDraft({ ...draft, assets: pending }, build), /未完成上传/);
});

test('更新清单须描述本版本并将两个 Windows x64 更新项绑定到同一个安装包和签名', () => {
  for (const fields of [{ version: '0.5.1' }, { notes: 10 }, { pub_date: '2026-11-19' }]) {
    const { draft, build } = draftFixture({ ...validManifest, ...fields });
    assert.throws(() => verifyReleaseDraft(draft, build), /更新清单的版本、说明或日期无效/);
  }
  for (const key of Object.keys(validManifest.platforms)) {
    for (const entry of [undefined, { ...platform, url: installerApiUrl }, { ...platform, url: installerUrl + '0' }, { ...platform, signature: 'wrong' }]) {
      const platforms = { ...validManifest.platforms, [key]: entry };
      const { draft, build } = draftFixture({ ...validManifest, platforms });
      assert.throws(() => verifyReleaseDraft(draft, build), /更新清单的下载地址或签名不匹配/);
    }
  }
});

test('生成的更新清单使用公开下载地址并保留发行说明和签名', () => {
  const manifest = createUpdateManifest({ version: '0.5.0', installerName, signature,
    notes: '测试发行说明', createdAt: '2026-11-19T00:00:00Z' });
  assert.deepEqual(manifest, validManifest);
  const { draft, build } = draftFixture(manifest);
  verifyReleaseDraft(draft, build);
  draft.assets[0].browser_download_url = installerUrl.replace('/v0.5.0/', '/untagged-a1b2c3/');
  verifyReleaseDraft(draft, build);
  draft.assets[0].browser_download_url = installerApiUrl;
  assert.throws(() => verifyReleaseDraft(draft, build), /公开下载地址不匹配/);
});
