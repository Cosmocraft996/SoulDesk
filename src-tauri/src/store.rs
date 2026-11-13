use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub type AppResult<T> = Result<T, String>;
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct Store {
    pub root: PathBuf,
    pub db: Mutex<Connection>,
    log_lock: Mutex<()>,
}

impl Store {
    pub fn backup(&self) -> AppResult<PathBuf> {
        let dir = self.root.join("backups");
        std::fs::create_dir_all(&dir).map_err(err)?;
        let path = dir.join(format!(
            "manual-v4-{}-{}.sqlite",
            now_ms(),
            uuid::Uuid::new_v4()
        ));
        let db = self.db.lock().map_err(err)?;
        db.execute("VACUUM INTO ?1", [path.to_str().ok_or("备份路径编码无效")?])
            .map_err(err)?;
        let backup = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(err)?;
        let check: String = backup
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(err)?;
        let version: u32 = backup
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(err)?;
        if check != "ok" || version != 4 {
            return Err("备份校验失败，原数据仍保留".into());
        }
        Ok(path)
    }
    pub fn log(&self, category: &str, message: &str) -> AppResult<()> {
        use std::io::Write;
        let _lock = self.log_lock.lock().map_err(err)?;
        let dir = self.root.join("logs");
        std::fs::create_dir_all(&dir).map_err(err)?;
        let path = dir.join("current.jsonl");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() >= 10 * 1024 * 1024) {
            let oldest = dir.join("previous-4.jsonl");
            if oldest.is_file() {
                std::fs::remove_file(&oldest).map_err(err)?;
            }
            for index in (1..=3).rev() {
                let from = dir.join(format!("previous-{index}.jsonl"));
                if from.is_file() {
                    std::fs::rename(from, dir.join(format!("previous-{}.jsonl", index + 1)))
                        .map_err(err)?;
                }
            }
            std::fs::rename(&path, dir.join("previous-1.jsonl")).map_err(err)?;
        }
        let line = json!({"time":now_ms(),"category":category,"message":message.chars().take(2000).collect::<String>()});
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(err)?;
        writeln!(file, "{line}").map_err(err)
    }
    pub fn open(root: PathBuf) -> AppResult<Self> {
        std::fs::create_dir_all(root.join("widgets")).map_err(err)?;
        let mut db = Connection::open(root.join("workbench.sqlite")).map_err(err)?;
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(err)?;
        if version > 4 {
            return Err("数据版本高于当前宿主；已停止写入并保留原文件".into());
        }
        let check: String = db
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(err)?;
        if check != "ok" {
            return Err("数据库完整性检查失败；保留原文件，请勿重置数据".into());
        }
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
        )
        .map_err(err)?;
        if version < 4 {
            if version > 0 {
                let backup_dir = root.join("backups");
                std::fs::create_dir_all(&backup_dir).map_err(err)?;
                let backup_path = backup_dir.join(format!(
                    "before-v{version}-{}-{}.sqlite",
                    now_ms(),
                    uuid::Uuid::new_v4()
                ));
                db.execute(
                    "VACUUM INTO ?1",
                    [backup_path.to_str().ok_or("备份路径编码无效")?],
                )
                .map_err(err)?;
                let backup = Connection::open_with_flags(
                    &backup_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .map_err(err)?;
                let valid: String = backup
                    .query_row("PRAGMA quick_check", [], |r| r.get(0))
                    .map_err(err)?;
                let saved_version: u32 = backup
                    .query_row("PRAGMA user_version", [], |r| r.get(0))
                    .map_err(err)?;
                if valid != "ok" || saved_version != version {
                    return Err("迁移前备份校验失败；未修改原数据版本".into());
                }
            }
            // 结构与版本号在同一事务中提交；失败由 Transaction 回滚，不重置用户数据。
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(err)?;
            if version == 0 {
                tx.execute_batch("
          CREATE TABLE IF NOT EXISTS app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS widget_packages(id TEXT PRIMARY KEY,manifest TEXT NOT NULL,directory TEXT NOT NULL,digest TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS widget_instances(id TEXT PRIMARY KEY,package_id TEXT NOT NULL REFERENCES widget_packages(id),settings TEXT NOT NULL,storage TEXT NOT NULL DEFAULT '{}',floating INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE IF NOT EXISTS test_events(id TEXT PRIMARY KEY,created INTEGER NOT NULL,completed INTEGER,notification TEXT NOT NULL,sound TEXT NOT NULL,view_count INTEGER);
                ").map_err(err)?;
            }
            if version < 2 {
                tx.execute_batch("ALTER TABLE widget_instances ADD COLUMN position INTEGER NOT NULL DEFAULT 0 CHECK(position>=0);
              ALTER TABLE widget_instances ADD COLUMN width INTEGER CHECK(width BETWEEN 120 AND 1600);
              ALTER TABLE widget_instances ADD COLUMN height INTEGER CHECK(height BETWEEN 80 AND 1200);
              CREATE TABLE window_layouts(label TEXT PRIMARY KEY,geometry TEXT NOT NULL);
              CREATE TABLE widget_versions(package_id TEXT NOT NULL,digest TEXT NOT NULL,manifest TEXT NOT NULL,directory TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(package_id,digest));").map_err(err)?;
                tx.execute_batch("CREATE TABLE widget_upgrade_snapshots(package_id TEXT NOT NULL,from_digest TEXT NOT NULL,to_digest TEXT NOT NULL,instance_id TEXT NOT NULL,settings TEXT NOT NULL,storage TEXT NOT NULL,created INTEGER NOT NULL);").map_err(err)?;
                tx.execute("INSERT INTO widget_versions SELECT id,digest,manifest,directory,?1 FROM widget_packages", [now_ms()]).map_err(err)?;
            }
            if version < 3 {
                tx.execute_batch("CREATE TABLE watchlist(instance_id TEXT NOT NULL REFERENCES widget_instances(id) ON DELETE CASCADE,symbol TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(instance_id,symbol));
              CREATE TABLE alert_rules(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,definition TEXT NOT NULL,state TEXT NOT NULL,enabled INTEGER NOT NULL,market_key TEXT NOT NULL);
              CREATE INDEX alert_rules_market ON alert_rules(market_key,enabled);
              CREATE TABLE alert_events(id TEXT PRIMARY KEY,rule_id TEXT NOT NULL,revision INTEGER NOT NULL,sequence INTEGER NOT NULL,event TEXT NOT NULL,created INTEGER NOT NULL,notification TEXT NOT NULL,sound TEXT NOT NULL,notification_attempts INTEGER NOT NULL DEFAULT 0,sound_attempts INTEGER NOT NULL DEFAULT 0,notification_error TEXT,sound_error TEXT,UNIQUE(rule_id,revision,sequence));
              CREATE INDEX alert_events_time ON alert_events(created);").map_err(err)?;
            }
            tx.execute_batch("ALTER TABLE widget_instances ADD COLUMN location TEXT NOT NULL DEFAULT 'desktop' CHECK(location IN ('desktop','panel'));
              ALTER TABLE widget_instances ADD COLUMN visible INTEGER NOT NULL DEFAULT 1 CHECK(visible IN (0,1));
              ALTER TABLE widget_instances ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1));
              UPDATE widget_instances SET pinned=COALESCE((SELECT json_extract(geometry,'$.pinned') FROM window_layouts WHERE label='float-'||widget_instances.id),0);
              INSERT INTO app_settings(key,value) VALUES('hidden_all','false') ON CONFLICT(key) DO NOTHING;").map_err(err)?;
            tx.pragma_update(None, "user_version", 4).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        // 中断时不重播已经尝试过的系统递送。
        db.execute(
            "UPDATE test_events SET notification='unknown' WHERE notification='sending'",
            [],
        )
        .map_err(err)?;
        db.execute_batch("UPDATE alert_events SET notification='unknown',notification_error='上次在递送中退出，系统结果不确定' WHERE notification='sending';
          UPDATE alert_events SET sound='unknown',sound_error='上次在递送中退出，系统结果不确定' WHERE sound='sending';").map_err(err)?;
        // 待发送事件使用交易所校正UTC判断时效；启动时尚未校时，不能按本地墙钟提前过期。
        db.execute(
            "UPDATE test_events SET sound='unknown' WHERE sound='sending'",
            [],
        )
        .map_err(err)?;
        Ok(Self {
            root,
            db: Mutex::new(db),
            log_lock: Mutex::new(()),
        })
    }

    pub fn snapshot(&self) -> AppResult<Value> {
        let db = self.db.lock().map_err(err)?;
        let mut packages = db
            .prepare("SELECT manifest FROM widget_packages ORDER BY id")
            .map_err(err)?;
        let packages = packages
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|r| serde_json::from_str::<Value>(&r.map_err(err)?).map_err(err))
            .collect::<AppResult<Vec<_>>>()?;
        let mut instances = db
            .prepare("SELECT id,package_id,settings,floating,width,height,location,visible,pinned FROM widget_instances ORDER BY position,rowid")
            .map_err(err)?;
        let instances = instances.query_map([], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, bool>(3)?,r.get::<_,Option<u32>>(4)?,r.get::<_,Option<u32>>(5)?,r.get::<_,String>(6)?,r.get::<_,bool>(7)?,r.get::<_,bool>(8)?))).map_err(err)?
            .map(|r| { let (id,package_id,settings,floating,width,height,location,visible,pinned)=r.map_err(err)?; Ok(json!({"id":id,"packageId":package_id,"settings":serde_json::from_str::<Value>(&settings).map_err(err)?,"floating":floating,"width":width,"height":height,"location":location,"visible":visible,"pinned":pinned})) }).collect::<AppResult<Vec<_>>>()?;
        let mut events = db.prepare("SELECT id,created,completed,notification,sound,view_count FROM test_events ORDER BY created DESC LIMIT 30").map_err(err)?;
        let events = events.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"created":r.get::<_,u64>(1)?,"completed":r.get::<_,Option<u64>>(2)?,"notification":r.get::<_,String>(3)?,"sound":r.get::<_,String>(4)?,"viewCount":r.get::<_,Option<u64>>(5)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        Ok(json!({"packages":packages,"instances":instances,"events":events}))
    }

    pub fn preference(&self, key: &str) -> Option<String> {
        self.db
            .lock()
            .ok()?
            .query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .ok()
    }

    pub fn presentation(
        &self,
        id: &str,
        location: &str,
        visible: bool,
        pinned: bool,
    ) -> AppResult<()> {
        if !["desktop", "panel"].contains(&location) {
            return Err("INVALID_ARGUMENT: 未知展示位置".into());
        }
        if self
            .db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE widget_instances SET location=?1,visible=?2,pinned=?3 WHERE id=?4",
                params![location, visible, pinned, id],
            )
            .map_err(err)?
            != 1
        {
            return Err("实例不存在".into());
        }
        Ok(())
    }

    pub fn save_preference(&self, key: &str, value: &str) -> AppResult<()> {
        self.db.lock().map_err(err)?.execute("INSERT INTO app_settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(err)?;
        Ok(())
    }

    pub fn panel_layouts(&self) -> AppResult<Value> {
        let db = self.db.lock().map_err(err)?;
        let mut statement = db.prepare("SELECT i.id,s.value FROM widget_instances i JOIN app_settings s ON s.key='panel_layout:'||i.id").map_err(err)?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(err)?;
        let mut layouts = serde_json::Map::new();
        for row in rows {
            let (id, raw) = row.map_err(err)?;
            let layout: crate::panel::Layout = serde_json::from_str(&raw).map_err(err)?;
            layout.validate(1.0, 1.0)?;
            layouts.insert(id, serde_json::to_value(layout).map_err(err)?);
        }
        Ok(Value::Object(layouts))
    }

    pub fn save_panel_layout(&self, id: &str, layout: crate::panel::Layout) -> AppResult<()> {
        let db = self.db.lock().map_err(err)?;
        let (location, manifest): (String, String) = db.query_row("SELECT i.location,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
        if location != "panel" {
            return Err("CONFLICT: 组件已经移出面板，请刷新后重试".into());
        }
        let manifest: Value = serde_json::from_str(&manifest).map_err(err)?;
        layout.validate(
            manifest["minSize"]["width"]
                .as_f64()
                .ok_or("组件最小尺寸无效")?,
            manifest["minSize"]["height"]
                .as_f64()
                .ok_or("组件最小尺寸无效")?,
        )?;
        db.execute("INSERT INTO app_settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("panel_layout:{id}"), serde_json::to_string(&layout).map_err(err)?]).map_err(err)?;
        Ok(())
    }

    pub fn save_window_layout(&self, label: &str, geometry: &Value) -> AppResult<()> {
        if label != "main"
            && !label
                .strip_prefix("float-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            return Err("INVALID_ARGUMENT: 不保存临时容器布局".into());
        }
        if !["x", "y"].iter().all(|k| {
            geometry[*k]
                .as_f64()
                .is_some_and(|v| v.is_finite() && v.abs() <= 10_000_000.)
        }) || !["width", "height"].iter().all(|k| {
            geometry[*k]
                .as_f64()
                .is_some_and(|v| (80.0..=16_384.0).contains(&v))
        }) || !geometry["scale"]
            .as_f64()
            .is_some_and(|v| (0.5..=8.0).contains(&v))
            || !geometry["pinned"].is_boolean()
        {
            return Err("INVALID_ARGUMENT: 窗口布局参数无效".into());
        }
        self.db.lock().map_err(err)?.execute("INSERT INTO window_layouts(label,geometry) VALUES(?1,?2) ON CONFLICT(label) DO UPDATE SET geometry=excluded.geometry",params![label,geometry.to_string()]).map_err(err)?;
        Ok(())
    }

    pub fn window_layout(&self, label: &str) -> AppResult<Option<Value>> {
        let data: Option<String> = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT geometry FROM window_layouts WHERE label=?1",
                [label],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        data.map(|s| serde_json::from_str(&s).map_err(err))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panel_layout_survives_restart_without_changing_desktop_geometry() {
        let root = std::env::temp_dir().join(format!("souldesk-panel-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let bytes = crate::presets::find("local.example.clock")
            .unwrap()
            .unwrap()
            .archive()
            .unwrap();
        let package = crate::widgets::stage_bytes(&store, bytes).unwrap();
        crate::widgets::activate(&store, &package).unwrap();
        let id = crate::widgets::create_instance(&store, "local.example.clock").unwrap();
        crate::widgets::set_instance_size(&store, &id, 600, 400).unwrap();
        let desktop = json!({"x":10,"y":20,"width":600,"height":400,"scale":1.5,"pinned":false});
        store
            .save_window_layout(&format!("float-{id}"), &desktop)
            .unwrap();
        let layout = crate::panel::Layout {
            x: 30.0,
            y: 60.0,
            width: 360.0,
            height: 220.0,
        };
        assert!(store.save_panel_layout(&id, layout).is_err());
        store.presentation(&id, "panel", true, false).unwrap();
        store.save_panel_layout(&id, layout).unwrap();
        let bad = crate::panel::Layout {
            width: 1.0,
            ..layout
        };
        assert!(store.save_panel_layout(&id, bad).is_err());
        drop(store);
        let store = Store::open(root).unwrap();
        assert_eq!(
            store.panel_layouts().unwrap()[&id],
            serde_json::to_value(layout).unwrap()
        );
        store.presentation(&id, "desktop", true, false).unwrap();
        assert_eq!(
            store
                .window_layout(&format!("float-{id}"))
                .unwrap()
                .unwrap(),
            desktop
        );
        assert_eq!(store.snapshot().unwrap()["instances"][0]["width"], 600);
    }
    #[test]
    fn corrupt_database_is_refused_and_the_original_bytes_are_preserved() {
        let root = std::env::temp_dir().join(format!("souldesk-corrupt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("workbench.sqlite");
        let original = b"SQLite format 3\0deliberately damaged database body";
        std::fs::write(&path, original).unwrap();
        assert!(Store::open(root.clone()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!root.join("backups").exists());
    }
    #[test]
    fn a_pre_upgrade_v2_backup_can_be_restored_offline_without_losing_original_settings() {
        let root = legacy_v2_database();
        let store = Store::open(root.clone()).unwrap();
        store.save_preference("shortcut", "Ctrl+Alt+Z").unwrap();
        drop(store);
        let paths = std::fs::read_dir(root.join("backups"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 1);
        let backup_before_restore = std::fs::read(&paths[0]).unwrap();
        let restore =
            std::env::temp_dir().join(format!("souldesk-offline-restore-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&restore).unwrap();
        std::fs::copy(&paths[0], restore.join("workbench.sqlite")).unwrap();
        let db = Connection::open_with_flags(
            restore.join("workbench.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert_eq!(
            db.query_row(
                "SELECT value FROM app_settings WHERE key='shortcut'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "Ctrl+Alt+Q"
        );
        drop(db);
        let restored = Store::open(restore).unwrap();
        assert_eq!(
            restored.preference("shortcut").as_deref(),
            Some("Ctrl+Alt+Q")
        );
        assert_eq!(std::fs::read(&paths[0]).unwrap(), backup_before_restore);
        assert_eq!(
            restored.snapshot().unwrap()["instances"][0]["settings"]["title"],
            "保留的配置"
        );
        let upgraded = Store::open(root).unwrap();
        assert_eq!(
            upgraded.preference("shortcut").as_deref(),
            Some("Ctrl+Alt+Z")
        );
    }
    #[test]
    fn manual_backup_is_readable_and_contains_saved_preferences() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-backup-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        store.save_preference("shortcut", "Ctrl+Alt+Q").unwrap();
        let path = store.backup().unwrap();
        let db =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT value FROM app_settings WHERE key='shortcut'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "Ctrl+Alt+Q"
        );
    }

    fn legacy_database() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("souldesk-migration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        // 固定 G0 发布结构，模拟用户已经保存的数据，不调用新版本建库代码。
        db.execute_batch(r#"CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
          CREATE TABLE widget_packages(id TEXT PRIMARY KEY,manifest TEXT NOT NULL,directory TEXT NOT NULL,digest TEXT NOT NULL);
          CREATE TABLE widget_instances(id TEXT PRIMARY KEY,package_id TEXT NOT NULL REFERENCES widget_packages(id),settings TEXT NOT NULL,storage TEXT NOT NULL DEFAULT '{}',floating INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE test_events(id TEXT PRIMARY KEY,created INTEGER NOT NULL,completed INTEGER,notification TEXT NOT NULL,sound TEXT NOT NULL,view_count INTEGER);
          INSERT INTO app_settings VALUES('shortcut','Ctrl+Alt+Q');
          INSERT INTO widget_packages VALUES('local.test.clock','{}','old-directory','old-digest');
          INSERT INTO widget_instances(id,package_id,settings) VALUES('old-instance','local.test.clock','{"title":"保留的配置"}');
          PRAGMA user_version=1;"#).unwrap();
        root
    }

    fn populated_v3() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "souldesk-desktop-migration-{}",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(root.clone()).unwrap();
        drop(store);
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        db.execute_batch(r#"
          ALTER TABLE widget_instances DROP COLUMN location;
          ALTER TABLE widget_instances DROP COLUMN visible;
          ALTER TABLE widget_instances DROP COLUMN pinned;
          INSERT INTO widget_packages VALUES('local.test.clock','{}','saved-package','digest');
          INSERT INTO widget_versions VALUES('local.test.clock','digest','{}','saved-package',1);
          INSERT INTO widget_instances(id,package_id,settings,storage,floating,position,width,height) VALUES('old','local.test.clock','{"title":"非空配置"}','{"text":"独立存储"}',1,2,430,490);
          INSERT INTO window_layouts VALUES('float-old','{"x":80,"y":90,"width":430,"height":490,"scale":1.5,"pinned":true}');
          INSERT INTO watchlist VALUES('old','BTCUSDT',0);
          INSERT INTO alert_rules VALUES('rule',2,'{"saved":"definition"}','{"completed":true}',1,'saved-market');
          INSERT INTO alert_events(id,rule_id,revision,sequence,event,created,notification,sound) VALUES('event','rule',2,1,'{"saved":"history"}',1,'sent','suppressed');
          PRAGMA user_version=3;
        "#).unwrap();
        root
    }
    #[test]
    fn desktop_migration_preserves_nonempty_data_backup_and_offline_rollback() {
        let root = populated_v3();
        let store = Store::open(root.clone()).unwrap();
        let i = &store.snapshot().unwrap()["instances"][0];
        assert_eq!(i["location"], "desktop");
        assert_eq!(i["visible"], true);
        assert_eq!(i["pinned"], true);
        assert_eq!(i["settings"]["title"], "非空配置");
        assert_eq!(i["width"], 430);
        assert_eq!(store.window_layout("float-old").unwrap().unwrap()["x"], 80);
        let backup = std::fs::read_dir(root.join("backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let saved =
            Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            saved
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            3
        );
        let db = store.db.lock().unwrap();
        for (table, columns) in [
            ("widget_packages", "*"),
            ("widget_versions", "*"),
            ("watchlist", "*"),
            ("alert_rules", "*"),
            ("alert_events", "*"),
            ("window_layouts", "*"),
            (
                "widget_instances",
                "id,package_id,settings,storage,floating,position,width,height",
            ),
        ] {
            let sql = format!("SELECT * FROM (SELECT {columns} FROM {table})");
            let rows = |db: &Connection| {
                let mut q = db.prepare(&sql).unwrap();
                let n = q.column_count();
                q.query_map([], |r| {
                    (0..n)
                        .map(|c| r.get::<_, rusqlite::types::Value>(c))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
            };
            assert_eq!(rows(&db), rows(&saved), "{table}");
        }
        drop(db);
        drop(saved);
        drop(store);
        let recovery = root.join("offline-copy");
        std::fs::create_dir(&recovery).unwrap();
        std::fs::copy(&backup, recovery.join("workbench.sqlite")).unwrap();
        let copy = Store::open(recovery).unwrap();
        assert_eq!(
            copy.snapshot().unwrap()["instances"][0]["settings"]["title"],
            "非空配置"
        );
        assert_eq!(
            copy.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT notification FROM alert_events WHERE id='event'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "sent"
        );
    }
    #[test]
    fn desktop_migration_failure_keeps_v3_data_and_rolls_back_added_columns() {
        let root = populated_v3();
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        db.execute_batch("ALTER TABLE widget_instances ADD COLUMN visible INTEGER;")
            .unwrap();
        drop(db);
        assert!(Store::open(root.clone()).is_err());
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            3
        );
        assert!(db.prepare("SELECT location FROM widget_instances").is_err());
        assert_eq!(
            db.query_row(
                "SELECT storage FROM widget_instances WHERE id='old'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            r#"{"text":"独立存储"}"#
        );
        assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(), 1);
    }
    #[test]
    fn display_preferences_are_saved_and_invalid_locations_do_not_change_instances() {
        let root = populated_v3();
        let store = Store::open(root.clone()).unwrap();
        assert!(store.presentation("old", "other", false, false).is_err());
        store.presentation("old", "panel", false, true).unwrap();
        store.save_preference("hidden_all", "true").unwrap();
        drop(store);
        let store = Store::open(root).unwrap();
        let data = store.snapshot().unwrap();
        assert_eq!(data["instances"][0]["location"], "panel");
        assert_eq!(data["instances"][0]["visible"], false);
        assert_eq!(store.preference("hidden_all").as_deref(), Some("true"));
    }

    #[test]
    fn fresh_database_has_current_layout_and_version_tables() {
        let root = std::env::temp_dir().join(format!("souldesk-fresh-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let db = store.db.lock().unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            4
        );
        assert!(db.prepare("SELECT geometry FROM window_layouts").is_ok());
        assert!(db.prepare("SELECT digest FROM widget_versions").is_ok());
        assert!(db
            .prepare("SELECT position,width,height FROM widget_instances")
            .is_ok());
    }

    #[test]
    fn migration_preserves_data_and_validates_a_pre_migration_snapshot() {
        let root = legacy_database();
        let store = Store::open(root.clone()).unwrap();
        assert_eq!(store.preference("shortcut").as_deref(), Some("Ctrl+Alt+Q"));
        let db = store.db.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT settings FROM widget_instances WHERE id='old-instance'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "{\"title\":\"保留的配置\"}"
        );
        assert_eq!(
            db.query_row(
                "SELECT digest FROM widget_versions WHERE package_id='local.test.clock'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "old-digest"
        );
        let backups = std::fs::read_dir(root.join("backups"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        let backup =
            Connection::open_with_flags(&backups[0], rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            backup
                .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert_eq!(
            backup
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            backup
                .query_row(
                    "SELECT settings FROM widget_instances WHERE id='old-instance'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "{\"title\":\"保留的配置\"}"
        );
    }

    #[test]
    fn migration_failure_rolls_back_schema_and_keeps_saved_data() {
        let root = legacy_database();
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE window_layouts(conflicting_column TEXT);")
            .unwrap();
        drop(db);
        assert!(Store::open(root.clone()).is_err());
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert!(db.prepare("SELECT position FROM widget_instances").is_err());
        assert_eq!(
            db.query_row(
                "SELECT value FROM app_settings WHERE key='shortcut'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "Ctrl+Alt+Q"
        );
    }

    #[test]
    fn newer_database_is_refused_without_overwriting_it() {
        let root = legacy_database();
        let path = root.join("workbench.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=99;").unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(Store::open(root).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn window_layout_and_floating_instance_survive_restart() {
        let root = legacy_database();
        let store = Store::open(root.clone()).unwrap();
        let geometry =
            json!({"x":120.0,"y":80.0,"width":820.0,"height":540.0,"scale":1.5,"pinned":true});
        store.save_window_layout("main", &geometry).unwrap();
        store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_instances SET floating=1 WHERE id='old-instance'",
                [],
            )
            .unwrap();
        drop(store);
        let store = Store::open(root).unwrap();
        assert_eq!(store.window_layout("main").unwrap(), Some(geometry));
        assert_eq!(store.snapshot().unwrap()["instances"][0]["floating"], true);
        assert!(store
            .save_window_layout(
                "main",
                &json!({"x":0.0,"y":0.0,"width":-1.0,"height":100.0,"scale":1.0,"pinned":false})
            )
            .is_err());
    }

    fn legacy_v2_database() -> PathBuf {
        let root = legacy_database();
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        db.execute_batch("ALTER TABLE widget_instances ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
          ALTER TABLE widget_instances ADD COLUMN width INTEGER;ALTER TABLE widget_instances ADD COLUMN height INTEGER;
          CREATE TABLE window_layouts(label TEXT PRIMARY KEY,geometry TEXT NOT NULL);
          CREATE TABLE widget_versions(package_id TEXT NOT NULL,digest TEXT NOT NULL,manifest TEXT NOT NULL,directory TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(package_id,digest));
          CREATE TABLE widget_upgrade_snapshots(package_id TEXT NOT NULL,from_digest TEXT NOT NULL,to_digest TEXT NOT NULL,instance_id TEXT NOT NULL,settings TEXT NOT NULL,storage TEXT NOT NULL,created INTEGER NOT NULL);
          PRAGMA user_version=2;").unwrap();
        root
    }
    #[test]
    fn g1_database_migrates_to_g2_with_a_valid_v2_backup() {
        let root = legacy_v2_database();
        let store = Store::open(root.clone()).unwrap();
        assert_eq!(
            store.snapshot().unwrap()["instances"][0]["settings"]["title"],
            "保留的配置"
        );
        let db = store.db.lock().unwrap();
        assert!(db.prepare("SELECT id FROM alert_rules").is_ok());
        assert!(db.prepare("SELECT id FROM alert_events").is_ok());
        let backups = std::fs::read_dir(root.join("backups"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        let backup =
            Connection::open_with_flags(&backups[0], rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            backup
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            backup
                .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }
    #[test]
    fn g2_migration_conflict_keeps_g1_version_and_rolls_back_added_tables() {
        let root = legacy_v2_database();
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE alert_rules(conflicting_column TEXT);")
            .unwrap();
        drop(db);
        assert!(Store::open(root.clone()).is_err());
        let db = Connection::open(root.join("workbench.sqlite")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert!(db.prepare("SELECT * FROM watchlist").is_err());
        assert_eq!(
            db.query_row(
                "SELECT value FROM app_settings WHERE key='shortcut'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "Ctrl+Alt+Q"
        );
    }
}
