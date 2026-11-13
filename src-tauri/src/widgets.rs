use crate::store::{err, AppResult, Store};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub host_api_major: u32,
    pub entry: String,
    pub default_size: Size,
    pub min_size: Size,
    pub capabilities: Vec<String>,
    pub settings_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_check: Option<u32>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0', '%'])
        && !path.starts_with('/')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && path.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && !s.ends_with(['.', ' '])
                && !matches!(
                    s.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
}

fn validate_manifest(m: &Manifest) -> AppResult<()> {
    if m.load_check.is_some_and(|v| v != 1) {
        return Err("VERSION_INCOMPATIBLE: 不支持的试加载握手版本".into());
    }
    if m.schema_version != 1 || m.host_api_major != 1 {
        return Err("VERSION_INCOMPATIBLE: 只支持宿主接口 v1".into());
    }
    if m.id.len() > 80
        || !m.id.contains('.')
        || !m
            .id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        || m.id.split('.').any(str::is_empty)
    {
        return Err("INVALID_ARGUMENT: 组件标识无效".into());
    }
    if m.version.split('.').count() != 3
        || !m
            .version
            .split('.')
            .all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err("INVALID_ARGUMENT: 需要三段数字版本".into());
    }
    version(&m.version).map_err(|_| "INVALID_ARGUMENT: 版本段超出支持范围".to_owned())?;
    if m.name.trim().is_empty()
        || m.name.len() > 128
        || !safe_path(&m.entry)
        || !m.entry.starts_with("dist/")
        || !m.entry.ends_with(".html")
    {
        return Err("INVALID_ARGUMENT: 名称或入口无效".into());
    }
    if m.capabilities
        .iter()
        .any(|s| !["storage.instance", "market.read", "alerts.manage"].contains(&s.as_str()))
    {
        return Err("CAPABILITY_DENIED: 未知能力".into());
    }
    if !(120..=1600).contains(&m.min_size.width)
        || !(80..=1200).contains(&m.min_size.height)
        || m.default_size.width < m.min_size.width
        || m.default_size.height < m.min_size.height
        || m.default_size.width > 1600
        || m.default_size.height > 1200
    {
        return Err("INVALID_ARGUMENT: 尺寸无效".into());
    }
    validate_schema(&m.settings_schema)?;
    Ok(())
}

fn validate_schema(schema: &Value) -> AppResult<()> {
    if schema.to_string().len() > 16 * 1024
        || schema["type"] != "object"
        || schema["additionalProperties"] != false
        || !schema["properties"].is_object()
    {
        return Err(
            "INVALID_ARGUMENT: 配置结构需要 object/properties/additionalProperties=false".into(),
        );
    }
    let allowed = ["type", "properties", "additionalProperties", "required"];
    if schema
        .as_object()
        .unwrap()
        .keys()
        .any(|k| !allowed.contains(&k.as_str()))
    {
        return Err("INVALID_ARGUMENT: 不支持的配置结构字段".into());
    }
    let properties = schema["properties"].as_object().unwrap();
    if properties.len() > 64 {
        return Err("INVALID_ARGUMENT: 配置属性最多 64 项".into());
    }
    if let Some(required) = schema.get("required") {
        let names = required
            .as_array()
            .ok_or("INVALID_ARGUMENT: required 必须是名称数组")?;
        let mut unique = HashSet::new();
        for value in names {
            let name = value
                .as_str()
                .ok_or("INVALID_ARGUMENT: required 需要字符串名称")?;
            if !properties.contains_key(name) || !unique.insert(name) {
                return Err("INVALID_ARGUMENT: required 包含未知或重复属性".into());
            }
        }
    }
    for p in properties.values() {
        if !["boolean", "string", "number", "integer"].contains(&p["type"].as_str().unwrap_or(""))
            || p.as_object().is_none_or(|o| {
                o.keys().any(|k| {
                    ![
                        "type",
                        "default",
                        "title",
                        "minimum",
                        "maximum",
                        "maxLength",
                    ]
                    .contains(&k.as_str())
                })
            })
        {
            return Err("INVALID_ARGUMENT: 只支持已公布的基本配置类型和约束".into());
        }
        let numeric = p["type"] == "number" || p["type"] == "integer";
        if p.get("title").is_some_and(|v| !v.is_string())
            || p.get("maxLength")
                .is_some_and(|v| p["type"] != "string" || v.as_u64().is_none())
            || ["minimum", "maximum"]
                .iter()
                .any(|key| p.get(key).is_some_and(|v| !numeric || !v.is_number()))
            || p["minimum"]
                .as_f64()
                .zip(p["maximum"].as_f64())
                .is_some_and(|(min, max)| min > max)
        {
            return Err("INVALID_ARGUMENT: 配置约束的类型或范围无效".into());
        }
    }
    // 默认值由宿主显式生成；本契约要求新实例立即具有有效配置。
    validate_settings(schema, &default_settings(schema))
}

fn default_settings(schema: &Value) -> Value {
    Value::Object(
        schema["properties"]
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(name, property)| {
                property
                    .get("default")
                    .map(|value| (name.clone(), value.clone()))
            })
            .collect(),
    )
}
pub fn initial_settings(manifest: &Manifest) -> Value {
    default_settings(&manifest.settings_schema)
}

