import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { configurationSummary } from './verify-upgrade.mjs';

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
