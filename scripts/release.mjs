import { readFileSync, readdirSync } from 'node:fs';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const releaseRepository = 'Cosmocraft996/SoulDesk';
export function verifyRelease(packageVersion, cargoVersion, config, ref = '') {
  if (!/^\d+\.\d+\.\d+$/.test(packageVersion) || packageVersion !== cargoVersion || packageVersion !== config.version) throw new Error('主程序三处版本必须一致，且为正式语义版本');
  if (ref.startsWith('refs/tags/') && ref !== `refs/tags/v${packageVersion}`) throw new Error('版本标签与源码版本不一致，已阻止错误发布');
  const expected = `https://github.com/${releaseRepository}/releases/latest/download/latest.json`;
  if (JSON.stringify(config.plugins?.updater?.endpoints) !== JSON.stringify([expected])) throw new Error('更新源必须指向当前公开仓库的发行页面');
  if (!config.bundle?.createUpdaterArtifacts || !config.plugins?.updater?.pubkey || !config.plugins.updater.requireSignedVersion) throw new Error('更新签名及版本绑定配置不完整');
  if (JSON.stringify(config.bundle.targets) !== '["nsis"]' || config.identifier !== 'local.souldesk.desktop') throw new Error('安装器类型或应用标识发生变化，请先处理升级兼容');
  return `v${packageVersion}`;
}
// 仅用于交付检查；应用内安装仍使用官方更新插件验证。
export function verifySignedArtifact(payload, encodedPublicKey, encodedSignature, version) {
  const publicLines = Buffer.from(encodedPublicKey.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
  const lines = Buffer.from(encodedSignature.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
  if (publicLines.length !== 2 || lines.length !== 4 || !lines[2].startsWith('trusted comment: ')) throw new Error('更新签名格式无效');
  const pub = Buffer.from(publicLines[1], 'base64'), sig = Buffer.from(lines[1], 'base64');
  if (pub.length !== 42 || sig.length !== 74 || sig.subarray(0, 2).toString() !== 'ED' || !pub.subarray(2, 10).equals(sig.subarray(2, 10))) throw new Error('签名算法或密钥不匹配');
  const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), pub.subarray(10)]), type: 'spki', format: 'der' });
  const trusted = lines[2].slice('trusted comment: '.length);
  if (!verify(null, createHash('blake2b512').update(payload).digest(), key, sig.subarray(10)) || !verify(null, Buffer.concat([sig.subarray(10), Buffer.from(trusted)]), key, Buffer.from(lines[3], 'base64'))) throw new Error('安装包或签名内容被修改');
  if (trusted.split('\t').find(part => part.startsWith('version:')) !== `version:${version}`) throw new Error('安装包签名版本与声明版本不一致');
  return createHash('sha256').update(payload).digest('hex');
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const root = new URL('../', import.meta.url);
  const pkg = JSON.parse(readFileSync(new URL('package.json', root), 'utf8'));
  const cargo = readFileSync(new URL('src-tauri/Cargo.toml', root), 'utf8').match(/^version = "([^"]+)"/m)?.[1];
  const config = JSON.parse(readFileSync(new URL('src-tauri/tauri.conf.json', root), 'utf8'));
  console.log(`发行配置通过：${verifyRelease(pkg.version, cargo, config, process.env.GITHUB_REF)} → ${releaseRepository}`);
  if (process.argv.includes('--verify-installer')) {
    const dir = resolve('src-tauri/target/release/bundle/nsis');
    const files = readdirSync(dir).filter(name => name.endsWith(`_${pkg.version}_x64-setup.exe`));
    if (files.length !== 1) throw new Error('未找到唯一的本版本Windows安装包');
    const file = join(dir, files[0]);
    const digest = verifySignedArtifact(readFileSync(file), config.plugins.updater.pubkey, readFileSync(`${file}.sig`, 'utf8'), pkg.version);
    console.log(`安装包与签名通过：${file}\nSHA-256：${digest}`);
  }
}