pub fn validate_settings(schema: &Value, settings: &Value) -> AppResult<()> {
    if settings.to_string().len() > 16 * 1024 {
        return Err("INVALID_ARGUMENT: 配置超过 16 KiB".into());
    }
    let settings = settings
        .as_object()
        .ok_or("INVALID_ARGUMENT: 配置应为对象")?;
    let props = schema["properties"]
        .as_object()
        .ok_or("INVALID_ARGUMENT: 配置结构无效")?;
    for (k, v) in settings {
        let p = props.get(k).ok_or("INVALID_ARGUMENT: 未知配置项")?;
        let valid = match p["type"].as_str().unwrap_or("") {
            "boolean" => v.is_boolean(),
            "string" => v.as_str().is_some_and(|s| {
                s.chars().count() <= p["maxLength"].as_u64().unwrap_or(256) as usize
            }),
            "number" => v.is_number(),
            // 普通配置跨 JavaScript 消息桥传输，限制在其安全整数范围内；不用于价格。
            "integer" => v.as_f64().is_some_and(|n| {
                n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
            }),
            _ => false,
        };
        if !valid
            || v.as_f64().is_some_and(|n| {
                p["minimum"].as_f64().is_some_and(|min| n < min)
                    || p["maximum"].as_f64().is_some_and(|max| n > max)
            })
        {
            return Err(format!("INVALID_ARGUMENT: 配置项 {k} 无效"));
        }
    }
    if schema["required"].as_array().is_some_and(|a| {
        a.iter()
            .any(|k| k.as_str().is_none_or(|k| !settings.contains_key(k)))
    }) {
        return Err("INVALID_ARGUMENT: 缺少必要配置".into());
    }
    Ok(())
}

#[derive(Clone)]
pub struct StagedPackage {
    pub manifest: Manifest,
    pub digest: String,
    pub directory: PathBuf,
    previous_digest: Option<String>,
    pub instances: Vec<(String, String, String)>,
}

fn version(value: &str) -> AppResult<Vec<u64>> {
    value
        .split('.')
        .map(|s| s.parse::<u64>().map_err(err))
        .collect()
}

pub fn compatible_settings(schema: &Value, old: &str) -> AppResult<Value> {
    let mut settings: Value = serde_json::from_str(old).map_err(err)?;
    let object = settings
        .as_object_mut()
        .ok_or("原配置不是对象，拒绝自动升级")?;
    for (name, value) in default_settings(schema).as_object().unwrap() {
        object.entry(name).or_insert_with(|| value.clone());
    }
    validate_settings(schema, &settings)
        .map_err(|e| format!("VERSION_INCOMPATIBLE: 旧配置无法兼容新版本，原版已保留：{e}"))?;
    Ok(settings)
}
impl StagedPackage {
    pub fn unchanged(&self) -> bool {
        self.previous_digest.as_deref() == Some(self.digest.as_str())
    }
}

fn static_resource(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "html"
                    | "css"
                    | "js"
                    | "mjs"
                    | "json"
                    | "svg"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "avif"
                    | "ico"
                    | "woff"
                    | "woff2"
                    | "ttf"
                    | "otf"
                    | "txt"
                    | "map"
            )
        })
}

fn monitor_html(bytes: Vec<u8>, source: &str) -> AppResult<Vec<u8>> {
    let mut html = String::from_utf8(bytes).map_err(err)?;
    let start = html
        .find(|c: char| !c.is_whitespace() && c != '\u{feff}')
        .unwrap_or(0);
    let insert = if html[start..].to_ascii_lowercase().starts_with("<!doctype") {
        html[start..]
            .find('>')
            .map(|i| start + i + 1)
            .ok_or("HTML文档声明不完整")?
    } else {
        0
    };
    html.insert_str(insert, &format!("<script src=\"{source}\"></script>"));
    Ok(html.into_bytes())
}

