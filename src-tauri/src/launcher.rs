//! 文件收纳盒只持有用户为当前实例选择的目录与额外项目；排除仅修改显示记录。
use crate::{
    store::{err, AppResult, Store},
    widgets, Core,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

const FILE_LIMIT: usize = 4096;
const EXTRA_LIMIT: usize = 1024;
#[derive(Clone, Serialize, Deserialize)]
struct Extra {
    id: String,
    path: PathBuf,
    directory: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Grant {
    revision: String,
    root: Option<PathBuf>,
    extras: Vec<Extra>,
    excluded: Vec<PathBuf>,
}
fn key(instance: &str) -> String {
    format!("file_launcher:{instance}")
}
fn load(store: &Store, instance: &str) -> AppResult<Option<Grant>> {
    let raw: Option<String> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance)],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(err))
        .transpose()
}
fn commit(store: &Store, instance: &str, previous: Option<&str>, grant: &Grant) -> AppResult<()> {
    let db = store.db.lock().map_err(err)?;
    let current: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance)],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let current = current
        .map(|raw| serde_json::from_str::<Grant>(&raw).map_err(err))
        .transpose()?;
    if current.as_ref().map(|grant| grant.revision.as_str()) != previous {
        return Err("CONFLICT: 文件授权已变化，请刷新后重试".into());
    }
    let changed = db.execute("INSERT INTO app_settings(key,value) SELECT ?1,?2 FROM widget_instances WHERE id=?3 ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key(instance),serde_json::to_string(grant).map_err(err)?,instance]).map_err(err)?;
    if changed != 1 {
        return Err("SESSION_EXPIRED: 文件收纳盒实例已删除".into());
    }
    Ok(())
}
fn revision<'a>(grant: &'a Grant, args: &Value) -> AppResult<&'a str> {
    if args["revision"].as_str() != Some(grant.revision.as_str()) {
        return Err("CONFLICT: 文件列表已变化，请刷新后重试".into());
    }
    Ok(&grant.revision)
}
fn child_ref(name: &str) -> String {
    format!("child:{}", URL_SAFE_NO_PAD.encode(name.as_bytes()))
}
fn child_name(id: &str) -> AppResult<String> {
    let encoded = id
        .strip_prefix("child:")
        .filter(|value| value.len() <= 4096)
        .ok_or("INVALID_ARGUMENT: 文件标识无效")?;
    let name = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).map_err(err)?).map_err(err)?;
    if name.is_empty()
        || [".", ".."].contains(&name.as_str())
        || name.contains(['/', '\\', ':', '\0'])
    {
        return Err("CAPABILITY_DENIED: 仅允许主目录的直接子项目".into());
    }
    Ok(name)
}
fn excluded_ref(path: &Path) -> String {
    format!(
        "excluded:{:x}",
        Sha256::digest(path.to_string_lossy().as_bytes())
    )
}
fn checked_path(path: &Path) -> AppResult<PathBuf> {
    let resolved = path.canonicalize().map_err(err)?;
    if resolved != path || !(resolved.is_file() || resolved.is_dir()) {
        return Err("CAPABILITY_DENIED: 已授权项目已变化，请重新选择".into());
    }
    Ok(resolved)
}
fn resolve(grant: &Grant, id: &str, existing: bool) -> AppResult<PathBuf> {
    let path = if id == "root" {
        grant.root.clone().ok_or("NOT_FOUND: 尚未选择主目录")?
    } else if id.starts_with("child:") {
        let root = grant.root.as_ref().ok_or("NOT_FOUND: 尚未选择主目录")?;
        checked_path(root)?;
        root.join(child_name(id)?)
    } else {
        grant
            .extras
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.path.clone())
            .ok_or("CAPABILITY_DENIED: 未授权的文件标识")?
    };
    if existing {
        checked_path(&path)
    } else {
        Ok(path)
    }
}
fn milliseconds(time: std::io::Result<SystemTime>) -> Option<u64> {
    time.ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}
fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(path) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{path}")
    } else {
        value.strip_prefix("\\\\?\\").unwrap_or(&value).to_owned()
    }
}
fn file_info(path: &Path, id: &str, source: &str, directory: bool) -> Value {
    let mut info = json!({"id":id,"name":path.file_name().unwrap_or(path.as_os_str()).to_string_lossy(),"displayPath":display_path(path),"source":source,"isDirectory":directory,"birthTime":null,"modifiedTime":null,"size":null,"unavailable":false});
    match checked_path(path).and_then(|path| std::fs::metadata(path).map_err(err)) {
        Ok(metadata) => {
            info["isDirectory"] = json!(metadata.is_dir());
            info["birthTime"] = json!(milliseconds(metadata.created()));
            info["modifiedTime"] = json!(milliseconds(metadata.modified()));
            info["size"] = json!(metadata.len().to_string());
        }
        Err(error) => {
            info["unavailable"] = json!(true);
            info["error"] = json!(error);
        }
    }
    info
}
fn snapshot(grant: &Grant) -> AppResult<Value> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut root_error = None;
    if let Some(root) = &grant.root {
        let collected = (|| -> AppResult<Vec<(PathBuf, Value)>> {
            checked_path(root)?;
            let mut items = Vec::new();
            for entry in std::fs::read_dir(root).map_err(err)? {
                if items.len() >= FILE_LIMIT {
                    return Err("LIMIT: 主目录最多4096项，请选择较小的目录".into());
                }
                let entry = entry.map_err(err)?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "INVALID_ARGUMENT: 目录中存在无法表示的文件名")?;
                let id = child_ref(&name);
                let path = root.join(&name);
                items.push((
                    path.clone(),
                    file_info(
                        &path,
                        &id,
                        "root",
                        entry.file_type().is_ok_and(|kind| kind.is_dir()),
                    ),
                ));
            }
            Ok(items)
        })();
        match collected {
            Ok(items) => {
                for (path, info) in items {
                    seen.insert(path.clone());
                    if !grant.excluded.contains(&path) {
                        files.push(info);
                    }
                }
            }
            Err(error) => root_error = Some(error),
        }
    }
    for extra in &grant.extras {
        if seen.insert(extra.path.clone()) && !grant.excluded.contains(&extra.path) {
            files.push(file_info(&extra.path, &extra.id, "extra", extra.directory));
        }
    }
    let excluded = grant.excluded.iter().map(|path|json!({"id":excluded_ref(path),"name":path.file_name().unwrap_or(path.as_os_str()).to_string_lossy(),"displayPath":display_path(path)})).collect::<Vec<_>>();
    Ok(
        json!({"revision":grant.revision,"root":grant.root.as_ref().map(|path|json!({"id":"root","name":path.file_name().unwrap_or(path.as_os_str()).to_string_lossy(),"displayPath":display_path(path)})),"rootError":root_error,"files":files,"excluded":excluded}),
    )
}
fn choose_path(
    store: &Store,
    instance: &str,
    args: &Value,
    mode: &str,
    selected: &Path,
) -> AppResult<Value> {
    let previous = load(store, instance)?;
    if previous.as_ref().map(|grant| grant.revision.as_str()) != args["revision"].as_str() {
        return Err("CONFLICT: 文件授权已变化，请刷新后重新选择".into());
    }
    let path = selected.canonicalize().map_err(err)?;
    if mode == "file" && !path.is_file() || mode != "file" && !path.is_dir() {
        return Err("INVALID_ARGUMENT: 选择的项目类型不符".into());
    }
    let mut grant = previous.clone().unwrap_or_default();
    if mode == "root" {
        grant.root = Some(path.clone());
    } else if !grant.extras.iter().any(|extra| extra.path == path) {
        if grant.extras.len() >= EXTRA_LIMIT {
            return Err("LIMIT: 最多额外添加1024个项目".into());
        }
        grant.extras.push(Extra {
            id: format!("extra:{}", uuid::Uuid::new_v4()),
            directory: path.is_dir(),
            path: path.clone(),
        });
    }
    grant.excluded.retain(|item| item != &path);
    grant.revision = uuid::Uuid::new_v4().to_string();
    let value = snapshot(&grant)?;
    if mode == "root" && !value["rootError"].is_null() {
        return Err(value["rootError"].as_str().unwrap().to_owned());
    }
    commit(
        store,
        instance,
        previous.as_ref().map(|grant| grant.revision.as_str()),
        &grant,
    )?;
    Ok(value)
}

