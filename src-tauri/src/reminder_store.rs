//! 三款专用后台的实例配置合同；本模块不启动采样、窗口或音频。
use crate::{
    alert_notice,
    alerts::Direction,
    reminder_state::{PhonePreset, PhoneRule, SitConfig},
    store::{err, AppResult, Store},
    widgets,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Source {
    Island,
    Sit,
    Phone,
}
impl Source {
    pub fn package(self) -> &'static str {
        match self {
            Self::Island => "local.souldesk.widgetjs.dynamic-island",
            Self::Sit => "local.souldesk.widgetjs.sit-reminder",
            Self::Phone => "local.souldesk.widgetjs.phone-reminder",
        }
    }
    pub fn capability(self) -> &'static str {
        match self {
            Self::Island => "notifications.island",
            Self::Sit => "reminders.sit",
            Self::Phone => "reminders.phone",
        }
    }
    pub fn get(self) -> &'static str {
        match self {
            Self::Island => "island.get",
            Self::Sit => "sit.get",
            Self::Phone => "phone.get",
        }
    }
    fn configure(self) -> &'static str {
        match self {
            Self::Island => "island.configure",
            Self::Sit => "sit.configure",
            Self::Phone => "phone.configure",
        }
    }
    pub fn default_config(self) -> Value {
        match self {
            Self::Island => json!({"enabled":false}),
            Self::Sit => serde_json::to_value(SitConfig::default()).expect("固定默认配置可序列化"),
            Self::Phone => {
                json!({"enabled":false,"rules":[PhoneRule::new(PhonePreset::Squidward),PhoneRule::new(PhonePreset::Tea),PhoneRule::new(PhonePreset::Pill),PhoneRule::new(PhonePreset::Study)]})
            }
        }
    }
    fn validate(self, value: &Value) -> AppResult<()> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Island {
            enabled: bool,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Phone {
            enabled: bool,
            rules: Vec<PhoneRule>,
        }
        match self {
            Self::Island => {
                let config: Island = serde_json::from_value(value.clone())
                    .map_err(|e| format!("INVALID_ARGUMENT: {e}"))?;
                let _ = config.enabled;
            }
            Self::Sit => serde_json::from_value::<SitConfig>(value.clone())
                .map_err(|e| format!("INVALID_ARGUMENT: {e}"))?
                .validate()?,
            Self::Phone => {
                let config: Phone = serde_json::from_value(value.clone())
                    .map_err(|e| format!("INVALID_ARGUMENT: {e}"))?;
                let _ = config.enabled;
                if config.rules.len() != 4 {
                    return Err("INVALID_ARGUMENT: 来电须保留四种固定预置".into());
                }
                let mut seen = std::collections::HashSet::new();
                for rule in &config.rules {
                    rule.validate()?;
                    if !seen.insert(rule.preset) {
                        return Err("INVALID_ARGUMENT: 来电预置不能重复".into());
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn for_package(package: &str) -> Option<Self> {
        [Self::Island, Self::Sit, Self::Phone]
            .into_iter()
            .find(|source| source.package() == package)
    }
    fn for_method(method: &str) -> Option<Self> {
        [Self::Island, Self::Sit, Self::Phone]
            .into_iter()
            .find(|source| source.get() == method || source.configure() == method)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    version: u32,
    pub revision: u64,
    source: Source,
    pub config: Value,
    receipts: Vec<Receipt>,
}
pub(crate) struct InstanceConfig {
    pub id: String,
    pub source: Source,
    pub revision: u64,
    pub config: Value,
}
pub(crate) struct ConfigurationScan {
    pub valid: Vec<InstanceConfig>,
    pub failures: std::collections::HashMap<String, String>,
}
const LEGACY_NOTIFICATION_CHANNEL: &str = "widget_notification_channel";
pub(crate) const WIDGET_CALLS: &str = "widget_notification_calls";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Channel {
    Windows,
    Island,
}
impl Channel {
    pub(crate) fn preference_key(self) -> &'static str {
        match self {
            Self::Windows => "notification.windows.enabled",
            Self::Island => "notification.island.enabled",
        }
    }
    pub(crate) fn enabled(self, db: &Connection) -> AppResult<bool> {
        Ok(preference(db, self.preference_key())?.as_deref() != Some("false"))
    }
    pub(crate) fn enabled_in(self, store: &Store) -> AppResult<bool> {
        self.enabled(&*store.db.lock().map_err(err)?)
    }
    pub(crate) fn legacy_default(db: &Connection) -> AppResult<Self> {
        Ok(
            match preference(db, LEGACY_NOTIFICATION_CHANNEL)?.as_deref() {
                Some("island") => Self::Island,
                _ => Self::Windows,
            },
        )
    }
}

pub(crate) const WINDOWS_DISABLED: &str = "NOTIFICATION_DISABLED: Windows 系统通知已关闭";

pub(crate) const ISLAND_MAX_VISIBLE: &str = "notification.island.max_visible";
pub(crate) const ISLAND_OPACITY: &str = "notification.island.opacity";
pub(crate) const ISLAND_VOLUME: &str = "notification.island.volume";
pub(crate) const ISLAND_HIDDEN: &str = "notification.island.hidden";
pub(crate) const ISLAND_MUTED: &str = "notification.island.muted";
pub(crate) const MARKET_SOUND: &str = "notification.island.market_sound";
pub(crate) const ISLAND_VISIBLE_RANGE: std::ops::RangeInclusive<usize> = 1..=20;
pub(crate) const ISLAND_PERCENT_RANGE: std::ops::RangeInclusive<u8> = 0..=100;

pub(crate) fn market_tone_key(direction: Direction) -> &'static str {
    match direction {
        Direction::Up => "notification.island.market_tone_up",
        Direction::Down => "notification.island.market_tone_down",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct IslandPreferences {
    pub max_visible: usize,
    pub opacity: u8,
    pub volume: u8,
    pub hidden: bool,
    pub muted: bool,
    pub market_sound: bool,
    pub market_up_tone: &'static str,
    pub market_down_tone: &'static str,
}

impl Default for IslandPreferences {
    fn default() -> Self {
        Self {
            max_visible: 9,
            opacity: 100,
            volume: 100,
            hidden: false,
            muted: false,
            market_sound: true,
            market_up_tone: alert_notice::default_tone(Direction::Up),
            market_down_tone: alert_notice::default_tone(Direction::Down),
        }
    }
}

impl IslandPreferences {
    pub fn read(db: &Connection) -> AppResult<Self> {
        let tone = |direction| -> AppResult<&'static str> {
            Ok(preference(db, market_tone_key(direction))?
                .as_deref()
                .and_then(alert_notice::tone_id)
                .unwrap_or_else(|| alert_notice::default_tone(direction)))
        };
        Ok(Self {
            max_visible: preference(db, ISLAND_MAX_VISIBLE)?
                .and_then(|value| value.parse().ok())
                .filter(|value| ISLAND_VISIBLE_RANGE.contains(value))
                .unwrap_or(Self::default().max_visible),
            opacity: preference(db, ISLAND_OPACITY)?
                .and_then(|value| value.parse().ok())
                .filter(|value| ISLAND_PERCENT_RANGE.contains(value))
                .unwrap_or(Self::default().opacity),
            volume: preference(db, ISLAND_VOLUME)?
                .and_then(|value| value.parse().ok())
                .filter(|value| ISLAND_PERCENT_RANGE.contains(value))
                .unwrap_or(Self::default().volume),
            hidden: preference(db, ISLAND_HIDDEN)?.as_deref() == Some("true"),
            muted: preference(db, ISLAND_MUTED)?.as_deref() == Some("true"),
            market_sound: preference(db, MARKET_SOUND)?.as_deref() != Some("false"),
            market_up_tone: tone(Direction::Up)?,
            market_down_tone: tone(Direction::Down)?,
        })
    }
}

pub(crate) fn ensure_windows_enabled(store: &Store) -> AppResult<()> {
    if Channel::Windows.enabled_in(store)? {
        Ok(())
    } else {
        Err(WINDOWS_DISABLED.into())
    }
}

pub(crate) fn preference(db: &Connection, key: &str) -> AppResult<Option<String>> {
    db.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .map_err(err)
}

const ISLAND_HISTORY_PREFIX: &str = "notification.island.history/";
const ISLAND_HISTORY_PATTERN: &str = "notification.island.history/*";
const ISLAND_HISTORY_DAYS: &str = "notification.island.history_days";
const DEFAULT_ISLAND_HISTORY_DAYS: u32 = 3;
const ISLAND_HISTORY_LIMIT: u32 = 1000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn island_entry(notice: &crate::reminder_engine::Notice, at: u64) -> Value {
    use crate::reminder_engine::{NoticeKind, HOST_NOTICE_OWNER};
    let source =
        notice
            .source_name
            .as_deref()
            .unwrap_or(match (notice.source.as_str(), notice.kind) {
                (HOST_NOTICE_OWNER, _) => "SoulDesk",
                (_, NoticeKind::Call) => "来电提醒",
                (_, NoticeKind::Reminder) if notice.break_seconds.is_some() => "久坐提醒",
                _ => "灵动通知",
            });
    let mut entry = json!({
        "id": notice.id, "kind": notice.kind, "title": notice.title,
        "message": notice.message, "source": source, "at": at,
    });
    if let Some(alert) = notice.market_alert.as_ref().filter(|alert| alert.replayed) {
        entry["triggeredAt"] = json!(alert.triggered_at);
    }
    entry
}

pub(crate) fn island_pending_entry(notice: &crate::reminder_engine::Notice) -> Value {
    island_entry(notice, notice.created_at)
}

pub(crate) fn remember_island_notice(
    db: &Connection,
    notice: &crate::reminder_engine::Notice,
    shown_at: u64,
) -> AppResult<()> {
    let value = island_entry(notice, shown_at);
    if db
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
            params![
                format!("{ISLAND_HISTORY_PREFIX}{}", notice.id),
                value.to_string()
            ],
        )
        .map_err(err)?
        != 0
    {
        db.execute(
            "DELETE FROM app_settings WHERE rowid IN (
                SELECT rowid FROM app_settings WHERE key GLOB ?1
                ORDER BY rowid DESC LIMIT -1 OFFSET ?2
            )",
            params![ISLAND_HISTORY_PATTERN, ISLAND_HISTORY_LIMIT],
        )
        .map_err(err)?;
    }
    Ok(())
}

pub(crate) fn island_history_days(db: &Connection) -> AppResult<u32> {
    Ok(preference(db, ISLAND_HISTORY_DAYS)?
        .and_then(|value| value.parse().ok())
        .filter(|&days| days > 0)
        .unwrap_or(DEFAULT_ISLAND_HISTORY_DAYS))
}

fn remove_expired_island_history(db: &Connection, now: u64, days: u32) -> AppResult<()> {
    db.execute(
        "DELETE FROM app_settings WHERE key GLOB ?1 AND
           CASE WHEN json_valid(value) THEN coalesce(json_extract(value,'$.at'),json_extract(value,'$.shownAt'),0)
                ELSE 0 END < ?2",
        params![ISLAND_HISTORY_PATTERN, now.saturating_sub(u64::from(days) * DAY_MS)],
    ).map_err(err)?;
    Ok(())
}

pub(crate) fn prune_island_history(store: &Store, now: u64) -> AppResult<()> {
    let db = store.db.lock().map_err(err)?;
    remove_expired_island_history(&db, now, island_history_days(&db)?)
}

pub(crate) fn set_island_history_days(store: &Store, now: u64, days: u32) -> AppResult<()> {
    if days == 0 {
        return Err("INVALID_ARGUMENT: 保留天数必须为正整数".into());
    }
    let mut db = store.db.lock().map_err(err)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    tx.execute(
        "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![ISLAND_HISTORY_DAYS, days.to_string()],
    ).map_err(err)?;
    remove_expired_island_history(&tx, now, days)?;
    tx.commit().map_err(err)
}

pub(crate) fn clear_island_history(store: &Store) -> AppResult<()> {
    store
        .db
        .lock()
        .map_err(err)?
        .execute(
            "DELETE FROM app_settings WHERE key GLOB ?1",
            [ISLAND_HISTORY_PATTERN],
        )
        .map_err(err)?;
    Ok(())
}

pub(crate) fn island_history(store: &Store) -> AppResult<Value> {
    let db = store.db.lock().map_err(err)?;
    let mut query = db
        .prepare("SELECT value FROM app_settings WHERE key GLOB ?1 ORDER BY rowid DESC LIMIT ?2")
        .map_err(err)?;
    let rows = query
        .query_map(
            params![ISLAND_HISTORY_PATTERN, ISLAND_HISTORY_LIMIT],
            |row| row.get::<_, String>(0),
        )
        .map_err(err)?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(serde_json::from_str(&row.map_err(err)?).map_err(err)?);
    }
    Ok(Value::Array(entries))
}

pub(crate) fn migrate_notification_channel(store: &Store) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    if preference(&tx, LEGACY_NOTIFICATION_CHANNEL)?.is_none() {
        let channel = if configurations(&tx)?
            .valid
            .iter()
            .any(|c| c.source == Source::Island && c.config["enabled"] == true)
        {
            "island"
        } else {
            "windows"
        };
        tx.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2)",
            params![LEGACY_NOTIFICATION_CHANNEL, channel],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)
}
pub(crate) fn configurations(db: &Connection) -> AppResult<ConfigurationScan> {
    let mut query=db.prepare("SELECT i.id,p.id,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id ORDER BY i.id").map_err(err)?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(err)?;
    let mut result = Vec::new();
    let mut failures = std::collections::HashMap::new();
    for row in rows {
        let (id, package, raw) = row.map_err(err)?;
        let Some(source) = Source::for_package(&package) else {
            continue;
        };
        let manifest: widgets::Manifest = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => {
                failures.insert(id, "提醒组件清单损坏，已停用此实例".into());
                continue;
            }
        };
        if !manifest
            .capabilities
            .iter()
            .any(|cap| cap == source.capability())
        {
            continue;
        }
        let document = match load(db, &id, source) {
            Ok(value) => value,
            Err(error) if error.starts_with("DATA_UNAVAILABLE:") => {
                failures.insert(id, error);
                continue;
            }
            Err(error) => return Err(error),
        };
        result.push(InstanceConfig {
            id,
            source,
            revision: document.revision,
            config: document.config,
        });
    }
    Ok(ConfigurationScan {
        valid: result,
        failures,
    })
}
fn key(instance: &str) -> String {
    format!("reminder.source/{instance}")
}
pub(crate) fn load(db: &Connection, id: &str, source: Source) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(id)],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(Document {
            version: 1,
            revision: 0,
            source,
            config: source.default_config(),
            receipts: Vec::new(),
        });
    };
    if raw.len() > 65536 {
        return Err("DATA_UNAVAILABLE: 提醒配置超过容量，未覆盖".into());
    }
    let document: Document =
        serde_json::from_str(&raw).map_err(|_| "DATA_UNAVAILABLE: 提醒配置损坏，未覆盖")?;
    if document.version != 1
        || document.source != source
        || document.revision > 9_007_199_254_740_991
        || document.receipts.len() > 64
    {
        return Err("DATA_UNAVAILABLE: 提醒配置类型、版本或回执无效".into());
    }
    source
        .validate(&document.config)
        .map_err(|_| "DATA_UNAVAILABLE: 已保存提醒配置无效，未覆盖")?;
    Ok(document)
}
fn snapshot(document: &Document) -> Value {
    json!({"version":1,"revision":document.revision,"config":document.config})
}
pub(crate) fn authorized(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    authorized_with(
        store,
        sessions,
        window,
        token,
        method,
        args,
        |_, _, value, _| Ok(value),
    )
}
/// 回调仍处于native_actions→Sessions→SQLite边界内；只允许更新内存，不能再次锁数据库。
pub(crate) fn authorized_with<T>(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
    after: impl FnOnce(&str, Source, Value, bool) -> AppResult<T>,
) -> AppResult<T> {
    let source = Source::for_method(method).ok_or("CAPABILITY_DENIED: 未开放的专用提醒方法")?;
    let _native = store.native_actions.lock().map_err(err)?;
    widgets::with_capability(
        store,
        sessions,
        window,
        token,
        source.capability(),
        |session| {
            let mut db = store.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            let package: String = tx
                .query_row(
                    "SELECT package_id FROM widget_instances WHERE id=?1",
                    [&session.instance],
                    |row| row.get(0),
                )
                .map_err(err)?;
            if package != source.package() {
                return Err("CAPABILITY_DENIED: 此接口仅用于固定提醒组件".into());
            }
            let mut document = load(&tx, &session.instance, source)?;
            if method == source.get() {
                if !args.is_null() && args.as_object().is_none_or(|args| !args.is_empty()) {
                    return Err("INVALID_ARGUMENT: 提醒状态读取不接受参数".into());
                }
                return after(&session.instance, source, snapshot(&document), false);
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Change {
                request_id: String,
                expected_revision: u64,
                config: Value,
            }
            let change: Change = serde_json::from_value(args.clone())
                .map_err(|e| format!("INVALID_ARGUMENT: {e}"))?;
            uuid::Uuid::parse_str(&change.request_id)
                .map_err(|_| "INVALID_ARGUMENT: 请求标识须为UUID")?;
            source.validate(&change.config)?;
            let fingerprint = format!("{:x}", Sha256::digest(args.to_string().as_bytes()));
            if let Some(receipt) = document
                .receipts
                .iter()
                .find(|receipt| receipt.id == change.request_id)
            {
                return if receipt.fingerprint == fingerprint {
                    after(&session.instance, source, snapshot(&document), false)
                } else {
                    Err("INVALID_ARGUMENT: 请求标识已用于不同提醒配置".into())
                };
            }
            if change.expected_revision != document.revision {
                return Err("REVISION_CONFLICT: 提醒配置已更新，请重新读取后保存".into());
            }
            document.revision = document
                .revision
                .checked_add(1)
                .filter(|rev| *rev <= 9_007_199_254_740_991)
                .ok_or("LIMIT: 提醒配置版本达到上限")?;
            document.config = change.config;
            document.receipts.push(Receipt {
                id: change.request_id,
                fingerprint,
            });
            if document.receipts.len() > 64 {
                document.receipts.remove(0);
            }
            let raw = serde_json::to_string(&document).map_err(err)?;
            tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key(&session.instance),raw]).map_err(err)?;
            tx.commit().map_err(err)?;
            after(&session.instance, source, snapshot(&document), true)
        },
    )
}
pub(crate) fn enabled_sources(store: &Store) -> AppResult<Vec<(Source, String)>> {
    let db = store.db.lock().map_err(err)?;
    let mut query=db.prepare("SELECT i.id,p.id,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id ORDER BY i.id").map_err(err)?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(err)?;
    let mut result = Vec::new();
    for row in rows {
        let (instance, package, raw) = row.map_err(err)?;
        let Some(source) = Source::for_package(&package) else {
            continue;
        };
        let manifest: widgets::Manifest = serde_json::from_str(&raw).map_err(err)?;
        if !manifest
            .capabilities
            .iter()
            .any(|cap| cap == source.capability())
        {
            continue;
        }
        let document = load(&db, &instance, source)?;
        if document.config["enabled"] == true {
            result.push((source, instance));
        }
    }
    Ok(result)
}
pub(crate) fn remove_data(db: &Connection, id: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(id)])
        .map_err(err)?;
    for prefix in ["reminder.phone-ledger/", "reminder.delivery/"] {
        db.execute(
            "DELETE FROM app_settings WHERE key=?1",
            [format!("{prefix}{id}")],
        )
        .map_err(err)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{store::Store, widgets};
    use serde_json::{json, Value};
    fn fixture(source: Source) -> (Store, widgets::Sessions, String, String) {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-reminder-config-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let manifest = json!({"schemaVersion":1,"id":source.package(),"name":"提醒合同检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":350,"height":180},"minSize":{"width":280,"height":140},"capabilities":[source.capability()],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'reminder-test','reminder-test')",rusqlite::params![source.package(),manifest.to_string()]).unwrap();
        let a = widgets::create_instance(&store, source.package()).unwrap();
        let b = widgets::create_instance(&store, source.package()).unwrap();
        (store, widgets::Sessions::default(), a, b)
    }
    fn open(store: &Store, sessions: &widgets::Sessions, id: &str) -> String {
        widgets::open_display_session(store, sessions, &format!("float-{id}"), id, None).unwrap()
            ["token"]
            .as_str()
            .unwrap()
            .into()
    }
    fn call(
        store: &Store,
        sessions: &widgets::Sessions,
        id: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> Result<Value, String> {
        authorized(store, sessions, &format!("float-{id}"), token, method, args)
    }
    fn change(source: Source, revision: u64, enabled: bool) -> Value {
        let mut config = source.default_config();
        config["enabled"] = json!(enabled);
        json!({"requestId":uuid::Uuid::new_v4().to_string(),"expectedRevision":revision,"config":config})
    }
    #[test]
    fn notification_channel_migrates_enabled_legacy_instance_once_and_keeps_preferences() {
        let (store, sessions, id, other) = fixture(Source::Island);
        let token = open(&store, &sessions, &id);
        call(
            &store,
            &sessions,
            &id,
            &token,
            "island.configure",
            change(Source::Island, 0, true),
        )
        .unwrap();
        migrate_notification_channel(&store).unwrap();
        assert_eq!(
            store.preference(LEGACY_NOTIFICATION_CHANNEL).as_deref(),
            Some("island")
        );
        store
            .save_preference(LEGACY_NOTIFICATION_CHANNEL, "off")
            .unwrap();
        migrate_notification_channel(&store).unwrap();
        assert_eq!(
            store.preference(LEGACY_NOTIFICATION_CHANNEL).as_deref(),
            Some("off")
        );
        let db = store.db.lock().unwrap();
        assert_eq!(
            load(&db, &id, Source::Island).unwrap().config["enabled"],
            true
        );
        assert_eq!(
            load(&db, &other, Source::Island).unwrap().config["enabled"],
            false
        );

        let (fresh, _, _, _) = fixture(Source::Phone);
        migrate_notification_channel(&fresh).unwrap();
        assert_eq!(
            fresh.preference(LEGACY_NOTIFICATION_CHANNEL).as_deref(),
            Some("windows")
        );
        assert!(fresh.preference(WIDGET_CALLS).is_none());
        assert!(Channel::Windows.enabled(&db).unwrap());
        assert!(Channel::Island.enabled(&db).unwrap());
    }
    #[test]
    fn reminder_configs_default_disabled_and_reading_does_not_register_active_sources() {
        for source in [Source::Island, Source::Sit, Source::Phone] {
            let (store, sessions, id, _) = fixture(source);
            let token = open(&store, &sessions, &id);
            let value = call(&store, &sessions, &id, &token, source.get(), Value::Null).unwrap();
            assert_eq!(value["config"]["enabled"], false);
            assert_eq!(value["revision"], 0);
            assert!(enabled_sources(&store).unwrap().is_empty());
            assert_eq!(
                store
                    .db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT count(*) FROM app_settings WHERE key LIKE 'reminder.source/%'",
                        [],
                        |row| row.get::<_, u64>(0)
                    )
                    .unwrap(),
                0
            );
        }
    }
    #[test]
    fn reminder_config_write_is_window_bound_instance_scoped_and_strict() {
        let (store, sessions, a, b) = fixture(Source::Sit);
        let token = open(&store, &sessions, &a);
        let args = change(Source::Sit, 0, true);
        assert!(authorized(
            &store,
            &sessions,
            "other-window",
            &token,
            "sit.configure",
            args.clone()
        )
        .is_err());
        let mut invalid = args.clone();
        invalid["instanceId"] = json!(b);
        assert!(call(&store, &sessions, &a, &token, "sit.configure", invalid).is_err());
        let saved = call(&store, &sessions, &a, &token, "sit.configure", args).unwrap();
        assert_eq!(saved["config"]["enabled"], true);
        assert_eq!(
            enabled_sources(&store).unwrap(),
            vec![(Source::Sit, a.clone())]
        );
        let other = open(&store, &sessions, &b);
        assert_eq!(
            call(&store, &sessions, &b, &other, "sit.get", Value::Null).unwrap()["config"]
                ["enabled"],
            false
        );
        assert!(call(
            &store,
            &sessions,
            &a,
            &token,
            "phone.configure",
            change(Source::Phone, 0, true)
        )
        .is_err());
    }
    #[test]
    fn reminder_saved_config_is_retry_safe_and_old_revision_cannot_override() {
        let (store, sessions, id, _) = fixture(Source::Island);
        let token = open(&store, &sessions, &id);
        let args = change(Source::Island, 0, true);
        let saved = call(
            &store,
            &sessions,
            &id,
            &token,
            "island.configure",
            args.clone(),
        )
        .unwrap();
        assert_eq!(saved["revision"], 1);
        assert_eq!(
            call(
                &store,
                &sessions,
                &id,
                &token,
                "island.configure",
                args.clone()
            )
            .unwrap(),
            saved
        );
        let mut changed = args;
        changed["config"]["enabled"] = json!(false);
        assert!(
            call(&store, &sessions, &id, &token, "island.configure", changed)
                .unwrap_err()
                .contains("请求标识")
        );
        assert!(call(
            &store,
            &sessions,
            &id,
            &token,
            "island.configure",
            change(Source::Island, 0, false)
        )
        .unwrap_err()
        .starts_with("REVISION_CONFLICT:"));
    }
    #[test]
    fn reminder_save_failure_rolls_back_and_same_request_can_be_retried_after_repair() {
        let (store, sessions, id, _) = fixture(Source::Sit);
        let token = open(&store, &sessions, &id);
        let args = change(Source::Sit, 0, true);
        store.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_reminder BEFORE INSERT ON app_settings WHEN new.key LIKE 'reminder.source/%' BEGIN SELECT RAISE(FAIL,'storage denied'); END;").unwrap();
        assert!(call(
            &store,
            &sessions,
            &id,
            &token,
            "sit.configure",
            args.clone()
        )
        .is_err());
        assert!(enabled_sources(&store).unwrap().is_empty());
        store
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_reminder")
            .unwrap();
        assert_eq!(
            call(&store, &sessions, &id, &token, "sit.configure", args).unwrap()["revision"],
            1
        );
    }
    #[test]
    fn reminder_phone_has_four_fixed_disabled_rules_and_rejects_external_audio_or_duplicate_rule() {
        let (store, sessions, id, _) = fixture(Source::Phone);
        let token = open(&store, &sessions, &id);
        let value = call(&store, &sessions, &id, &token, "phone.get", Value::Null).unwrap();
        assert_eq!(value["config"]["rules"].as_array().unwrap().len(), 4);
        assert!(value["config"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["enabled"] == false));
        let mut args = change(Source::Phone, 0, true);
        args["config"]["rules"][0]["audio"] = json!("https://example.com/secret");
        assert!(call(&store, &sessions, &id, &token, "phone.configure", args).is_err());
        let mut args = change(Source::Phone, 0, true);
        args["config"]["rules"][1] = args["config"]["rules"][0].clone();
        assert!(call(&store, &sessions, &id, &token, "phone.configure", args).is_err());
    }
    #[test]
    fn reminder_delete_only_removes_target_config_and_keeps_other_enabled_source() {
        let (store, sessions, a, b) = fixture(Source::Island);
        for id in [&a, &b] {
            let token = open(&store, &sessions, id);
            call(
                &store,
                &sessions,
                id,
                &token,
                "island.configure",
                change(Source::Island, 0, true),
            )
            .unwrap();
        }
        widgets::remove_instance(&store, &sessions, &a).unwrap();
        assert_eq!(enabled_sources(&store).unwrap(), vec![(Source::Island, b)]);
        assert_eq!(
            store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM app_settings WHERE key=?1",
                    [key(&a)],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
    }
    #[test]
    fn reminder_missing_capability_does_not_gain_background_permissions() {
        let (store, sessions, id, _) = fixture(Source::Island);
        let token = open(&store, &sessions, &id);
        store.db.lock().unwrap().execute("UPDATE widget_packages SET manifest=replace(manifest,'notifications.island','storage.instance')",[]).unwrap();
        assert!(call(
            &store,
            &sessions,
            &id,
            &token,
            "island.configure",
            change(Source::Island, 0, true)
        )
        .is_err());
        assert!(enabled_sources(&store).unwrap().is_empty());
    }
    #[test]
    fn reminder_hidden_preview_cannot_enable_or_persist_background_configuration() {
        use std::{
            io::{Cursor, Write},
            sync::{Arc, Mutex},
        };
        let (store, sessions, _, _) = fixture(Source::Island);
        let mut manifest: Value = serde_json::from_str(
            &store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT manifest FROM widget_packages WHERE id=?1",
                    [Source::Island.package()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        // 正式包已登记0.1.0；试加载候选必须是更高版本，才会进入预览权限路径。
        manifest["version"] = json!("0.1.1");
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.json", options).unwrap();
        archive.write_all(manifest.to_string().as_bytes()).unwrap();
        archive.start_file("dist/index.html", options).unwrap();
        archive
            .write_all(b"<!doctype html><title>preview</title>")
            .unwrap();
        let package = widgets::stage_bytes(&store, archive.finish().unwrap().into_inner()).unwrap();
        let window = format!("trial-{}", uuid::Uuid::new_v4());
        let preview = Arc::new(Mutex::new(widgets::Preview {
            package,
            settings: json!({}),
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let session = widgets::open_preview(&sessions, &window, &window, preview).unwrap();
        assert!(authorized(
            &store,
            &sessions,
            &window,
            session["token"].as_str().unwrap(),
            "island.configure",
            change(Source::Island, 0, true)
        )
        .unwrap_err()
        .contains("试加载"));
        assert!(enabled_sources(&store).unwrap().is_empty());
    }
    #[test]
    fn reminder_same_capability_on_foreign_package_cannot_access_fixed_interface() {
        let (store, sessions, _, _) = fixture(Source::Island);
        let foreign = "local.test.foreign-island";
        let mut manifest: Value = serde_json::from_str(
            &store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT manifest FROM widget_packages WHERE id=?1",
                    [Source::Island.package()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        manifest["id"] = json!(foreign);
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'foreign','foreign')",params![foreign,manifest.to_string()]).unwrap();
        let id = widgets::create_instance(&store, foreign).unwrap();
        let token = open(&store, &sessions, &id);
        assert!(call(
            &store,
            &sessions,
            &id,
            &token,
            "island.configure",
            change(Source::Island, 0, true)
        )
        .unwrap_err()
        .contains("固定提醒组件"));
        assert!(enabled_sources(&store).unwrap().is_empty());
    }
}