pub fn stage(store: &Store, path: &Path) -> AppResult<StagedPackage> {
    if std::fs::metadata(path).map_err(err)?.len() > 20 * 1024 * 1024 {
        return Err("INVALID_ARGUMENT: 压缩包超过 20 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(err)?;
    stage_bytes(store, bytes)
}

pub fn stage_bytes(store: &Store, bytes: Vec<u8>) -> AppResult<StagedPackage> {
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("INVALID_ARGUMENT: 压缩包超过 20 MiB".into());
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
    if archive.len() > 256 {
        return Err("INVALID_ARGUMENT: 文件项超过 256".into());
    }
    let mut files = HashMap::new();
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(err)?;
        let name = f.name().to_owned();
        if !safe_path(name.trim_end_matches('/'))
            || f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
            || !seen.insert(name.to_ascii_lowercase())
        {
            return Err("INVALID_ARGUMENT: 越界、链接或重名路径".into());
        }
        total = total
            .checked_add(f.size())
            .ok_or("INVALID_ARGUMENT: 展开大小溢出")?;
        if total > 50 * 1024 * 1024 {
            return Err("INVALID_ARGUMENT: 展开后超过 50 MiB".into());
        }
        if !f.is_dir() {
            if name != "manifest.json" && (!name.starts_with("dist/") || !static_resource(&name)) {
                return Err("INVALID_ARGUMENT: 只接受 manifest.json 和 dist/".into());
            }
            let limit = f.size() + 1;
            let mut data = Vec::new();
            (&mut f).take(limit).read_to_end(&mut data).map_err(err)?;
            if data.len() as u64 >= limit {
                return Err("INVALID_ARGUMENT: 文件长度不符".into());
            }
            files.insert(name, data);
        }
    }
    let m: Manifest = serde_json::from_slice(
        files
            .get("manifest.json")
            .ok_or("INVALID_ARGUMENT: 缺少清单")?,
    )
    .map_err(err)?;
    validate_manifest(&m)?;
    if !files.contains_key(&m.entry) {
        return Err("INVALID_ARGUMENT: 缺少入口文件".into());
    }
    if files.contains_key("dist/__souldesk_runtime.js") {
        return Err("INVALID_ARGUMENT: 使用了宿主保留资源名".into());
    }
    let db = store.db.lock().map_err(err)?;
    let previous: Option<(String, String, String)> = db
        .query_row(
            "SELECT digest,manifest,directory FROM widget_packages WHERE id=?1",
            [&m.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((old_digest, old_manifest, old_directory)) = &previous {
        if old_digest == &digest {
            return Ok(StagedPackage {
                manifest: m,
                digest,
                directory: old_directory.into(),
                previous_digest: Some(old_digest.clone()),
                instances: Vec::new(),
            });
        }
        let old: Manifest = serde_json::from_str(old_manifest).map_err(err)?;
        if version(&m.version)? <= version(&old.version)? {
            return Err(
                "VERSION_INCOMPATIBLE: 只接受更高版本；相同版本的不同内容或降级不会覆盖原包".into(),
            );
        }
        let mut query = db
            .prepare("SELECT settings FROM widget_instances WHERE package_id=?1")
            .map_err(err)?;
        for settings in query
            .query_map([&m.id], |r| r.get::<_, String>(0))
            .map_err(err)?
        {
            compatible_settings(&m.settings_schema, &settings.map_err(err)?)?;
        }
    }
    let instances = {
        let mut query = db
            .prepare(
                "SELECT id,settings,storage FROM widget_instances WHERE package_id=?1 ORDER BY id",
            )
            .map_err(err)?;
        let rows = query
            .query_map([&m.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)?
    };
    drop(db);
    // 不可变目录：失败只留下未登记目录，不覆盖旧包或清理用户文件。
    let directory =
        store
            .root
            .join("widgets")
            .join(&m.id)
            .join(format!("{}-{}", m.version, Uuid::new_v4()));
    std::fs::create_dir_all(&directory).map_err(err)?;
    for (name, data) in files {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        std::fs::write(path, data).map_err(err)?;
    }
    store.db.lock().map_err(err)?.execute(
        "INSERT OR IGNORE INTO widget_versions(package_id,digest,manifest,directory,created) VALUES(?1,?2,?3,?4,?5)",
        params![m.id,digest,serde_json::to_string(&m).map_err(err)?,directory.to_str().ok_or("路径编码无效")?,crate::store::now_ms()]).map_err(err)?;
    Ok(StagedPackage {
        manifest: m,
        digest,
        directory,
        previous_digest: previous.map(|p| p.0),
        instances,
    })
}

pub fn activate(store: &Store, staged: &StagedPackage) -> AppResult<Manifest> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let current: Option<String> = tx
        .query_row(
            "SELECT digest FROM widget_packages WHERE id=?1",
            [&staged.manifest.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if current != staged.previous_digest {
        return Err("CONFLICT: 组件版本已经变化，请重新导入".into());
    }
    if staged.previous_digest.as_deref() == Some(staged.digest.as_str()) {
        return Ok(staged.manifest.clone());
    }
    let current_instances = {
        let mut query = tx
            .prepare(
                "SELECT id,settings,storage FROM widget_instances WHERE package_id=?1 ORDER BY id",
            )
            .map_err(err)?;
        let rows = query
            .query_map([&staged.manifest.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)?
    };
    if current_instances != staged.instances {
        return Err("CONFLICT: 试加载期间实例或配置已变化，请重新导入".into());
    }
    if let Some(previous) = &staged.previous_digest {
        for (id, settings, storage) in &current_instances {
            tx.execute("INSERT INTO widget_upgrade_snapshots(package_id,from_digest,to_digest,instance_id,settings,storage,created) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![staged.manifest.id,previous,staged.digest,id,settings,storage,crate::store::now_ms()]).map_err(err)?;
        }
    }
    let updated = {
        let mut query = tx
            .prepare("SELECT id,settings FROM widget_instances WHERE package_id=?1")
            .map_err(err)?;
        let rows = query
            .query_map([&staged.manifest.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?;
        let mut values = Vec::new();
        for row in rows {
            let (id, settings) = row.map_err(err)?;
            values.push((
                id,
                compatible_settings(&staged.manifest.settings_schema, &settings)?,
            ));
        }
        values
    };
    for (id, settings) in updated {
        tx.execute(
            "UPDATE widget_instances SET settings=?1 WHERE id=?2",
            params![settings.to_string(), id],
        )
        .map_err(err)?;
    }
    tx.execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET manifest=excluded.manifest,directory=excluded.directory,digest=excluded.digest",
        params![staged.manifest.id,serde_json::to_string(&staged.manifest).map_err(err)?,staged.directory.to_str().ok_or("路径编码无效")?,staged.digest]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(staged.manifest.clone())
}

pub fn activate_sessions(
    store: &Store,
    sessions: &Sessions,
    staged: &StagedPackage,
) -> AppResult<Manifest> {
    let mut sessions = sessions.0.lock().map_err(err)?;
    let manifest = activate(store, staged)?;
    if staged.previous_digest.as_deref() != Some(staged.digest.as_str()) {
        sessions.retain(|_, s| !staged.instances.iter().any(|i| i.0 == s.instance));
    }
    Ok(manifest)
}

// 核心回归用；桌面入口必须经 packages::install 的真实受限试加载。
#[cfg(test)]
pub fn import(store: &Store, path: &Path) -> AppResult<Manifest> {
    activate(store, &stage(store, path)?)
}

pub fn create_instance(store: &Store, package_id: &str) -> AppResult<String> {
    let db = store.db.lock().map_err(err)?;
    let manifest: String = db
        .query_row(
            "SELECT manifest FROM widget_packages WHERE id=?1",
            [package_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let manifest: Manifest = serde_json::from_str(&manifest).map_err(err)?;
    let settings = default_settings(&manifest.settings_schema);
    validate_settings(&manifest.settings_schema, &settings)?;
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO widget_instances(id,package_id,settings,position) VALUES(?1,?2,?3,(SELECT COALESCE(MAX(position),-1)+1 FROM widget_instances))",
        params![id, package_id, settings.to_string()],
    )
    .map_err(err)?;
    Ok(id)
}

pub fn set_instance_size(store: &Store, instance: &str, width: u32, height: u32) -> AppResult<()> {
    let db = store.db.lock().map_err(err)?;
    let manifest:String=db.query_row("SELECT p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[instance],|r|r.get(0)).map_err(err)?;
    let m: Manifest = serde_json::from_str(&manifest).map_err(err)?;
    if width < m.min_size.width || height < m.min_size.height || width > 1600 || height > 1200 {
        return Err("INVALID_ARGUMENT: 实例尺寸不符合组件范围".into());
    }
    db.execute(
        "UPDATE widget_instances SET width=?1,height=?2 WHERE id=?3",
        params![width, height, instance],
    )
    .map_err(err)?;
    Ok(())
}

pub fn reorder_instances(store: &Store, ids: &[String]) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let all = {
        let mut query = tx.prepare("SELECT id FROM widget_instances").map_err(err)?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?;
        rows.collect::<Result<HashSet<_>, _>>().map_err(err)?
    };
    if ids.len() != all.len()
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        || ids.iter().any(|id| !all.contains(id))
    {
        return Err("CONFLICT: 实例列表已变化，请刷新后排序".into());
    }
    for (position, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE widget_instances SET position=?1 WHERE id=?2",
            params![position, id],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)
}

pub fn remove_instance(store: &Store, sessions: &Sessions, instance: &str) -> AppResult<()> {
    let mut sessions = sessions.0.lock().map_err(err)?;
    if store
        .db
        .lock()
        .map_err(err)?
        .execute("DELETE FROM widget_instances WHERE id=?1", [instance])
        .map_err(err)?
        != 1
    {
        return Err("INVALID_ARGUMENT: 实例不存在".into());
    }
    sessions.retain(|_, s| s.instance != instance);
    Ok(())
}

pub fn unregister_package(store: &Store, package: &str) -> AppResult<()> {
    // 外键阻止仍被实例使用的包取消登记；不删除包目录或历史版本。
    if store
        .db
        .lock()
        .map_err(err)?
        .execute("DELETE FROM widget_packages WHERE id=?1", [package])
        .map_err(err)?
        != 1
    {
        return Err("INVALID_ARGUMENT: 组件不存在".into());
    }
    Ok(())
}

#[derive(Clone)]
pub struct Session {
    pub instance: String,
    pub window: String,
    pub preview: Option<Arc<Mutex<Preview>>>,
}
pub struct Preview {
    pub package: StagedPackage,
    pub settings: Value,
    pub storage: Value,
    pub ready_since: Option<Instant>,
    pub failure: Option<String>,
}
#[derive(Default)]
pub struct Sessions(pub Mutex<HashMap<String, Session>>);

pub fn set_floating(
    store: &Store,
    sessions: &Sessions,
    instance: &str,
    floating: bool,
) -> AppResult<bool> {
    let mut sessions = sessions.0.lock().map_err(err)?;
    let db = store.db.lock().map_err(err)?;
    let previous: bool = db
        .query_row(
            "SELECT floating FROM widget_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(err)?;
    db.execute(
        "UPDATE widget_instances SET floating=?1 WHERE id=?2",
        params![floating, instance],
    )
    .map_err(err)?;
    sessions.retain(|_, s| s.instance != instance);
    Ok(previous)
}

#[cfg(test)]
pub fn open_session(
    store: &Store,
    sessions: &Sessions,
    window: &str,
    instance: &str,
) -> AppResult<Value> {
    open_session_expected(store, sessions, window, instance, None)
}
pub fn open_session_expected(
    store: &Store,
    sessions: &Sessions,
    window: &str,
    instance: &str,
    expected_digest: Option<&str>,
) -> AppResult<Value> {
    open_session_inner(store, sessions, window, instance, expected_digest, false)
}

/// 正式宿主角色直接校验数据库归属；main 管理窗口没有展示权限。
pub fn open_display_session(
    store: &Store,
    sessions: &Sessions,
    window: &str,
    instance: &str,
    expected_digest: Option<&str>,
) -> AppResult<Value> {
    open_session_inner(store, sessions, window, instance, expected_digest, true)
}
fn open_session_inner(
    store: &Store,
    sessions: &Sessions,
    window: &str,
    instance: &str,
    expected_digest: Option<&str>,
    display: bool,
) -> AppResult<Value> {
    let mut sessions = sessions.0.lock().map_err(err)?;
    let db = store.db.lock().map_err(err)?;
    let (package, settings, floating): (String, String, bool) = db
        .query_row(
            "SELECT package_id,settings,floating FROM widget_instances WHERE id=?1",
            [instance],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(err)?;
    let detail = window == format!("detail-{instance}");
    if display {
        let (location, visible): (String, bool) = db
            .query_row(
                "SELECT location,visible FROM widget_instances WHERE id=?1",
                [instance],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let hidden = db
            .query_row(
                "SELECT value FROM app_settings WHERE key='hidden_all'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_default()
            == "true";
        if hidden
            || !visible
            || !(detail
                || window == "panel" && location == "panel"
                || window == format!("float-{instance}") && location == "desktop")
        {
            return Err("SESSION_EXPIRED: 当前窗口不是可见实例的展示所有者".into());
        }
    } else if !(window == "main" && !floating || window == format!("float-{instance}") && floating)
    {
        return Err("SESSION_EXPIRED: 实例已移动".into());
    }
    let manifest: String = db
        .query_row(
            "SELECT manifest FROM widget_packages WHERE id=?1",
            [&package],
            |r| r.get(0),
        )
        .map_err(err)?;
    let manifest: Manifest = serde_json::from_str(&manifest).map_err(err)?;
    let digest: String = db
        .query_row(
            "SELECT digest FROM widget_packages WHERE id=?1",
            [&package],
            |r| r.get(0),
        )
        .map_err(err)?;
    if expected_digest.is_some_and(|expected| expected != digest) {
        return Err("VERSION_INCOMPATIBLE: 组件版本已更新，请重新加载".into());
    }
    if display && detail && !manifest.capabilities.iter().any(|c| c == "alerts.manage") {
        return Err("CAPABILITY_DENIED: 此组件未声明详情能力".into());
    }
    let token = Uuid::new_v4().to_string();
    sessions.retain(|_, s| {
        s.instance != instance || display && (s.window.starts_with("detail-") != detail)
    });
    sessions.insert(
        token.clone(),
        Session {
            instance: instance.into(),
            window: window.into(),
            preview: None,
        },
    );
    Ok(
        json!({"token":token,"instanceId":instance,"packageId":package,"digest":digest,"entry":manifest.entry,"resourcePath":format!("{package}/{digest}/{}",manifest.entry),"version":manifest.version,"role":if detail {"details"} else {"primary"},"settings":serde_json::from_str::<Value>(&settings).map_err(err)?}),
    )
}

pub fn open_preview(
    sessions: &Sessions,
    window: &str,
    instance: &str,
    preview: Arc<Mutex<Preview>>,
) -> AppResult<Value> {
    if window != instance {
        return Err("CAPABILITY_DENIED: 试加载只能使用对应隐藏容器".into());
    }
    let mut sessions = sessions.0.lock().map_err(err)?;
    let value = preview.lock().map_err(err)?;
    let token = Uuid::new_v4().to_string();
    let m = &value.package.manifest;
    let result = json!({"token":token,"instanceId":instance,"packageId":m.id,"entry":m.entry,"digest":value.package.digest,"resourcePath":format!("{}/{}/{}",m.id,value.package.digest,m.entry),"version":m.version,"preview":true});
    drop(value);
    sessions.retain(|_, s| s.instance != instance);
    sessions.insert(
        token,
        Session {
            instance: instance.into(),
            window: window.into(),
            preview: Some(preview),
        },
    );
    Ok(result)
}

pub fn gateway(
    store: &Store,
    sessions: &Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let sessions = sessions.0.lock().map_err(err)?;
    let session = sessions
        .get(token)
        .filter(|s| s.window == window)
        .ok_or("SESSION_EXPIRED: 请重新加载组件")?;
    if let Some(preview) = &session.preview {
        let mut preview = preview.lock().map_err(err)?;
        if preview.failure.is_some() {
            return Err("DATA_UNAVAILABLE: 试加载已失败".into());
        }
        if method != "runtime.ready"
            && !preview
                .package
                .manifest
                .capabilities
                .iter()
                .any(|c| c == "storage.instance")
        {
            return Err("CAPABILITY_DENIED: 未声明实例存储".into());
        }
        let result = match method {
            "runtime.ready" => Ok(Value::Null),
            "settings.get" => Ok(preview.settings.clone()),
            "settings.update" => {
                validate_settings(&preview.package.manifest.settings_schema, &args)?;
                preview.settings = args.clone();
                Ok(args)
            }
            "storage.get" => Ok(preview.storage.clone()),
            "storage.set" if args.is_object() && args.to_string().len() <= 16 * 1024 => {
                preview.storage = args.clone();
                Ok(args)
            }
            _ => Err("CAPABILITY_DENIED: 试加载仅允许隔离的实例配置及存储".into()),
        };
        if result.is_ok()
            && preview.ready_since.is_none()
            && (method == "runtime.ready" || preview.package.manifest.load_check.is_none())
        {
            preview.ready_since = Some(Instant::now());
        }
        return result;
    }
    let db = store.db.lock().map_err(err)?;
    let (manifest,settings,storage):(String,String,String)=db.query_row("SELECT p.manifest,i.settings,i.storage FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[&session.instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)?;
    let m: Manifest = serde_json::from_str(&manifest).map_err(err)?;
    if method == "runtime.ready" {
        return Ok(Value::Null);
    }
    if !m.capabilities.iter().any(|c| c == "storage.instance") {
        return Err("CAPABILITY_DENIED: 未声明实例存储".into());
    }
    match method {
        "settings.get" => serde_json::from_str(&settings).map_err(err),
        "settings.update" => {
            validate_settings(&m.settings_schema, &args)?;
            db.execute(
                "UPDATE widget_instances SET settings=?1 WHERE id=?2",
                params![args.to_string(), session.instance],
            )
            .map_err(err)?;
            Ok(args)
        }
        "storage.get" => serde_json::from_str(&storage).map_err(err),
        "storage.set" => {
            if !args.is_object() || args.to_string().len() > 16 * 1024 {
                return Err("INVALID_ARGUMENT: 存储需要 16 KiB 内对象".into());
            }
            db.execute(
                "UPDATE widget_instances SET storage=?1 WHERE id=?2",
                params![args.to_string(), session.instance],
            )
            .map_err(err)?;
            Ok(args)
        }
        _ => Err("CAPABILITY_DENIED: 未知或未开放的方法".into()),
    }
}

pub fn resource(store: &Store, path: &str) -> AppResult<(Vec<u8>, String)> {
    let path = percent_encoding::percent_decode_str(path.trim_start_matches('/'))
        .decode_utf8()
        .map_err(err)?;
    if !safe_path(&path) {
        return Err("无效资源路径".into());
    }
    let (id, remaining) = path.split_once('/').ok_or("缺少组件标识")?;
    let (digest, relative) = if remaining.starts_with("dist/") {
        (None, remaining)
    } else {
        let (digest, relative) = remaining.split_once('/').ok_or("缺少组件版本")?;
        if digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("组件版本无效".into());
        }
        (Some(digest), relative)
    };
    if !relative.starts_with("dist/") || !static_resource(relative) {
        return Err("只允许页面资源".into());
    }
    let db = store.db.lock().map_err(err)?;
    let directory: String = if let Some(digest) = digest {
        db.query_row(
            "SELECT directory FROM widget_versions WHERE package_id=?1 AND digest=?2",
            params![id, digest],
            |r| r.get(0),
        )
    } else {
        db.query_row(
            "SELECT directory FROM widget_packages WHERE id=?1",
            [id],
            |r| r.get(0),
        )
    }
    .map_err(err)?;
    drop(db);
    if relative == "dist/__souldesk_runtime.js" {
        return Ok((br#"(()=>{const hostOrigin=new URLSearchParams(location.search).get('hostOrigin');const report=message=>{if(hostOrigin)parent.postMessage({type:'souldesk:failed',protocol:1,message:String(message).slice(0,1000)},hostOrigin);};window.addEventListener('error',e=>report(e.message||'component resource failed'),true);window.addEventListener('unhandledrejection',e=>report(e.reason?.message||e.reason));})();"#.to_vec(),"text/javascript".into()));
    }
    let root = PathBuf::from(directory).canonicalize().map_err(err)?;
    let file = root.join(relative).canonicalize().map_err(err)?;
    if !file.starts_with(&root)
        || !file.is_file()
        || std::fs::metadata(&file).map_err(err)?.len() > 20 * 1024 * 1024
    {
        return Err("资源不可访问".into());
    }
    let mut bytes = std::fs::read(&file).map_err(err)?;
    if relative.ends_with(".html") {
        let prefix = if let Some(digest) = digest {
            format!("/{id}/{digest}")
        } else {
            format!("/{id}")
        };
        bytes = monitor_html(bytes, &format!("{prefix}/dist/__souldesk_runtime.js"))?;
    }
    Ok((
        bytes,
        mime_guess::from_path(file)
            .first_or_octet_stream()
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn formal_primary_and_detail_sessions_are_isolated_and_manager_cannot_mount() {
        let root =
            std::env::temp_dir().join(format!("souldesk-display-session-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let sessions = Sessions::default();
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        manifest["capabilities"] = json!(["storage.instance", "alerts.manage"]);
        import(&store, &fixture(&root, &manifest, None)).unwrap();
        let id = create_instance(&store, manifest["id"].as_str().unwrap()).unwrap();
        let label = format!("float-{id}");
        let detail = format!("detail-{id}");
        assert!(open_display_session(&store, &sessions, "main", &id, None).is_err());
        assert!(open_display_session(&store, &sessions, "panel", &id, None).is_err());
        let primary = open_display_session(&store, &sessions, &label, &id, None).unwrap();
        let extra = open_display_session(&store, &sessions, &detail, &id, None).unwrap();
        assert_eq!(extra["role"], "details");
        assert_eq!(sessions.0.lock().unwrap().len(), 2);
        assert!(gateway(
            &store,
            &sessions,
            &label,
            primary["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_ok());
        assert!(gateway(
            &store,
            &sessions,
            &detail,
            primary["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_err());
        store.presentation(&id, "panel", true, false).unwrap();
        assert!(open_display_session(&store, &sessions, &label, &id, None).is_err());
        // 正式协调器在转移之前撤销旧会话；新握手只替换同角色，详情不挤掉主展示。
        let panel = open_display_session(&store, &sessions, "panel", &id, None).unwrap();
        assert!(gateway(
            &store,
            &sessions,
            &label,
            primary["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_err());
        assert!(gateway(
            &store,
            &sessions,
            "panel",
            panel["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_ok());
        store.save_preference("hidden_all", "true").unwrap();
        assert!(open_display_session(&store, &sessions, "panel", &id, None).is_err());
    }
    #[test]
    fn upgrade_rejects_old_page_handshake_and_concurrent_configuration_changes() {
        let root = std::env::temp_dir().join(format!("souldesk-upgrade-race-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        import(&store, &fixture(&root, &manifest, None)).unwrap();
        let id = create_instance(&store, "local.example.clock").unwrap();
        let sessions = Sessions::default();
        let old = open_session(&store, &sessions, "main", &id).unwrap();
        manifest["version"] = json!("0.2.0");
        let staged = stage(&store, &fixture(&root, &manifest, None)).unwrap();
        gateway(
            &store,
            &sessions,
            "main",
            old["token"].as_str().unwrap(),
            "storage.set",
            json!({"edited":"试加载期间的修改"}),
        )
        .unwrap();
        assert!(activate_sessions(&store, &sessions, &staged).is_err());
        assert!(gateway(
            &store,
            &sessions,
            "main",
            old["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_ok());
        let staged = stage(&store, &fixture(&root, &manifest, None)).unwrap();
        activate_sessions(&store, &sessions, &staged).unwrap();
        assert!(
            open_session_expected(&store, &sessions, "main", &id, old["digest"].as_str()).is_err()
        );
        assert!(gateway(
            &store,
            &sessions,
            "main",
            old["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_err());
        assert_eq!(
            store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT storage FROM widget_upgrade_snapshots WHERE instance_id=?1",
                    [&id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            json!({"edited":"试加载期间的修改"}).to_string()
        );
    }

    #[test]
    fn static_resource_rules_and_html_monitor_preserve_document_mode() {
        let html = monitor_html(
            b"<!doctype html><html><head><title>test</title></head></html>".to_vec(),
            "/widget/dist/monitor.js",
        )
        .unwrap();
        assert!(String::from_utf8(html)
            .unwrap()
            .starts_with("<!doctype html><script"));
        let root = std::env::temp_dir().join(format!("souldesk-static-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let manifest: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        assert!(stage(&store, &fixture(&root, &manifest, Some("dist/backend.dll"))).is_err());
        let staged = stage(&store, &fixture(&root, &manifest, None)).unwrap();
        activate(&store, &staged).unwrap();
        std::fs::write(
            staged.directory.join("dist/legacy.exe"),
            "旧包中的非页面文件",
        )
        .unwrap();
        assert!(resource(&store, "local.example.clock/dist/legacy.exe").is_err());
        let preview = Arc::new(Mutex::new(Preview {
            settings: initial_settings(&staged.manifest),
            package: staged,
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let sessions = Sessions::default();
        let session =
            open_preview(&sessions, "trial-ready", "trial-ready", preview.clone()).unwrap();
        gateway(
            &store,
            &sessions,
            "trial-ready",
            session["token"].as_str().unwrap(),
            "settings.get",
            Value::Null,
        )
        .unwrap();
        assert!(preview.lock().unwrap().ready_since.is_none());
        gateway(
            &store,
            &sessions,
            "trial-ready",
            session["token"].as_str().unwrap(),
            "runtime.ready",
            Value::Null,
        )
        .unwrap();
        assert!(preview.lock().unwrap().ready_since.is_some());
    }
    #[test]
    fn identical_package_reimport_keeps_live_sessions_and_preview_obeys_capabilities() {
        let root = std::env::temp_dir().join(format!("souldesk-reimport-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let manifest: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        let path = fixture(&root, &manifest, None);
        import(&store, &path).unwrap();
        let id = create_instance(&store, "local.example.clock").unwrap();
        let sessions = Sessions::default();
        let session = open_session(&store, &sessions, "main", &id).unwrap();
        let staged = stage(&store, &path).unwrap();
        activate_sessions(&store, &sessions, &staged).unwrap();
        assert!(gateway(
            &store,
            &sessions,
            "main",
            session["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_ok());
        let mut unprivileged = staged;
        unprivileged.manifest.capabilities.clear();
        let preview = Arc::new(Mutex::new(Preview {
            settings: initial_settings(&unprivileged.manifest),
            package: unprivileged,
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let session = open_preview(&sessions, "trial-cap", "trial-cap", preview).unwrap();
        assert!(gateway(
            &store,
            &sessions,
            "trial-cap",
            session["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_err());
    }

    #[test]
    fn instance_layout_is_validated_and_removal_only_invalidates_its_own_session() {
        let root = std::env::temp_dir().join(format!("souldesk-layout-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let manifest: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        import(&store, &fixture(&root, &manifest, None)).unwrap();
        let a = create_instance(&store, "local.example.clock").unwrap();
        let b = create_instance(&store, "local.example.clock").unwrap();
        let sessions = Sessions::default();
        let sa = open_session(&store, &sessions, "main", &a).unwrap();
        let sb = open_session(&store, &sessions, "main", &b).unwrap();
        set_instance_size(&store, &a, 400, 300).unwrap();
        assert!(set_instance_size(&store, &a, 1, 1).is_err());
        reorder_instances(&store, &[b.clone(), a.clone()]).unwrap();
        assert!(reorder_instances(&store, &[a.clone(), a.clone()]).is_err());
        assert_eq!(store.snapshot().unwrap()["instances"][0]["id"], b);
        remove_instance(&store, &sessions, &a).unwrap();
        assert!(gateway(
            &store,
            &sessions,
            "main",
            sa["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_err());
        assert!(gateway(
            &store,
            &sessions,
            "main",
            sb["token"].as_str().unwrap(),
            "settings.get",
            Value::Null
        )
        .is_ok());
        assert!(unregister_package(&store, "local.example.clock").is_err());
        remove_instance(&store, &sessions, &b).unwrap();
        unregister_package(&store, "local.example.clock").unwrap();
        assert!(store.snapshot().unwrap()["packages"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM widget_versions", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }

    fn fixture(root: &Path, manifest: &Value, extra: Option<&str>) -> PathBuf {
        let path = root.join(format!("{}.zip", Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file("dist/index.html", options).unwrap();
        zip.write_all(b"<!doctype html><title>test</title>")
            .unwrap();
        if let Some(extra) = extra {
            zip.start_file(extra, options).unwrap();
            zip.write_all(b"bad").unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn independent_instances_sessions_and_invalid_upgrade() {
        let root = std::env::temp_dir().join(format!("souldesk-g0-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let manifest: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        let package = fixture(&root, &manifest, None);
        import(&store, &package).unwrap();
        let a = create_instance(&store, "local.example.clock").unwrap();
        let b = create_instance(&store, "local.example.clock").unwrap();
        let sessions = Sessions::default();
        let sa = open_session(&store, &sessions, "main", &a).unwrap();
        let sb = open_session(&store, &sessions, "main", &b).unwrap();
        let token = sa["token"].as_str().unwrap();
        gateway(
            &store,
            &sessions,
            "main",
            token,
            "settings.update",
            json!({"title":"实例甲","showSeconds":false}),
        )
        .unwrap();
        let value = gateway(
            &store,
            &sessions,
            "main",
            sb["token"].as_str().unwrap(),
            "settings.get",
            Value::Null,
        )
        .unwrap();
        assert_eq!(value["title"], "此刻");
        assert!(gateway(
            &store,
            &sessions,
            "other",
            token,
            "settings.get",
            Value::Null
        )
        .is_err());
        assert!(gateway(&store, &sessions, "main", token, "shell.exec", Value::Null).is_err());
        assert!(gateway(
            &store,
            &sessions,
            "main",
            token,
            "settings.update",
            json!({"instanceId":b})
        )
        .is_err());
        let fresh = open_session(&store, &sessions, "main", &a).unwrap();
        assert!(gateway(
            &store,
            &sessions,
            "main",
            token,
            "settings.get",
            Value::Null
        )
        .is_err());
        assert_eq!(
            gateway(
                &store,
                &sessions,
                "main",
                fresh["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .unwrap()["title"],
            "实例甲"
        );
        let token = fresh["token"].as_str().unwrap();
        assert!(!set_floating(&store, &sessions, &a, true).unwrap());
        assert!(gateway(
            &store,
            &sessions,
            "main",
            token,
            "settings.get",
            Value::Null
        )
        .is_err());
        assert!(open_session(&store, &sessions, "main", &a).is_err());
        let float_session = open_session(&store, &sessions, &format!("float-{a}"), &a).unwrap();
        assert_eq!(
            gateway(
                &store,
                &sessions,
                &format!("float-{a}"),
                float_session["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .unwrap()["title"],
            "实例甲"
        );
        let bad = fixture(&root, &manifest, Some("../escape.txt"));
        assert!(import(&store, &bad).is_err());
        let mut incompatible = manifest.clone();
        incompatible["hostApiMajor"] = json!(2);
        assert!(import(&store, &fixture(&root, &incompatible, None)).is_err());
        assert!(resource(&store, "local.example.clock/dist/index.html").is_ok());
        assert!(resource(&store, "local.example.clock/dist/%2e%2e/manifest.json").is_err());
        drop(store);
        let reopened = Store::open(root).unwrap();
        assert_eq!(
            reopened.snapshot().unwrap()["instances"][0]["floating"],
            true
        );
        assert_eq!(
            reopened.snapshot().unwrap()["instances"][0]["settings"]["title"],
            "实例甲"
        );
    }
    #[test]
    fn paths_reject_windows_escape() {
        for p in [
            "../x",
            "dist/../x",
            "C:/a",
            "dist\\a",
            "dist/CON.txt",
            "dist/a.",
            "dist/%2e%2e/x",
            "/x",
        ] {
            assert!(!safe_path(p), "{p}");
        }
        assert!(safe_path("dist/assets/app.js"));
    }
    #[test]
    fn configuration_boundaries() {
        let s = json!({"properties":{"name":{"type":"string","maxLength":3},"seconds":{"type":"boolean"}},"additionalProperties":false,"type":"object"});
        assert!(validate_settings(&s, &json!({"name":"时钟","seconds":true})).is_ok());
        assert!(validate_settings(&s, &json!({"name":"1234"})).is_err());
        assert!(validate_settings(&s, &json!({"other":true})).is_err());
        assert!(validate_settings(&s, &json!({"seconds":1})).is_err());
    }

    #[test]
    fn malformed_configuration_contract_is_rejected_before_registration() {
        let baseline: Manifest =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        for schema in [
            json!({"type":"object","additionalProperties":false,"properties":{"x":{"type":"boolean","default":"yes"}}}),
            json!({"type":"object","additionalProperties":false,"properties":{"x":{"type":"string","maxLength":"3"}}}),
            json!({"type":"object","additionalProperties":false,"properties":{"x":{"type":"number","minimum":10,"maximum":2}}}),
            json!({"type":"object","additionalProperties":false,"properties":{"x":{"type":"number","minimum":"zero"}}}),
            json!({"type":"object","additionalProperties":false,"properties":{},"required":["missing"]}),
            json!({"type":"object","additionalProperties":false,"properties":{},"required":"name"}),
            json!({"type":"object","additionalProperties":false,"properties":{"x":{"type":"string"}},"required":["x"]}),
        ] {
            let mut manifest = baseline.clone();
            manifest.settings_schema = schema.clone();
            assert!(
                validate_manifest(&manifest).is_err(),
                "不应安装无法产生有效初始配置的清单：{schema}"
            );
        }
    }

    #[test]
    fn integer_settings_follow_numeric_semantics_and_js_safe_range() {
        let schema = json!({"type":"object","additionalProperties":false,"properties":{"count":{"type":"integer"}}});
        assert!(validate_settings(&schema, &json!({"count":1.0})).is_ok());
        assert!(validate_settings(&schema, &json!({"count":1.5})).is_err());
        assert!(validate_settings(&schema, &json!({"count":9007199254740992_u64})).is_err());
    }

    #[test]
    fn compatible_upgrade_preserves_instances_and_incompatible_upgrade_keeps_old_version() {
        let root = std::env::temp_dir().join(format!("souldesk-upgrade-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let baseline: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        import(&store, &fixture(&root, &baseline, None)).unwrap();
        let id = create_instance(&store, "local.example.clock").unwrap();
        let sessions = Sessions::default();
        let session = open_session(&store, &sessions, "main", &id).unwrap();
        gateway(
            &store,
            &sessions,
            "main",
            session["token"].as_str().unwrap(),
            "settings.update",
            json!({"title":"自定义名称","showSeconds":true}),
        )
        .unwrap();
        gateway(
            &store,
            &sessions,
            "main",
            session["token"].as_str().unwrap(),
            "storage.set",
            json!({"note":"不能丢失"}),
        )
        .unwrap();
        let mut next = baseline.clone();
        next["version"] = json!("0.2.0");
        next["settingsSchema"]["properties"]["zone"] = json!({"type":"string","default":"UTC"});
        import(&store, &fixture(&root, &next, None)).unwrap();
        let value = store.snapshot().unwrap();
        assert_eq!(value["packages"][0]["version"], "0.2.0");
        assert_eq!(
            value["instances"][0]["settings"],
            json!({"title":"自定义名称","showSeconds":true,"zone":"UTC"})
        );
        let db = store.db.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT storage FROM widget_instances WHERE id=?1",
                [&id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            json!({"note":"不能丢失"}).to_string()
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM widget_versions WHERE package_id='local.example.clock'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            2
        );
        drop(db);
        let mut incompatible = next.clone();
        incompatible["version"] = json!("0.3.0");
        incompatible["settingsSchema"]["properties"]["title"] =
            json!({"type":"integer","default":1});
        assert!(import(&store, &fixture(&root, &incompatible, None)).is_err());
        assert_eq!(store.snapshot().unwrap()["packages"], value["packages"]);
        assert_eq!(store.snapshot().unwrap()["instances"], value["instances"]);
        assert!(import(&store, &fixture(&root, &baseline, None)).is_err());
        let mut replaced = next;
        replaced["name"] = json!("同版本的不同内容");
        assert!(import(&store, &fixture(&root, &replaced, None)).is_err());
    }

    #[test]
    fn staged_preview_writes_never_change_active_instances_and_commit_failures_roll_back() {
        let root = std::env::temp_dir().join(format!("souldesk-staging-{}", Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let baseline: Value =
            serde_json::from_str(include_str!("../../examples/clock/manifest.json")).unwrap();
        import(&store, &fixture(&root, &baseline, None)).unwrap();
        let id = create_instance(&store, "local.example.clock").unwrap();
        let before = store.snapshot().unwrap();
        let mut next = baseline.clone();
        next["version"] = json!("0.2.0");
        let staged = stage(&store, &fixture(&root, &next, None)).unwrap();
        assert_eq!(store.snapshot().unwrap(), before);
        let preview = Arc::new(Mutex::new(Preview {
            package: staged.clone(),
            settings: initial_settings(&staged.manifest),
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let sessions = Sessions::default();
        assert!(open_preview(&sessions, "main", "trial-test", preview.clone()).is_err());
        let session = open_preview(&sessions, "trial-test", "trial-test", preview).unwrap();
        gateway(
            &store,
            &sessions,
            "trial-test",
            session["token"].as_str().unwrap(),
            "settings.update",
            json!({"title":"试加载写入","showSeconds":true}),
        )
        .unwrap();
        gateway(
            &store,
            &sessions,
            "trial-test",
            session["token"].as_str().unwrap(),
            "storage.set",
            json!({"test":true}),
        )
        .unwrap();
        assert!(gateway(
            &store,
            &sessions,
            "trial-test",
            session["token"].as_str().unwrap(),
            "notifications.test",
            Value::Null
        )
        .is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        // 模拟实例配置已更新后，登记新版本时磁盘/约束失败；整个事务必须回滚。
        store.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_upgrade BEFORE UPDATE ON widget_packages BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
        assert!(activate(&store, &staged).is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        let active = open_session(&store, &sessions, "main", &id).unwrap();
        assert!(resource(&store, active["resourcePath"].as_str().unwrap()).is_ok());
    }
}