/// 复用documents.open的原生用户等待，不开放组件自选任意路径。
pub async fn choose(app: &AppHandle, window: &str, token: &str, args: Value) -> AppResult<Value> {
    let mode = args["launcher"]
        .as_str()
        .filter(|mode| ["root", "file", "folder"].contains(mode))
        .ok_or("INVALID_ARGUMENT: 文件选择类型无效")?
        .to_owned();
    let core = app.state::<Core>();
    widgets::require_capability(&core.store, &core.sessions, window, token, "files.launcher")?;
    let _guard = crate::documents::start_dialog(&core)?;
    let handle = app.clone();
    let label = window.to_owned();
    let token = token.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let mut picker = handle.dialog().file().set_title(if mode == "root" {
            "选择文件收纳盒的主目录"
        } else if mode == "file" {
            "添加文件到收纳盒"
        } else {
            "添加文件夹到收纳盒"
        });
        if let Some(owner) = handle.get_webview_window(&label) {
            picker = picker.set_parent(&owner);
        }
        let selected = if mode == "file" {
            picker.blocking_pick_file()
        } else {
            picker.blocking_pick_folder()
        };
        let Some(selected) = selected else {
            return Ok(Value::Null);
        };
        let core = handle.state::<Core>();
        let instance = widgets::require_capability(
            &core.store,
            &core.sessions,
            &label,
            &token,
            "files.launcher",
        )?;
        choose_path(
            &core.store,
            &instance,
            &args,
            &mode,
            &selected.into_path().map_err(err)?,
        )
    })
    .await
    .map_err(err)?
}
fn edit_grant(grant: &mut Grant, method: &str, args: &Value) -> AppResult<()> {
    match method {
        "files.clear" => grant.root = None,
        "files.exclude" => {
            let id = args["id"]
                .as_str()
                .filter(|id| *id != "root")
                .ok_or("INVALID_ARGUMENT: 仅可排除文件列表项目")?;
            let path = resolve(grant, id, false)?;
            if !grant.excluded.contains(&path) {
                if grant.excluded.len() >= FILE_LIMIT {
                    return Err("LIMIT: 排除列表已满，请先恢复部分项目".into());
                }
                grant.excluded.push(path);
            }
        }
        "files.restore" => {
            if let Some(value) = args.get("id") {
                let id = value.as_str().ok_or("INVALID_ARGUMENT: 排除项目标识无效")?;
                let old = grant.excluded.len();
                grant.excluded.retain(|path| excluded_ref(path) != id);
                if old == grant.excluded.len() {
                    return Err("NOT_FOUND: 排除项目已变化".into());
                }
            } else {
                grant.excluded.clear();
            }
        }
        _ => return Err("CAPABILITY_DENIED: 未开放的文件收纳操作".into()),
    }
    grant.revision = uuid::Uuid::new_v4().to_string();
    Ok(())
}

pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    if ![
        "files.list",
        "files.open",
        "files.icon",
        "files.exclude",
        "files.restore",
        "files.clear",
    ]
    .contains(&method)
    {
        return Err("CAPABILITY_DENIED: 未开放的文件收纳操作".into());
    }
    let handle = app.clone();
    let label = window.to_owned();
    let token = token.to_owned();
    let method = method.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        let instance = widgets::require_capability(
            &core.store,
            &core.sessions,
            &label,
            &token,
            "files.launcher",
        )?;
        let current = load(&core.store, &instance)?;
        if method == "files.list" {
            return current.as_ref().map(snapshot).unwrap_or_else(|| {
                Ok(json!({"revision":null,"root":null,"rootError":null,"files":[],"excluded":[]}))
            });
        }
        let mut grant = current.ok_or("NOT_FOUND: 请先选择文件或主目录")?;
        let previous = revision(&grant, &args)?.to_owned();
        if ["files.open", "files.icon"].contains(&method.as_str()) {
            let path = resolve(
                &grant,
                args["id"]
                    .as_str()
                    .ok_or("INVALID_ARGUMENT: 缺少文件标识")?,
                true,
            )?;
            widgets::require_capability(
                &core.store,
                &core.sessions,
                &label,
                &token,
                "files.launcher",
            )?;
            if load(&core.store, &instance)?.is_none_or(|latest| latest.revision != previous) {
                return Err("CONFLICT: 文件授权已变化".into());
            }
            let result = if method == "files.icon" {
                native_icon(&path)?
            } else {
                native_open(&path)?
            };
            widgets::require_capability(
                &core.store,
                &core.sessions,
                &label,
                &token,
                "files.launcher",
            )?;
            if load(&core.store, &instance)?.is_none_or(|latest| latest.revision != previous) {
                return Err("CONFLICT: 文件授权已变化".into());
            }
            return Ok(result);
        }
        edit_grant(&mut grant, &method, &args)?;
        let value = snapshot(&grant)?;
        commit(&core.store, &instance, Some(&previous), &grant)?;
        Ok(value)
    })
    .await
    .map_err(err)?
}

