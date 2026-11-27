//! 固定番茄钟后台：阶段、场景、记录和完成提醒均独立于组件视图。
use crate::{
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use uuid::Uuid;
const CAPABILITY: &str = "focus.pomodoro";
const SCENE_LIMIT: usize = 128;
const HISTORY_LIMIT: usize = 8192;
const BYTE_LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    pomo_time: u32,
    short_break_time: u32,
    is_auto_next: bool,
    is_sound_enabled: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            pomo_time: 30,
            short_break_time: 5,
            is_auto_next: true,
            is_sound_enabled: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scene {
    id: String,
    name: String,
    icon: String,
    sort_order: u32,
    created_at: u64,
    updated_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    id: String,
    scene_id: String,
    start_time: u64,
    finish_time: u64,
    duration: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Phase {
    status: String,
    cycle_id: Option<String>,
    scene_id: Option<String>,
    started_at: u64,
    phase_started_at: Option<u64>,
    focus_millis: u64,
    rest_millis: u64,
}
impl Default for Phase {
    fn default() -> Self {
        Self {
            status: "stop".into(),
            cycle_id: None,
            scene_id: None,
            started_at: 0,
            phase_started_at: None,
            focus_millis: 0,
            rest_millis: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Notice {
    cycle_id: String,
    scene_id: String,
    nonce: String,
    run_id: String,
    notification: String,
    sound: String,
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    version: u32,
    revision: u64,
    initialized: bool,
    config: Config,
    scenes: Vec<Scene>,
    history: Vec<Record>,
    current_scene_id: Option<String>,
    phase: Phase,
    notice: Option<Notice>,
    receipts: Vec<Receipt>,
    message: Option<String>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            initialized: false,
            config: Config::default(),
            scenes: Vec::new(),
            history: Vec::new(),
            current_scene_id: None,
            phase: Phase::default(),
            notice: None,
            receipts: Vec::new(),
            message: None,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneInput {
    id: String,
    name: String,
    icon: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Change {
    request_id: String,
    expected_revision: u64,
    operation: String,
    #[serde(default)]
    config: Option<Config>,
    #[serde(default)]
    scene: Option<SceneInput>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    ids: Option<Vec<String>>,
    #[serde(default)]
    templates: Option<Vec<String>>,
}
#[derive(Clone, Debug)]
struct Pending {
    cycle_id: String,
    scene_id: String,
    nonce: String,
    title: String,
    sound: bool,
}
struct Delivery {
    notification: String,
    sound: String,
    error: Option<String>,
}
pub struct Service {
    busy: AtomicBool,
    run_id: String,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            busy: AtomicBool::new(false),
            run_id: Uuid::new_v4().to_string(),
        }
    }
}
fn key(id: &str) -> String {
    format!("pomodoro.document:{}", URL_SAFE_NO_PAD.encode(id))
}
fn access(db: &Connection, id: &str) -> AppResult<()> {
    if !crate::shared_library::native_active(db, id, CAPABILITY)? {
        return Err("CAPABILITY_DENIED: 番茄钟数据源未启用、已暂停或已移除".into());
    }
    Ok(())
}
fn validate_config(c: &Config) -> AppResult<()> {
    if !(5..=60).contains(&c.pomo_time) || !(1..=30).contains(&c.short_break_time) {
        return Err("INVALID_ARGUMENT: 专注须为5～60分钟，休息须为1～30分钟".into());
    }
    Ok(())
}
fn load(db: &Connection, id: &str) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(id)],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(Document::default());
    };
    if raw.len() > BYTE_LIMIT {
        return Err("DATA_UNAVAILABLE: 番茄钟数据超限，未覆盖".into());
    }
    let d: Document = serde_json::from_str(&raw)
        .map_err(|_| "DATA_UNAVAILABLE: 番茄钟数据损坏，未覆盖".to_owned())?;
    validate_config(&d.config)?;
    if d.version != 1
        || d.scenes.len() > SCENE_LIMIT
        || d.history.len() > HISTORY_LIMIT
        || d.receipts.len() > 128
        || ![
            "stop",
            "running",
            "pause",
            "rest-pause",
            "waiting",
            "resting",
        ]
        .contains(&d.phase.status.as_str())
    {
        return Err("DATA_UNAVAILABLE: 番茄钟数据版本或状态无效".into());
    }
    let mut scenes = std::collections::HashSet::new();
    for s in &d.scenes {
        validate_scene(&SceneInput {
            id: s.id.clone(),
            name: s.name.clone(),
            icon: s.icon.clone(),
        })?;
        if !scenes.insert(s.id.as_str()) {
            return Err("DATA_UNAVAILABLE: 场景标识重复".into());
        }
    }
    if d.current_scene_id
        .as_ref()
        .is_some_and(|id| !scenes.contains(id.as_str()))
        || d.phase
            .scene_id
            .as_ref()
            .is_some_and(|id| !scenes.contains(id.as_str()))
    {
        return Err("DATA_UNAVAILABLE: 当前场景已不存在".into());
    }
    let mut records = std::collections::HashSet::new();
    for h in &d.history {
        if Uuid::parse_str(&h.id).is_err()
            || !records.insert(h.id.as_str())
            || !scenes.contains(h.scene_id.as_str())
            || h.duration < 60
            || h.duration > 3600
            || h.finish_time < h.start_time
        {
            return Err("DATA_UNAVAILABLE: 专注记录无效".into());
        }
    }
    let p = &d.phase;
    let active = p.status != "stop";
    if (active
        && (p
            .cycle_id
            .as_ref()
            .is_none_or(|id| Uuid::parse_str(id).is_err())
            || p.scene_id.is_none()))
        || (!active && *p != Phase::default())
        || (["running", "resting"].contains(&p.status.as_str()) != p.phase_started_at.is_some())
        || p.focus_millis > 3_600_000
        || p.rest_millis > 1_800_000
    {
        return Err("DATA_UNAVAILABLE: 计时阶段不完整，未覆盖".into());
    }
    if let Some(n) = &d.notice {
        if p.status != "waiting"
            || p.cycle_id.as_ref() != Some(&n.cycle_id)
            || p.scene_id.as_ref() != Some(&n.scene_id)
            || Uuid::parse_str(&n.nonce).is_err()
            || ![
                "waiting",
                "sending",
                "accepted",
                "failed",
                "unknown",
                "dismissed",
            ]
            .contains(&n.notification.as_str())
            || ![
                "waiting", "sending", "accepted", "failed", "unknown", "disabled",
            ]
            .contains(&n.sound.as_str())
        {
            return Err("DATA_UNAVAILABLE: 完成提醒状态无效，未覆盖".into());
        }
    }
    Ok(d)
}
fn persist(db: &Connection, id: &str, d: &Document) -> AppResult<()> {
    let text = serde_json::to_string(d).map_err(err)?;
    if text.len() > BYTE_LIMIT {
        return Err("INVALID_ARGUMENT: 番茄钟数据超过4 MiB，请先删除不需要的记录".into());
    }
    db.execute("INSERT INTO app_settings(key,value)VALUES(?1,?2) ON CONFLICT(key)DO UPDATE SET value=excluded.value WHERE app_settings.value<>excluded.value",params![key(id),text]).map_err(err)?;
    crate::shared_library::persist_native_revision(db, id, d.revision)?;
    Ok(())
}
pub(crate) fn pause_source(db: &Connection, id: &str, now: u64) -> AppResult<()> {
    let mut d = load(db, id)?;
    let auto = d.config.is_auto_next;
    d.config.is_auto_next = false;
    advance(&mut d, now)?;
    d.config.is_auto_next = auto;
    if d.phase.status == "running" {
        d.phase.status = "pause".into();
        d.phase.phase_started_at = None;
    } else if d.phase.status == "resting" {
        d.phase.status = "rest-pause".into();
        d.phase.phase_started_at = None;
    }
    if let Some(notice) = d.notice.as_mut() {
        notice.nonce = Uuid::new_v4().to_string();
        if notice.notification == "waiting" {
            notice.notification = "dismissed".into();
            notice.sound = "disabled".into();
        } else if notice.notification == "sending" {
            notice.notification = "unknown".into();
            notice.sound = if notice.sound == "sending" {
                "unknown".into()
            } else {
                notice.sound.clone()
            };
            notice.error = Some("切换前提醒递送结果未确定，未自动重发".into());
        }
    }
    bump(&mut d)?;
    persist(db, id, &d)
}
pub(crate) fn copy_source(db: &Connection, source: &str, target: &str) -> AppResult<()> {
    let mut d = load(db, source)?;
    d.revision = 1;
    reset(&mut d);
    d.receipts.clear();
    d.message = None;
    persist(db, target, &d)
}
fn bump(d: &mut Document) -> AppResult<()> {
    d.revision = d
        .revision
        .checked_add(1)
        .ok_or("DATA_UNAVAILABLE: 番茄钟版本号超出范围")?;
    Ok(())
}
fn reset(d: &mut Document) {
    d.phase = Phase::default();
    d.notice = None;
}
fn start(d: &mut Document, now: u64) -> AppResult<()> {
    if d.phase.status == "rest-pause" {
        d.phase.status = "resting".into();
        d.phase.phase_started_at = Some(now);
        return Ok(());
    }
    if d.phase.status == "pause" {
        d.phase.status = "running".into();
        d.phase.phase_started_at = Some(now);
        return Ok(());
    }
    if !["stop", "resting"].contains(&d.phase.status.as_str()) {
        return Err("INVALID_ARGUMENT: 请先完成当前专注".into());
    }
    let scene = d
        .current_scene_id
        .clone()
        .filter(|id| d.scenes.iter().any(|s| s.id == *id))
        .ok_or("INVALID_ARGUMENT: 请先选择专注场景")?;
    d.phase = Phase {
        status: "running".into(),
        cycle_id: Some(Uuid::new_v4().to_string()),
        scene_id: Some(scene),
        started_at: now,
        phase_started_at: Some(now),
        focus_millis: 0,
        rest_millis: 0,
    };
    d.notice = None;
    Ok(())
}
fn advance(d: &mut Document, now: u64) -> AppResult<()> {
    let old = d.phase.status.clone();
    if old == "running" || old == "resting" {
        let at = d
            .phase
            .phase_started_at
            .ok_or("DATA_UNAVAILABLE: 计时阶段缺少锚点")?;
        let elapsed = now.saturating_sub(at);
        d.phase.phase_started_at = Some(now);
        if old == "running" {
            let limit = u64::from(d.config.pomo_time) * 60000;
            d.phase.focus_millis = d
                .phase
                .focus_millis
                .checked_add(elapsed.min(limit.saturating_sub(d.phase.focus_millis)))
                .ok_or("计时超出范围")?;
            if d.phase.focus_millis >= limit {
                d.phase.status = "waiting".into();
                d.phase.phase_started_at = None;
                let cycle = d.phase.cycle_id.clone().ok_or("缺少专注周期")?;
                let scene = d.phase.scene_id.clone().ok_or("缺少专注场景")?;
                d.notice = Some(Notice {
                    cycle_id: cycle,
                    scene_id: scene,
                    nonce: Uuid::new_v4().to_string(),
                    run_id: String::new(),
                    notification: "waiting".into(),
                    sound: if d.config.is_sound_enabled {
                        "waiting"
                    } else {
                        "disabled"
                    }
                    .into(),
                    error: None,
                });
            }
        } else {
            d.phase.rest_millis = d
                .phase
                .rest_millis
                .checked_add(
                    elapsed.min(
                        (u64::from(d.config.short_break_time) * 60000)
                            .saturating_sub(d.phase.rest_millis),
                    ),
                )
                .ok_or("计时超出范围")?;
            if d.phase.rest_millis >= u64::from(d.config.short_break_time) * 60000 {
                reset(d);
                if d.config.is_auto_next && d.current_scene_id.is_some() {
                    start(d, now)?;
                }
            }
        }
    }
    if d.phase.status != old {
        bump(d)?;
    }
    Ok(())
}
fn snapshot(d: &Document, now: u64) -> Value {
    let mut scenes = d.scenes.clone();
    scenes.sort_by_key(|s| (s.sort_order, s.created_at));
    let scene_values: Vec<_> = scenes
        .iter()
        .map(|s| {
            let duration = d
                .history
                .iter()
                .filter(|h| h.scene_id == s.id)
                .map(|h| h.duration)
                .sum::<u64>();
            let mut value = serde_json::to_value(s).expect("固定场景可序列化");
            value["duration"] = json!(duration);
            value
        })
        .collect();
    let mut history = d.history.clone();
    history.sort_by_key(|h| std::cmp::Reverse((h.start_time, h.id.clone())));
    let target = if matches!(d.phase.status.as_str(), "resting" | "rest-pause") {
        u64::from(d.config.short_break_time) * 60000
    } else {
        u64::from(d.config.pomo_time) * 60000
    };
    let spent = if matches!(d.phase.status.as_str(), "resting" | "rest-pause") {
        d.phase.rest_millis
    } else {
        d.phase.focus_millis
    };
    let progress = if d.phase.status == "stop" || d.phase.status == "waiting" {
        100.
    } else {
        (target.saturating_sub(spent) as f64 / target as f64 * 100.).round()
    };
    json!({"version":1,"revision":d.revision,"initialized":d.initialized,"sampledAt":now,"config":d.config,"scenes":scene_values,"history":history,"currentSceneId":d.current_scene_id,"phase":d.phase,"remainingSeconds":target.saturating_sub(spent).div_ceil(1000),"progress":progress,"notice":d.notice.as_ref().map(|n|json!({"cycleId":n.cycle_id,"notification":n.notification,"sound":n.sound,"error":n.error})),"message":d.message})
}
pub(crate) fn read(store: &Store, id: &str, now: u64) -> AppResult<Value> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    access(&tx, id)?;
    let mut d = load(&tx, id)?;
    let before = serde_json::to_string(&d).map_err(err)?;
    advance(&mut d, now)?;
    if serde_json::to_string(&d).map_err(err)? != before {
        persist(&tx, id, &d)?;
    }
    let value = snapshot(&d, now);
    tx.commit().map_err(err)?;
    Ok(value)
}
fn finish(d: &mut Document, now: u64) -> AppResult<()> {
    if matches!(d.phase.status.as_str(), "resting" | "rest-pause") {
        reset(d);
        return Ok(());
    }
    if !["running", "pause", "waiting"].contains(&d.phase.status.as_str()) {
        return Err("INVALID_ARGUMENT: 当前没有可结束的专注".into());
    }
    if d.phase.focus_millis < 60000 {
        reset(d);
        d.message = Some("专注时间少于1分钟，不作记录".into());
        return Ok(());
    }
    if d.history.len() >= HISTORY_LIMIT {
        return Err("INVALID_ARGUMENT: 专注记录达到8192条，请先删除不需要的记录".into());
    }
    let id = d
        .phase
        .cycle_id
        .clone()
        .ok_or("INVALID_ARGUMENT: 缺少专注周期")?;
    if !d.history.iter().any(|h| h.id == id) {
        d.history.push(Record {
            id,
            scene_id: d
                .phase
                .scene_id
                .clone()
                .ok_or("INVALID_ARGUMENT: 缺少场景")?,
            start_time: d.phase.started_at,
            finish_time: now.max(d.phase.started_at),
            duration: d.phase.focus_millis / 1000,
        });
    }
    d.phase.status = "resting".into();
    d.phase.rest_millis = 0;
    d.phase.phase_started_at = Some(now);
    d.notice = None;
    Ok(())
}
fn validate_scene(s: &SceneInput) -> AppResult<()> {
    if Uuid::parse_str(&s.id).is_err()
        || s.name.trim().is_empty()
        || s.name.chars().count() > 120
        || s.icon.trim().is_empty()
        || s.icon.chars().count() > 32
    {
        return Err("INVALID_ARGUMENT: 场景需要UUID、1～120字名称和1～32字图标".into());
    }
    Ok(())
}
fn mutate(d: &mut Document, c: Change, now: u64) -> AppResult<()> {
    match c.operation.as_str() {
        "initialize" => {
            if d.initialized {
                return Err("INVALID_ARGUMENT: 初始场景已经选择".into());
            }
            let choices = c.templates.ok_or("INVALID_ARGUMENT: 缺少场景选择")?;
            let mut seen = std::collections::HashSet::new();
            for choice in choices {
                if !seen.insert(choice.clone()) {
                    return Err("INVALID_ARGUMENT: 初始场景重复".into());
                }
                let (name, icon) = match choice.as_str() {
                    "reading" => ("阅读", "📖"),
                    "coding" => ("敲代码", "👨‍💻"),
                    "working" => ("打工", "🔨"),
                    "relaxing" => ("摸鱼", "🐟"),
                    _ => return Err("INVALID_ARGUMENT: 未知初始场景".into()),
                };
                d.scenes.push(Scene {
                    id: Uuid::new_v4().to_string(),
                    name: name.into(),
                    icon: icon.into(),
                    sort_order: d.scenes.len() as u32,
                    created_at: now,
                    updated_at: now,
                });
            }
            d.initialized = true;
            d.current_scene_id = d.scenes.first().map(|s| s.id.clone());
        }
        "scene.save" => {
            let s = c.scene.ok_or("INVALID_ARGUMENT: 缺少场景")?;
            validate_scene(&s)?;
            if let Some(old) = d.scenes.iter_mut().find(|old| old.id == s.id) {
                old.name = s.name;
                old.icon = s.icon;
                old.updated_at = now;
            } else {
                if d.scenes.len() >= SCENE_LIMIT {
                    return Err("INVALID_ARGUMENT: 场景最多128项".into());
                }
                d.scenes.push(Scene {
                    id: s.id.clone(),
                    name: s.name,
                    icon: s.icon,
                    sort_order: d.scenes.len() as u32,
                    created_at: now,
                    updated_at: now,
                });
                if d.current_scene_id.is_none() {
                    d.current_scene_id = Some(s.id);
                }
            }
            d.initialized = true;
        }
        "scene.delete" => {
            let id = c.id.ok_or("INVALID_ARGUMENT: 缺少场景标识")?;
            if d.phase.scene_id.as_deref() == Some(&id)
                && ["running", "pause", "waiting", "rest-pause"].contains(&d.phase.status.as_str())
            {
                return Err("INVALID_ARGUMENT: 请先结束或重置该场景的专注".into());
            }
            let index = d
                .scenes
                .iter()
                .position(|s| s.id == id)
                .ok_or("INVALID_ARGUMENT: 场景不存在")?;
            d.scenes.remove(index);
            d.history.retain(|h| h.scene_id != id);
            if d.phase.scene_id.as_deref() == Some(&id) {
                reset(d);
            }
            if d.current_scene_id.as_deref() == Some(&id) {
                d.current_scene_id = d
                    .scenes
                    .iter()
                    .min_by_key(|s| s.sort_order)
                    .map(|s| s.id.clone());
            }
        }
        "select" => {
            if ["running", "pause", "waiting"].contains(&d.phase.status.as_str()) {
                return Err("INVALID_ARGUMENT: 请先结束当前专注再切换场景".into());
            }
            let id = c.id.ok_or("INVALID_ARGUMENT: 缺少场景")?;
            if !d.scenes.iter().any(|s| s.id == id) {
                return Err("INVALID_ARGUMENT: 场景不存在".into());
            }
            d.current_scene_id = Some(id);
        }
        "reorder" => {
            let ids = c.ids.ok_or("INVALID_ARGUMENT: 缺少场景顺序")?;
            let given: std::collections::HashSet<_> = ids.iter().collect();
            if given.len() != ids.len()
                || ids.len() != d.scenes.len()
                || d.scenes.iter().any(|s| !given.contains(&s.id))
            {
                return Err("INVALID_ARGUMENT: 顺序必须包含全部场景且不重复".into());
            }
            for (index, id) in ids.iter().enumerate() {
                d.scenes
                    .iter_mut()
                    .find(|s| s.id == *id)
                    .ok_or("场景不存在")?
                    .sort_order = index as u32;
            }
        }
        "configure" => {
            let config = c.config.ok_or("INVALID_ARGUMENT: 缺少番茄钟设置")?;
            validate_config(&config)?;
            d.config = config;
            advance(d, now)?;
        }
        "start" => start(d, now)?,
        "pause" => {
            if d.phase.status != "running" {
                return Err("INVALID_ARGUMENT: 仅正在专注时可以暂停".into());
            }
            d.phase.status = "pause".into();
            d.phase.phase_started_at = None;
        }
        "finish" => finish(d, now)?,
        "reset" => reset(d),
        "history.delete" => {
            let id = c.id.ok_or("INVALID_ARGUMENT: 缺少记录标识")?;
            let index = d
                .history
                .iter()
                .position(|h| h.id == id)
                .ok_or("INVALID_ARGUMENT: 记录不存在")?;
            d.history.remove(index);
        }
        _ => return Err("INVALID_ARGUMENT: 未知番茄钟操作".into()),
    }
    Ok(())
}
// 调用方须在原生动作锁内完成会话和当前源鉴权；这里仍校验源、版本和业务规则。
pub(crate) fn change(store: &Store, id: &str, args: Value, now: u64) -> AppResult<Value> {
    let c: Change =
        serde_json::from_value(args.clone()).map_err(|e| format!("INVALID_ARGUMENT: {e}"))?;
    Uuid::parse_str(&c.request_id).map_err(|_| "INVALID_ARGUMENT: 请求标识须为UUID".to_owned())?;
    let allowed = match c.operation.as_str() {
        "initialize" => "templates",
        "scene.save" => "scene",
        "scene.delete" | "history.delete" | "select" => "id",
        "reorder" => "ids",
        "configure" => "config",
        _ => "",
    };
    for k in ["templates", "scene", "id", "ids", "config"] {
        if args.get(k).is_some() && k != allowed {
            return Err("INVALID_ARGUMENT: 番茄钟参数与操作不匹配".into());
        }
    }
    let fingerprint = format!("{:x}", Sha256::digest(args.to_string().as_bytes()));
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    access(&tx, id)?;
    let mut d = load(&tx, id)?;
    if let Some(r) = d.receipts.iter().find(|r| r.id == c.request_id) {
        if r.fingerprint != fingerprint {
            return Err("INVALID_ARGUMENT: 请求标识已用于其他操作".into());
        }
        advance(&mut d, now)?;
        persist(&tx, id, &d)?;
        let value = snapshot(&d, now);
        tx.commit().map_err(err)?;
        return Ok(value);
    }
    advance(&mut d, now)?;
    if d.revision != c.expected_revision {
        return Err("REVISION_CONFLICT: 番茄钟已更新，请重新读取".into());
    }
    let request_id = c.request_id.clone();
    d.message = None;
    mutate(&mut d, c, now)?;
    bump(&mut d)?;
    d.receipts.push(Receipt {
        id: request_id,
        fingerprint,
    });
    if d.receipts.len() > 128 {
        d.receipts.remove(0);
    }
    persist(&tx, id, &d)?;
    let value = snapshot(&d, now);
    tx.commit().map_err(err)?;
    Ok(value)
}
fn reserve(store: &Store, id: &str, now: u64, run_id: &str) -> AppResult<Option<Pending>> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    if !crate::shared_library::native_active(&tx, id, CAPABILITY)? {
        return Ok(None);
    }
    let mut d = load(&tx, id)?;
    let before = serde_json::to_string(&d).map_err(err)?;
    advance(&mut d, now)?;
    let result = if let Some(n) = d.notice.as_mut() {
        if n.notification == "sending" && n.run_id != run_id {
            n.notification = "unknown".into();
            if n.sound == "sending" {
                n.sound = "unknown".into();
            }
            n.error = Some("上次完成提醒递送结果未确定，未自动重发".into());
        }
        if d.phase.status == "waiting" && n.notification == "waiting" {
            let title = d
                .scenes
                .iter()
                .find(|s| s.id == n.scene_id)
                .map(|s| s.name.clone())
                .ok_or("当前场景不存在")?;
            n.notification = "sending".into();
            n.run_id = run_id.into();
            n.sound = if d.config.is_sound_enabled {
                "sending"
            } else {
                "disabled"
            }
            .into();
            Some(Pending {
                cycle_id: n.cycle_id.clone(),
                scene_id: n.scene_id.clone(),
                nonce: n.nonce.clone(),
                title,
                sound: d.config.is_sound_enabled,
            })
        } else {
            None
        }
    } else {
        None
    };
    if serde_json::to_string(&d).map_err(err)? != before {
        persist(&tx, id, &d)?;
    }
    tx.commit().map_err(err)?;
    Ok(result)
}
fn current(d: &Document, p: &Pending) -> bool {
    d.phase.status == "waiting"
        && d.phase.cycle_id.as_deref() == Some(&p.cycle_id)
        && d.phase.scene_id.as_deref() == Some(&p.scene_id)
        && d.notice.as_ref().is_some_and(|n| {
            n.nonce == p.nonce
                && n.cycle_id == p.cycle_id
                && ["sending", "accepted"].contains(&n.notification.as_str())
        })
}
fn tick(
    store: &Store,
    id: &str,
    now: u64,
    run_id: &str,
    send: &mut impl FnMut(Pending) -> AppResult<Delivery>,
) -> AppResult<()> {
    let _native = store.native_actions.lock().map_err(err)?;
    if let Some(p) = reserve(store, id, now, run_id)? {
        {
            let db = store.db.lock().map_err(err)?;
            if !crate::shared_library::native_active(&db, id, CAPABILITY)? {
                return Ok(());
            }
            access(&db, id)?;
            if !current(&load(&db, id)?, &p) {
                return Ok(());
            }
        }
        let result = send(p.clone());
        let mut db = store.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let mut d = load(&tx, id)?;
        if let Some(n) = d
            .notice
            .as_mut()
            .filter(|n| n.nonce == p.nonce && n.notification == "sending")
        {
            match result {
                Ok(delivery) => {
                    n.notification = delivery.notification;
                    n.sound = delivery.sound;
                    n.error = delivery.error;
                }
                Err(error) => {
                    n.notification = "failed".into();
                    n.sound = if p.sound { "failed" } else { "disabled" }.into();
                    n.error = Some(error);
                }
            }
            persist(&tx, id, &d)?;
        }
        tx.commit().map_err(err)?;
    }
    Ok(())
}
fn acknowledge(store: &Store, id: &str, p: &Pending, complete: bool, now: u64) -> AppResult<bool> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    if !crate::shared_library::native_active(&tx, id, CAPABILITY)? {
        return Ok(false);
    }
    let mut d = load(&tx, id)?;
    if !current(&d, p) {
        return Ok(false);
    }
    if complete {
        finish(&mut d, now)?;
        bump(&mut d)?;
    } else if let Some(n) = d.notice.as_mut() {
        n.notification = "dismissed".into();
    }
    persist(&tx, id, &d)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
