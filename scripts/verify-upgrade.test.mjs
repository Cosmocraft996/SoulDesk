import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { copyFileSync, mkdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { configurationSummary, snapshotStoppedData, compareSavedData, bundleMarkerEquivalent, canResumeUpgrade } from './verify-upgrade.mjs';

// 全新检出没有被Git忽略的产物目录，测试自行准备夹具父目录。
mkdirSync(resolve('artifacts'), { recursive: true });

test('安装器映像只接受Tauri安装类型标记差异，其他字节篡改仍拒绝', () => {
  const build = Buffer.from('before:__TAURI_BUNDLE_TYPE_VAR_UNK:after');
  const bundled = Buffer.from('before:__TAURI_BUNDLE_TYPE_VAR_NSS:after');
  assert.equal(bundleMarkerEquivalent(build, bundled), true);
  assert.equal(bundleMarkerEquivalent(build, Buffer.from('before:__TAURI_BUNDLE_TYPE_VAR_NSS:changed')), false);
  const previous = { version: '0.4.0', before: { exists: true, version: 3, database: 'original-v3.sqlite' }, beforeExecutableSha256: 'old' };
  assert.equal(canResumeUpgrade(previous, 'bundled', 'bundled'), true);
  assert.equal(canResumeUpgrade(previous, 'old', 'bundled'), false);
  assert.equal(canResumeUpgrade({ ...previous, before: { ...previous.before, version: 4 } }, 'bundled', 'bundled'), false);
  assert.equal(canResumeUpgrade({ ...previous, programChecksComplete: true }, 'bundled', 'bundled'), false);
});

test('升级摘要只读核对配置，运行状态变化不会冒充配置丢失', () => {
  const root = resolve(`artifacts/upgrade-fixture-${randomUUID()}`);
  mkdirSync(root);
  const path = resolve(root, 'workbench.sqlite');
  const db = new DatabaseSync(path);
  db.exec(`PRAGMA user_version=3;
    CREATE TABLE app_settings(key TEXT,value TEXT);
    INSERT INTO app_settings VALUES('shortcut','Ctrl+Alt+Space');
    CREATE TABLE widget_packages(id TEXT,manifest TEXT,directory TEXT,digest TEXT);
    CREATE TABLE window_layouts(label TEXT,geometry TEXT);
    CREATE TABLE watchlist(instance_id TEXT,symbol TEXT,position INTEGER);
    CREATE TABLE widget_instances(id TEXT,package_id TEXT,settings TEXT,storage TEXT,floating INTEGER,position INTEGER,width INTEGER,height INTEGER);
    INSERT INTO widget_instances VALUES('one','market','{"title":"私有标题"}','{}',0,0,760,720);
    CREATE TABLE alert_rules(id TEXT,revision INTEGER,definition TEXT,state TEXT,enabled INTEGER,market_key TEXT);
    INSERT INTO alert_rules VALUES('rule',1,'{"threshold":"100"}','{}',1,'BTC');`);
  const bytes = readFileSync(path);
  const before = configurationSummary(path);
  assert.deepEqual(readFileSync(path), bytes);
  assert.ok(!JSON.stringify(before).includes('私有标题'));
  db.exec(`UPDATE alert_rules SET state='{"sequence":1}';`);
  assert.deepEqual(configurationSummary(path), before);
  db.exec(`UPDATE widget_instances SET storage='{"note":"用户笔记"}';`);
  assert.notEqual(configurationSummary(path).configurationSha256, before.configurationSha256);
  db.exec(`UPDATE widget_instances SET settings='{"title":"更改标题"}';`);
  assert.notEqual(configurationSummary(path).configurationSha256, before.configurationSha256);
  db.close();
});

test('不存在或损坏的数据不能伪装已有配置验收通过', () => {
  assert.equal(configurationSummary(resolve(`artifacts/missing-${randomUUID()}.sqlite`)).exists, false);
  assert.throws(() => configurationSummary(import.meta.filename));
});

test('已有空库不代表验证了非空配置保持', () => {
  const root = resolve(`artifacts/upgrade-empty-${randomUUID()}`);
  mkdirSync(root);
  const path = resolve(root, 'workbench.sqlite');
  const db = new DatabaseSync(path);
  db.exec(`PRAGMA user_version=3;
    CREATE TABLE app_settings(key TEXT,value TEXT);
    CREATE TABLE widget_packages(id TEXT,manifest TEXT,directory TEXT,digest TEXT);
    CREATE TABLE widget_instances(id TEXT,package_id TEXT,settings TEXT,storage TEXT,floating INTEGER,position INTEGER,width INTEGER,height INTEGER);
    CREATE TABLE window_layouts(label TEXT,geometry TEXT);
    CREATE TABLE watchlist(instance_id TEXT,symbol TEXT,position INTEGER);
    CREATE TABLE alert_rules(id TEXT,revision INTEGER,definition TEXT,state TEXT,enabled INTEGER,market_key TEXT);`);
  db.close();
  const result = configurationSummary(path);
  assert.equal(result.exists, true);
  assert.equal(result.configurationRows, 0);
  assert.equal(result.hasSavedConfiguration, false);
});

test('一致备份包含WAL中的配置且不改写原始主文件', () => {
  const source = resolve(`artifacts/upgrade-wal-${randomUUID()}`);
  mkdirSync(source);
  const path = resolve(source, 'workbench.sqlite');
  const db = new DatabaseSync(path);
  try {
    db.exec(`PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=3;
      CREATE TABLE app_settings(key TEXT,value TEXT);
      CREATE TABLE widget_packages(id TEXT,manifest TEXT,directory TEXT,digest TEXT);
      CREATE TABLE widget_instances(id TEXT,package_id TEXT,settings TEXT,storage TEXT,floating INTEGER,position INTEGER,width INTEGER,height INTEGER);
      CREATE TABLE window_layouts(label TEXT,geometry TEXT);
      CREATE TABLE watchlist(instance_id TEXT,symbol TEXT,position INTEGER);
      CREATE TABLE alert_rules(id TEXT,revision INTEGER,definition TEXT,state TEXT,enabled INTEGER,market_key TEXT);
      INSERT INTO app_settings VALUES('shortcut','WAL中尚未归并的配置');`);
    const original = readFileSync(path), wal = readFileSync(`${path}-wal`);
    // 此处故意保留夹具连接证明WAL覆盖；真实入口另强制全部SoulDesk进程退出。
    const backup = snapshotStoppedData(source, resolve(`artifacts/upgrade-backup-${randomUUID()}`));
    assert.equal(backup.integrity, 'ok');
    assert.equal(backup.version, 3);
    assert.equal(backup.tables.app_settings.rows, 1);
    assert.ok(backup.files.some(file => file.path.endsWith('-wal')));
    assert.deepEqual(readFileSync(path), original);
    assert.deepEqual(readFileSync(`${path}-wal`), wal);
    const restored = new DatabaseSync(backup.database, { readOnly: true });
    try { assert.equal(restored.prepare('SELECT value FROM app_settings').get().value, 'WAL中尚未归并的配置'); } finally { restored.close(); }
    assert.ok(!JSON.stringify(backup).includes('WAL中尚未归并的配置'));
  } finally { db.close(); }
});

test('v3到v4比较允许新增默认值，仍检出配置、历史和已完成规则丢失', () => {
  const root = resolve(`artifacts/upgrade-compare-${randomUUID()}`); mkdirSync(root);
  const previous = resolve(root, 'before.sqlite'), current = resolve(root, 'after.sqlite');
  const db = new DatabaseSync(previous);
  db.exec(`PRAGMA user_version=3;
    CREATE TABLE app_settings(key TEXT,value TEXT);
    INSERT INTO app_settings VALUES('shortcut','Ctrl+Alt+Space');
    CREATE TABLE widget_packages(id TEXT,manifest TEXT,directory TEXT,digest TEXT);
    CREATE TABLE widget_instances(id TEXT,package_id TEXT,settings TEXT,storage TEXT,floating INTEGER,position INTEGER,width INTEGER,height INTEGER);
    INSERT INTO widget_instances VALUES('one','card','{}','{"text":"应保留"}',0,0,300,200);
    CREATE TABLE window_layouts(label TEXT,geometry TEXT);
    CREATE TABLE watchlist(instance_id TEXT,symbol TEXT,position INTEGER);
    CREATE TABLE alert_rules(id TEXT,revision INTEGER,definition TEXT,state TEXT,enabled INTEGER,market_key TEXT);
    INSERT INTO alert_rules VALUES('rule',1,'{}','{"completed":true}',1,'BTC');
    CREATE TABLE alert_events(id TEXT,rule_id TEXT,revision INTEGER,sequence INTEGER,event TEXT,created INTEGER);
    INSERT INTO alert_events VALUES('event','rule',1,1,'{}',10);`);
  db.close(); copyFileSync(previous, current);
  const after = new DatabaseSync(current);
  try {
    after.exec(`ALTER TABLE widget_instances ADD COLUMN location TEXT DEFAULT 'desktop';
      ALTER TABLE widget_instances ADD COLUMN visible INTEGER DEFAULT 1;
      ALTER TABLE widget_instances ADD COLUMN pinned INTEGER DEFAULT 0;
      INSERT INTO app_settings VALUES('hidden_all','false'); PRAGMA user_version=4;`);
    assert.equal(compareSavedData(previous, current).preserved, true);
    assert.equal(configurationSummary(current).version, 4);
    after.exec(`UPDATE alert_rules SET state='{"completed":false}'`);
    assert.equal(compareSavedData(previous, current).completedRulesPreserved, false);
    after.exec(`UPDATE alert_rules SET state='{"completed":true}'; DELETE FROM alert_events;`);
    assert.equal(compareSavedData(previous, current).preserved, false);
    after.exec(`INSERT INTO alert_events VALUES('event','rule',1,1,'{}',10);
      UPDATE widget_instances SET storage='{}';`);
    const comparison = compareSavedData(previous, current);
    assert.equal(comparison.tables.widget_instances.retained, false);
    assert.ok(!JSON.stringify(comparison).includes('应保留'));
  } finally { after.close(); }
});

test('迁移新增默认设置不能将空库升级冒充非空数据升级', () => {
  const root = resolve(`artifacts/upgrade-empty-transition-${randomUUID()}`); mkdirSync(root);
  const before = resolve(root, 'before.sqlite'), after = resolve(root, 'after.sqlite');
  const db = new DatabaseSync(before);
  db.exec(`PRAGMA user_version=3;
    CREATE TABLE app_settings(key TEXT,value TEXT);
    CREATE TABLE widget_packages(id TEXT,manifest TEXT,directory TEXT,digest TEXT);
    CREATE TABLE widget_instances(id TEXT,package_id TEXT,settings TEXT,storage TEXT,floating INTEGER,position INTEGER,width INTEGER,height INTEGER);
    CREATE TABLE window_layouts(label TEXT,geometry TEXT);
    CREATE TABLE watchlist(instance_id TEXT,symbol TEXT,position INTEGER);
    CREATE TABLE alert_rules(id TEXT,revision INTEGER,definition TEXT,state TEXT,enabled INTEGER,market_key TEXT);`);
  db.close(); copyFileSync(before, after);
  const changed = new DatabaseSync(after);
  changed.exec(`INSERT INTO app_settings VALUES('hidden_all','false'); PRAGMA user_version=4;`); changed.close();
  assert.equal(compareSavedData(before, after).preserved, true);
  assert.equal(configurationSummary(before).hasSavedConfiguration, false);
});