#[cfg(not(windows))]
fn native_icon(_: &Path) -> AppResult<Value> {
    Err("UNSUPPORTED: 文件图标目前仅支持Windows".into())
}
#[cfg(not(windows))]
fn native_open(_: &Path) -> AppResult<Value> {
    Err("UNSUPPORTED: 文件打开目前仅支持Windows".into())
}
#[cfg(windows)]
use shell::{native_icon, native_open};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    fn sample() -> (Store, String, String, PathBuf) {
        let root = std::env::temp_dir().join(format!("souldesk-launcher-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.join("db")).unwrap();
        let package = widgets::stage_bytes(
            &store,
            presets::find("local.souldesk.widgetjs.grid")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&store, &package).unwrap();
        let first = widgets::create_instance(&store, "local.souldesk.widgetjs.grid").unwrap();
        let second = widgets::create_instance(&store, "local.souldesk.widgetjs.grid").unwrap();
        let files = root.join("资料");
        std::fs::create_dir(&files).unwrap();
        (store, first, second, files)
    }

    #[test]
    fn selection_survives_restart_exclusion_never_changes_files_and_removal_only_clears_own_grant()
    {
        let (store, first, second, files) = sample();
        std::fs::write(files.join("年度.报告.txt"), b"preserve").unwrap();
        std::fs::create_dir(files.join("目录.v2")).unwrap();
        std::fs::write(files.join("目录.v2/private.txt"), b"nested").unwrap();
        let snapshot =
            choose_path(&store, &first, &json!({"revision":null}), "root", &files).unwrap();
        assert_eq!(snapshot["files"].as_array().unwrap().len(), 2);
        let extra = files.parent().unwrap().join("额外.txt");
        std::fs::write(&extra, b"extra").unwrap();
        choose_path(
            &store,
            &first,
            &json!({"revision":snapshot["revision"]}),
            "file",
            &extra,
        )
        .unwrap();
        choose_path(&store, &second, &json!({"revision":null}), "root", &files).unwrap();
        let mut grant = load(&store, &first).unwrap().unwrap();
        let previous = grant.revision.clone();
        edit_grant(
            &mut grant,
            "files.exclude",
            &json!({"id":child_ref("年度.报告.txt")}),
        )
        .unwrap();
        commit(&store, &first, Some(&previous), &grant).unwrap();
        assert!(revision(&grant, &json!({"revision":previous})).is_err());
        assert_eq!(
            super::snapshot(&grant).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let reopened = Store::open(store.root.clone()).unwrap();
        let mut restored = load(&reopened, &first).unwrap().unwrap();
        assert_eq!(restored.excluded.len(), 1);
        assert_eq!(restored.extras.len(), 1);
        let excluded = super::snapshot(&restored).unwrap()["excluded"][0]["id"].clone();
        assert!(edit_grant(&mut restored, "files.restore", &json!({"id":false})).is_err());
        edit_grant(&mut restored, "files.restore", &json!({"id":excluded})).unwrap();
        assert_eq!(
            super::snapshot(&restored).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        edit_grant(&mut restored, "files.clear", &json!({})).unwrap();
        assert_eq!(
            super::snapshot(&restored).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let extra_id = restored.extras[0].id.clone();
        edit_grant(&mut restored, "files.exclude", &json!({"id":extra_id})).unwrap();
        edit_grant(&mut restored, "files.restore", &json!({})).unwrap();
        assert_eq!(
            super::snapshot(&restored).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let sessions = widgets::Sessions::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let mut settings = widgets::gateway(
            &store,
            &sessions,
            &label,
            session["token"].as_str().unwrap(),
            "settings.get",
            json!({}),
        )
        .unwrap();
        settings["title"] = json!("已保存的文件收纳盒");
        assert_eq!(
            widgets::gateway(
                &store,
                &sessions,
                &label,
                session["token"].as_str().unwrap(),
                "settings.update",
                settings.clone()
            )
            .unwrap(),
            settings
        );
        assert_eq!(
            widgets::gateway(
                &store,
                &sessions,
                &label,
                session["token"].as_str().unwrap(),
                "settings.get",
                json!({})
            )
            .unwrap(),
            settings
        );
        assert_eq!(
            widgets::require_capability(
                &store,
                &sessions,
                &label,
                session["token"].as_str().unwrap(),
                "files.launcher"
            )
            .unwrap(),
            first
        );
        widgets::remove_instance(&store, &sessions, &first).unwrap();
        assert!(load(&store, &first).unwrap().is_none());
        assert!(load(&store, &second).unwrap().is_some());
        assert!(commit(&store, &first, None, &restored).is_err());
        assert_eq!(
            std::fs::read(files.join("年度.报告.txt")).unwrap(),
            b"preserve"
        );
        assert_eq!(std::fs::read(extra).unwrap(), b"extra");
    }

    #[test]
    fn refs_cannot_escape_root_or_use_another_instances_extra_and_stale_selection_preserves_current(
    ) {
        let (store, first, second, files) = sample();
        std::fs::write(files.join("a.txt"), b"a").unwrap();
        choose_path(&store, &first, &json!({"revision":null}), "root", &files).unwrap();
        let grant = load(&store, &first).unwrap().unwrap();
        for name in [
            "../private.txt",
            "nested/private.txt",
            "C:\\private.txt",
            "a.txt:secret",
            "..",
            "",
        ] {
            assert!(resolve(&grant, &child_ref(name), true).is_err(), "{name}");
        }
        assert!(resolve(&grant, "C:/private.txt", true).is_err());
        assert!(choose_path(&store, &first, &json!({"revision":"old"}), "root", &files).is_err());
        assert_eq!(
            load(&store, &first).unwrap().unwrap().revision,
            grant.revision
        );
        let extra = files.join("a.txt");
        let selected =
            choose_path(&store, &second, &json!({"revision":null}), "file", &extra).unwrap();
        assert!(resolve(&grant, selected["files"][0]["id"].as_str().unwrap(), true).is_err());
        let label = format!("float-{first}");
        let sessions = widgets::Sessions::default();
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        assert!(widgets::require_capability(
            &store,
            &sessions,
            "another-window",
            session["token"].as_str().unwrap(),
            "files.launcher"
        )
        .is_err());
    }

    #[test]
    fn unavailable_root_keeps_revision_and_extra_items_for_reselection_without_fabricated_timestamps(
    ) {
        let (store, first, _, files) = sample();
        let missing = files.join("missing");
        let grant = Grant {
            revision: "revision".into(),
            root: Some(missing.clone()),
            extras: vec![Extra {
                id: "extra:a".into(),
                path: missing.clone(),
                directory: false,
            }],
            excluded: vec![],
        };
        commit(&store, &first, None, &grant).unwrap();
        let value = super::snapshot(&grant).unwrap();
        assert_eq!(value["revision"], "revision");
        assert!(value["rootError"].is_string());
        assert_eq!(value["files"][0]["unavailable"], true);
        assert!(value["files"][0]["birthTime"].is_null());
        let replacement = choose_path(
            &store,
            &first,
            &json!({"revision":"revision"}),
            "root",
            &files,
        )
        .unwrap();
        assert!(replacement["rootError"].is_null());
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_returns_real_thumbnail_and_folder_pixels_with_owned_resources() {
        let image = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../components/widgetjs/grid/image.png")
            .canonicalize()
            .unwrap();
        let thumbnail = native_icon(&image).unwrap();
        let folder = native_icon(image.parent().unwrap()).unwrap();
        for item in [&thumbnail, &folder] {
            let bytes = STANDARD.decode(item["rgba"].as_str().unwrap()).unwrap();
            assert_eq!(
                bytes.len(),
                item["width"].as_u64().unwrap() as usize
                    * item["height"].as_u64().unwrap() as usize
                    * 4
            );
            assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0));
        }
        assert_ne!(thumbnail["rgba"], folder["rgba"]);
    }
}

#[cfg(windows)]
mod shell {
    use super::*;
    use std::{
        mem::size_of,
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
    };
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::SIZE,
            System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT},
        },
    };
    use windows_sys::Win32::{
        Graphics::Gdi::*,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    struct Com;
    impl Com {
        fn initialize() -> AppResult<Self> {
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
                    .ok()
                    .map_err(err)?;
            }
            Ok(Self)
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    fn wide(path: &Path) -> Vec<u16> {
        let mut value = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let unc = "\\\\?\\UNC\\".encode_utf16().collect::<Vec<_>>();
        let prefix = "\\\\?\\".encode_utf16().collect::<Vec<_>>();
        if value.starts_with(&unc) {
            value.drain(..unc.len());
            value.splice(..0, [92, 92]);
        } else if value.starts_with(&prefix) {
            value.drain(..prefix.len());
        }
        value.push(0);
        value
    }
    struct Bitmap(HBITMAP);
    impl Drop for Bitmap {
        fn drop(&mut self) {
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
    struct Dc(HDC);
    impl Drop for Dc {
        fn drop(&mut self) {
            unsafe {
                DeleteDC(self.0);
            }
        }
    }
    pub(super) fn native_icon(path: &Path) -> AppResult<Value> {
        let _com = Com::initialize()?;
        let path = wide(path);
        let factory: IShellItemImageFactory =
            unsafe { SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None) }.map_err(err)?;
        let bitmap = Bitmap(
            unsafe { factory.GetImage(SIZE { cx: 96, cy: 96 }, SIIGBF_RESIZETOFIT) }
                .map_err(err)?
                .0,
        );
        let mut description = BITMAP::default();
        if unsafe {
            GetObjectW(
                bitmap.0,
                size_of::<BITMAP>() as i32,
                &mut description as *mut _ as _,
            )
        } == 0
        {
            return Err(err(std::io::Error::last_os_error()));
        }
        let (width, height) = (description.bmWidth, description.bmHeight.abs());
        if !(1..=96).contains(&width) || !(1..=96).contains(&height) {
            return Err("INVALID_DATA: 系统图标尺寸异常".into());
        }
        let dc = Dc(unsafe { CreateCompatibleDC(null_mut()) });
        if dc.0.is_null() {
            return Err(err(std::io::Error::last_os_error()));
        }
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        if unsafe {
            GetDIBits(
                dc.0,
                bitmap.0,
                0,
                height as u32,
                pixels.as_mut_ptr() as _,
                &mut info,
                DIB_RGB_COLORS,
            )
        } != height
        {
            return Err("INVALID_DATA: 系统图标像素读取不完整".into());
        }
        let opaque = pixels.chunks_exact(4).all(|pixel| pixel[3] == 0);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if opaque {
                pixel[3] = 255;
            } else if pixel[3] > 0 && pixel[3] < 255 {
                for channel in 0..3 {
                    pixel[channel] = (pixel[channel] as u32 * 255 / pixel[3] as u32).min(255) as u8;
                }
            }
        }
        Ok(json!({"width":width,"height":height,"rgba":STANDARD.encode(pixels)}))
    }
    pub(super) fn native_open(path: &Path) -> AppResult<Value> {
        let _com = Com::initialize()?;
        let directory = wide(path.parent().unwrap_or(path));
        let path = wide(path);
        let verb = "open\0".encode_utf16().collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                null_mut(),
                verb.as_ptr(),
                path.as_ptr(),
                null(),
                directory.as_ptr(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if result <= 32 {
            return Err(format!("OPEN_FAILED: 系统未接受打开请求（{result}）"));
        }
        Ok(json!({"launched":true}))
    }
}