pub fn remove_data(db: &Connection, id: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(id)])
        .map_err(err)?;
    Ok(())
}
pub(crate) fn authorized(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
    now: u64,
) -> AppResult<Value> {
    let _native = store.native_actions.lock().map_err(err)?;
    widgets::with_capability(store, sessions, window, token, CAPABILITY, |session| {
        let id = {
            let db = store.db.lock().map_err(err)?;
            crate::shared_library::native_source(&db, &session.instance, CAPABILITY)?
        };
        let value = match method {
            "pomodoro.get" if args.is_null() || args.as_object().is_some_and(|o| o.is_empty()) => {
                read(store, &id, now)
            }
            "pomodoro.change" => change(store, &id, args, now),
            _ => Err("INVALID_ARGUMENT: 番茄钟接口或参数无效".into()),
        }?;
        crate::shared_library::native_cursor(session, &id, &value);
        Ok(value)
    })
}
pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let handle = app.clone();
    let (window, token, method) = (window.to_owned(), token.to_owned(), method.to_owned());
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        authorized(
            &core.store,
            &core.sessions,
            &window,
            &token,
            &method,
            args,
            now_ms(),
        )
    })
    .await
    .map_err(err)?
}
impl Service {
    pub fn dispatch(&self, app: &AppHandle) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            struct Release(AppHandle);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0
                        .state::<Core>()
                        .pomodoro
                        .busy
                        .store(false, Ordering::Release);
                }
            }
            let _release = Release(handle.clone());
            let core = handle.state::<Core>();
            let result = (|| -> AppResult<()> {
                let ids = {
                    let db = core.store.db.lock().map_err(err)?;
                    crate::shared_library::native_sources(&db, CAPABILITY)?
                };
                for id in ids {
                    if core.quitting.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(error) = tick(
                        &core.store,
                        &id,
                        now_ms(),
                        &core.pomodoro.run_id,
                        &mut |p| notify(&handle, &id, &p),
                    ) {
                        core.report(format!("番茄钟：{error}"));
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                core.report(format!("番茄钟：{error}"));
            }
        });
    }
}
#[cfg(windows)]
fn play_completion_sound() -> AppResult<()> {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
    let bytes = include_bytes!("../component-assets/pomodoro-ding.wav");
    if unsafe {
        PlaySoundW(
            bytes.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_SYNC | SND_NODEFAULT,
        )
    } == 0
    {
        return Err("原番茄钟提示音播放调用失败".into());
    }
    Ok(())
}
#[cfg(windows)]
fn notify(app: &AppHandle, id: &str, p: &Pending) -> AppResult<Delivery> {
    use tauri_winrt_notification::{Duration, Toast};
    let core = app.state::<Core>();
    let store = std::sync::Arc::downgrade(&core.store);
    let errors = std::sync::Arc::downgrade(&core.last_error);
    let id = id.to_owned();
    let pending = p.clone();
    let notification = crate::reminder_store::ensure_windows_enabled(&core.store).and_then(|()| {
        Toast::new(&app.config().identifier)
            .title("专注已结束")
            .text1(&p.title)
            .duration(Duration::Long)
            .sound(None)
            .add_button("知道了", "close")
            .add_button("结束专注", "finish")
            .on_activated(move |action| {
                if let (Some(store), Some(action)) = (store.upgrade(), action) {
                    if ["close", "finish"].contains(&action.as_str()) {
                        if let Err(error) =
                            acknowledge(&store, &id, &pending, action == "finish", now_ms())
                        {
                            if let Some(slot) = errors.upgrade() {
                                if let Ok(mut value) = slot.lock() {
                                    *value = Some(format!("番茄钟回执失败：{error}"));
                                }
                            }
                        }
                    }
                }
                Ok(())
            })
            .show()
            .map_err(err)
    });
    let sound = if p.sound {
        play_completion_sound()
    } else {
        Ok(())
    };
    let mut errors = Vec::new();
    if let Err(error) = &notification {
        if error != crate::reminder_store::WINDOWS_DISABLED {
            errors.push(error.clone());
        }
    }
    if let Err(error) = &sound {
        errors.push(error.clone());
    }
    Ok(Delivery {
        notification: match notification.as_ref().err().map(String::as_str) {
            None => "accepted",
            Some(crate::reminder_store::WINDOWS_DISABLED) => "suppressed",
            Some(_) => "failed",
        }
        .into(),
        sound: if !p.sound {
            "disabled"
        } else if sound.is_ok() {
            "accepted"
        } else {
            "failed"
        }
        .into(),
        error: if errors.is_empty() {
            None
        } else {
            Some(errors.join("；"))
        },
    })
}
#[cfg(not(windows))]
fn notify(_app: &AppHandle, _id: &str, _p: &Pending) -> AppResult<Delivery> {
    Err("当前平台未实现番茄钟完成提醒".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pomodoro_source_switch_preserves_paused_rest_until_explicit_resume() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, true);
        send(&s, &a, "start", json!({}), AT);
        send(&s, &a, "finish", json!({}), AT + 70000);
        pause_source(&s.db.lock().unwrap(), &a, AT + 80000).unwrap();
        let paused = read(&s, &a, AT + 180000).unwrap();
        assert_eq!(paused["phase"]["status"], "rest-pause");
        assert_eq!(paused["remainingSeconds"], 50);
        assert_eq!(paused["phase"]["restMillis"], 10000);
        send(&s, &a, "start", json!({}), AT + 180000);
        let resumed = read(&s, &a, AT + 200000).unwrap();
        assert_eq!(resumed["phase"]["status"], "resting");
        assert_eq!(resumed["remainingSeconds"], 30);
    }
    #[test]
    fn native_pomodoro_pause_at_rest_end_does_not_auto_start_a_new_focus() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, true);
        send(&s, &a, "start", json!({}), AT);
        send(&s, &a, "finish", json!({}), AT + 70000);
        pause_source(&s.db.lock().unwrap(), &a, AT + 140000).unwrap();
        assert_eq!(
            read(&s, &a, AT + 200000).unwrap()["phase"]["status"],
            "stop"
        );
    }
    #[test]
    fn native_pomodoro_old_completion_callback_cannot_modify_a_paused_source() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, false);
        send(&s, &a, "start", json!({}), AT);
        let pending = reserve(&s, &a, AT + 300000, "source-test")
            .unwrap()
            .unwrap();
        pause_source(&s.db.lock().unwrap(), &a, AT + 300001).unwrap();
        assert!(!acknowledge(&s, &a, &pending, true, AT + 300002).unwrap());
        assert_eq!(
            read(&s, &a, AT + 300002).unwrap()["notice"]["notification"],
            "unknown"
        );
    }
    use serde_json::{json, Value};
    use uuid::Uuid;
    const AT: u64 = 1_800_000_000_000;
    #[test]
    fn pomodoro_detail_and_primary_sessions_coexist_without_alert_permissions() {
        let (s, a, _) = fixture();
        let sessions = widgets::Sessions::default();
        let primary =
            widgets::open_display_session(&s, &sessions, &format!("float-{a}"), &a, None).unwrap();
        let detail =
            widgets::open_display_session(&s, &sessions, &format!("detail-{a}"), &a, None).unwrap();
        assert_eq!(detail["role"], "details");
        for (window, session) in [
            (format!("float-{a}"), primary),
            (format!("detail-{a}"), detail),
        ] {
            assert_eq!(
                widgets::require_capability(
                    &s,
                    &sessions,
                    &window,
                    session["token"].as_str().unwrap(),
                    CAPABILITY
                )
                .unwrap(),
                a
            );
        }
    }
    #[test]
    fn corrupt_phase_and_future_version_are_rejected_without_overwriting() {
        let (s, a, _) = fixture();
        init(&s, &a);
        send(&s, &a, "start", json!({}), AT);
        let base: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [key(&a)],
                    |r| r.get(0),
                )
                .unwrap();
        for (field, value) in [
            ("cycleId", Value::Null),
            ("sceneId", Value::Null),
            ("focusMillis", json!(3600001)),
            ("phaseStartedAt", Value::Null),
        ] {
            let mut data: Value = serde_json::from_str(&base).unwrap();
            data["phase"][field] = value;
            let raw = data.to_string();
            s.db.lock()
                .unwrap()
                .execute(
                    "UPDATE app_settings SET value=?1 WHERE key=?2",
                    params![raw, key(&a)],
                )
                .unwrap();
            assert!(read(&s, &a, AT).is_err(), "损坏阶段应拒绝：{field}");
            let retained: String =
                s.db.lock()
                    .unwrap()
                    .query_row(
                        "SELECT value FROM app_settings WHERE key=?1",
                        [key(&a)],
                        |r| r.get(0),
                    )
                    .unwrap();
            assert_eq!(retained, raw);
        }
        let mut future: Value = serde_json::from_str(&base).unwrap();
        future["version"] = json!(2);
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value=?1 WHERE key=?2",
                params![future.to_string(), key(&a)],
            )
            .unwrap();
        assert!(read(&s, &a, AT).is_err());
    }
    fn fixture() -> (crate::store::Store, String, String) {
        let s = crate::store::Store::open(
            std::env::temp_dir().join(format!("souldesk-pomodoro-{}", Uuid::new_v4())),
        )
        .unwrap();
        let manifest = json!({"schemaVersion":1,"id":"local.test.pomodoro","name":"番茄检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":150,"height":150},"minSize":{"width":150,"height":150},"capabilities":["focus.pomodoro"],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        s.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest)VALUES(?1,?2,'pomo-test','pomo-test')",rusqlite::params!["local.test.pomodoro",manifest.to_string()]).unwrap();
        let a = crate::widgets::create_instance(&s, "local.test.pomodoro").unwrap();
        let b = crate::widgets::create_instance(&s, "local.test.pomodoro").unwrap();
        (s, a, b)
    }
    fn request(rev: u64, op: &str) -> Value {
        json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":rev,"operation":op})
    }
    fn send(s: &crate::store::Store, id: &str, op: &str, fields: Value, now: u64) -> Value {
        let mut r = request(read(s, id, now).unwrap()["revision"].as_u64().unwrap(), op);
        for (k, v) in fields.as_object().unwrap() {
            r[k] = v.clone();
        }
        change(s, id, r, now).unwrap()
    }
    fn init(s: &crate::store::Store, id: &str) -> String {
        let d = send(
            s,
            id,
            "initialize",
            json!({"templates":["reading","coding","working","relaxing"]}),
            AT,
        );
        d["scenes"][0]["id"].as_str().unwrap().into()
    }
    fn config(s: &crate::store::Store, id: &str, auto: bool) -> Value {
        send(
            s,
            id,
            "configure",
            json!({"config":{"pomoTime":5,"shortBreakTime":1,"isAutoNext":auto,"isSoundEnabled":true}}),
            AT,
        )
    }
    #[test]
    fn default_scenes_can_be_selected_or_skipped_and_do_not_reappear() {
        let (s, a, b) = fixture();
        let d = read(&s, &a, AT).unwrap();
        assert_eq!(d["config"]["pomoTime"], 30);
        assert_eq!(d["initialized"], false);
        assert!(d["scenes"].as_array().unwrap().is_empty());
        init(&s, &a);
        assert_eq!(
            read(&s, &a, AT).unwrap()["scenes"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        send(&s, &b, "initialize", json!({"templates":[]}), AT);
        assert!(read(&s, &b, AT).unwrap()["scenes"]
            .as_array()
            .unwrap()
            .is_empty());
        let root = s.root.clone();
        drop(s);
        assert_eq!(
            read(&crate::store::Store::open(root).unwrap(), &b, AT).unwrap()["initialized"],
            true
        );
    }
    #[test]
    fn pause_resume_and_restart_count_only_active_time_with_millisecond_precision() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, false);
        send(&s, &a, "start", json!({}), AT);
        let d = send(&s, &a, "pause", json!({}), AT + 30500);
        assert_eq!(d["phase"]["focusMillis"], 30500);
        let root = s.root.clone();
        drop(s);
        let s = crate::store::Store::open(root).unwrap();
        assert_eq!(
            read(&s, &a, AT + 100000).unwrap()["phase"]["focusMillis"],
            30500
        );
        send(&s, &a, "start", json!({}), AT + 100000);
        let d = read(&s, &a, AT + 110500).unwrap();
        assert_eq!(d["phase"]["focusMillis"], 41000);
        assert_eq!(d["phase"]["status"], "running");
    }
    #[test]
    fn focus_end_waits_for_confirmation_and_records_once_then_rests() {
        let (s, a, _) = fixture();
        let scene = init(&s, &a);
        config(&s, &a, false);
        send(&s, &a, "start", json!({}), AT);
        let d = read(&s, &a, AT + 300000).unwrap();
        assert_eq!(d["phase"]["status"], "waiting");
        assert!(d["history"].as_array().unwrap().is_empty());
        let r = request(d["revision"].as_u64().unwrap(), "finish");
        let d = change(&s, &a, r.clone(), AT + 310000).unwrap();
        assert_eq!(d["phase"]["status"], "resting");
        assert_eq!(d["history"][0]["sceneId"], scene);
        assert_eq!(d["history"][0]["duration"], 300);
        assert_eq!(
            change(&s, &a, r, AT + 310001).unwrap()["history"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            read(&s, &a, AT + 370000).unwrap()["phase"]["status"],
            "stop"
        );
    }
    #[test]
    fn short_focus_is_not_recorded_and_automatic_rest_starts_only_one_new_cycle() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, true);
        send(&s, &a, "start", json!({}), AT);
        let d = send(&s, &a, "finish", json!({}), AT + 59999);
        assert_eq!(d["phase"]["status"], "stop");
        assert!(d["history"].as_array().unwrap().is_empty());
        assert!(d["message"].as_str().unwrap().contains("少于1分钟"));
        send(&s, &a, "start", json!({}), AT + 60000);
        send(&s, &a, "finish", json!({}), AT + 120000);
        let d = read(&s, &a, AT + 900000).unwrap();
        assert_eq!(d["phase"]["status"], "running");
        assert_eq!(d["phase"]["focusMillis"], 0);
        assert_eq!(d["history"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn scene_identity_is_bound_to_cycle_and_history_deletion_updates_totals() {
        let (s, a, _) = fixture();
        let scene = init(&s, &a);
        send(&s, &a, "start", json!({}), AT);
        let mut r = request(
            read(&s, &a, AT).unwrap()["revision"].as_u64().unwrap(),
            "select",
        );
        r["id"] = read(&s, &a, AT).unwrap()["scenes"][1]["id"].clone();
        assert!(change(&s, &a, r, AT).is_err());
        let mut r = request(
            read(&s, &a, AT).unwrap()["revision"].as_u64().unwrap(),
            "scene.delete",
        );
        r["id"] = json!(scene);
        assert!(change(&s, &a, r, AT).is_err());
        let d = send(&s, &a, "finish", json!({}), AT + 60000);
        assert_eq!(d["scenes"][0]["duration"], 60);
        let id = d["history"][0]["id"].clone();
        let d = send(&s, &a, "history.delete", json!({"id":id}), AT + 60000);
        assert_eq!(d["scenes"][0]["duration"], 0);
        assert!(d["history"].as_array().unwrap().is_empty());
    }
    #[test]
    fn settings_changes_keep_elapsed_time_and_clock_rollback_never_loses_focus() {
        let (s, a, _) = fixture();
        init(&s, &a);
        send(&s, &a, "start", json!({}), AT);
        let d = read(&s, &a, AT + 720000).unwrap();
        assert_eq!(d["phase"]["focusMillis"], 720000);
        let d = send(
            &s,
            &a,
            "configure",
            json!({"config":{"pomoTime":5,"shortBreakTime":1,"isAutoNext":false,"isSoundEnabled":false}}),
            AT + 720000,
        );
        assert_eq!(d["phase"]["status"], "waiting");
        assert_eq!(d["phase"]["focusMillis"], 720000);
        send(&s, &a, "reset", json!({}), AT + 720000);
        send(&s, &a, "start", json!({}), AT + 720000);
        send(&s, &a, "pause", json!({}), AT + 750500);
        send(&s, &a, "start", json!({}), AT + 760000);
        assert_eq!(
            read(&s, &a, AT + 755000).unwrap()["phase"]["focusMillis"],
            30500
        );
        assert_eq!(
            read(&s, &a, AT + 756000).unwrap()["phase"]["focusMillis"],
            31500
        );
    }
    #[test]
    fn mutations_are_revisioned_retry_safe_and_isolated() {
        let (s, a, b) = fixture();
        let mut r = request(0, "initialize");
        r["templates"] = json!(["reading"]);
        let d = change(&s, &a, r.clone(), AT).unwrap();
        assert_eq!(
            change(&s, &a, r.clone(), AT).unwrap()["revision"],
            d["revision"]
        );
        r["templates"] = json!(["coding"]);
        assert!(change(&s, &a, r, AT).is_err());
        assert!(change(&s, &a, request(0, "start"), AT).is_err());
        assert_eq!(read(&s, &b, AT).unwrap()["initialized"], false);
    }
    #[test]
    fn invalid_config_scene_order_and_unknown_arguments_do_not_overwrite_data() {
        let (s, a, _) = fixture();
        let scene = init(&s, &a);
        let before = read(&s, &a, AT).unwrap();
        for cfg in [
            json!({"pomoTime":4,"shortBreakTime":1,"isAutoNext":true,"isSoundEnabled":true}),
            json!({"pomoTime":30,"shortBreakTime":31,"isAutoNext":true,"isSoundEnabled":true}),
        ] {
            let mut r = request(before["revision"].as_u64().unwrap(), "configure");
            r["config"] = cfg;
            assert!(change(&s, &a, r, AT).is_err());
        }
        for fields in [
            json!({"scene":{"id":scene,"name":" ","icon":"📖"}}),
            json!({"scene":{"id":"bad","name":"a","icon":"x"}}),
        ] {
            let mut r = request(before["revision"].as_u64().unwrap(), "scene.save");
            r["scene"] = fields["scene"].clone();
            assert!(change(&s, &a, r, AT).is_err());
        }
        let mut r = request(before["revision"].as_u64().unwrap(), "reorder");
        r["ids"] = json!([scene]);
        assert!(change(&s, &a, r, AT).is_err());
        assert_eq!(read(&s, &a, AT).unwrap(), before);
    }
    #[test]
    fn hidden_timer_reserves_completion_once_and_reports_delivery_facts() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, false);
        send(&s, &a, "start", json!({}), AT);
        let service = Service::default();
        let mut sent = Vec::new();
        tick(&s, &a, AT + 300000, &service.run_id, &mut |p| {
            sent.push(p);
            Ok(Delivery {
                notification: "accepted".into(),
                sound: "accepted".into(),
                error: None,
            })
        })
        .unwrap();
        tick(&s, &a, AT + 301000, &service.run_id, &mut |p| {
            sent.push(p);
            unreachable!()
        })
        .unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            read(&s, &a, AT + 301000).unwrap()["notice"]["notification"],
            "accepted"
        );
        let p = &sent[0];
        assert!(acknowledge(&s, &a, p, true, AT + 302000).unwrap());
        assert!(!acknowledge(&s, &a, p, true, AT + 302001).unwrap());
        assert_eq!(
            read(&s, &a, AT + 302001).unwrap()["history"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn interrupted_or_failed_notice_is_not_replayed_as_success() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, false);
        send(&s, &a, "start", json!({}), AT);
        let service = Service::default();
        let pending = reserve(&s, &a, AT + 300000, &service.run_id)
            .unwrap()
            .unwrap();
        tick(&s, &a, AT + 301000, "new-process", &mut |_| {
            panic!("不得重发未确定提醒")
        })
        .unwrap();
        let d = read(&s, &a, AT + 301000).unwrap();
        assert_eq!(d["notice"]["notification"], "unknown");
        assert!(!acknowledge(&s, &a, &pending, true, AT + 301000).unwrap());
        send(&s, &a, "reset", json!({}), AT + 301000);
        send(&s, &a, "start", json!({}), AT + 301000);
        tick(&s, &a, AT + 601000, "new-process", &mut |_| {
            Err("系统拒绝".into())
        })
        .unwrap();
        assert_eq!(
            read(&s, &a, AT + 601000).unwrap()["notice"]["notification"],
            "failed"
        );
    }
    #[test]
    fn persistence_failure_rolls_back_phase_and_blocks_unreserved_side_effects() {
        let (s, a, _) = fixture();
        init(&s, &a);
        config(&s, &a, false);
        send(&s, &a, "start", json!({}), AT);
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_pomodoro BEFORE UPDATE ON app_settings WHEN substr(new.key,1,18)='pomodoro.document:' BEGIN SELECT RAISE(FAIL,'storage denied'); END;").unwrap();
        assert!(read(&s, &a, AT + 300000).is_err());
        let service = Service::default();
        assert!(tick(&s, &a, AT + 300000, &service.run_id, &mut |_| panic!(
            "未预约不得发送"
        ))
        .is_err());
    }
    #[test]
    fn current_session_and_capability_are_required_and_delete_cleans_only_own_data() {
        let (s, a, b) = fixture();
        init(&s, &a);
        let sessions = crate::widgets::Sessions::default();
        sessions.0.lock().unwrap().insert(
            "formal".into(),
            crate::widgets::Session {
                instance: a.clone(),
                package_digest: None,
                window: "float-a".into(),
                shared_cursor: None,
                preview: None,
            },
        );
        assert!(authorized(
            &s,
            &sessions,
            "other",
            "formal",
            "pomodoro.get",
            Value::Null,
            AT
        )
        .is_err());
        assert_eq!(
            authorized(
                &s,
                &sessions,
                "float-a",
                "formal",
                "pomodoro.get",
                Value::Null,
                AT
            )
            .unwrap()["initialized"],
            true
        );
        assert!(authorized(
            &s,
            &sessions,
            "float-a",
            "formal",
            "pomodoro.get",
            json!({"instance":b}),
            AT
        )
        .is_err());
        remove_data(&s.db.lock().unwrap(), &a).unwrap();
        assert_eq!(read(&s, &a, AT).unwrap()["initialized"], false);
        assert_eq!(read(&s, &b, AT).unwrap()["initialized"], false);
        s.db.lock().unwrap().execute("UPDATE widget_packages SET manifest=replace(manifest,'focus.pomodoro','storage.instance')",[]).unwrap();
        assert!(authorized(
            &s,
            &sessions,
            "float-a",
            "formal",
            "pomodoro.get",
            Value::Null,
            AT
        )
        .is_err());
    }
}
