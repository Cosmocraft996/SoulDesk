//! 相册只读取用户选定的目录，目录授权按实例保存在宿主中。

use crate::{
    store::{err, AppResult, Store},
    widgets, Core,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

const IMAGE_LIMIT: usize = 16 * 1024 * 1024;
const ENTRY_LIMIT: usize = 8192;
const IMAGE_COUNT_LIMIT: usize = 4096;
const IMAGE_FORMATS: &[&str] = &["jpg", "jpeg", "gif", "png", "webp"];

#[derive(Serialize, Deserialize)]
struct DirectoryGrant {
    path: PathBuf,
    revision: String,
}

fn grant_key(instance: &str) -> String {
    format!("media_directory:{instance}")
}
fn load_grant(store: &Store, instance: &str) -> AppResult<Option<DirectoryGrant>> {
    let raw: Option<String> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [grant_key(instance)],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    raw.map(|value| serde_json::from_str(&value).map_err(err))
        .transpose()
}
fn remember_directory(
    store: &Store,
    instance: &str,
    directory: &Path,
) -> AppResult<DirectoryGrant> {
    let path = directory.canonicalize().map_err(err)?;
    if !path.is_dir() {
        return Err("INVALID_ARGUMENT: 请选择图片文件夹".into());
    }
    let grant = DirectoryGrant {
        path,
        revision: uuid::Uuid::new_v4().to_string(),
    };
    list_images(&grant)?;
    // 实例删除与授权写入共用数据库锁，不为已删除的实例留下目录授权。
    let changed = store.db.lock().map_err(err)?.execute("INSERT INTO app_settings(key,value) SELECT ?1,?2 FROM widget_instances WHERE id=?3 ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![grant_key(instance), serde_json::to_string(&grant).map_err(err)?, instance]).map_err(err)?;
    if changed != 1 {
        return Err("SESSION_EXPIRED: 相册实例已删除".into());
    }
    Ok(grant)
}
fn checked_root(grant: &DirectoryGrant) -> AppResult<PathBuf> {
    let path = grant.path.canonicalize().map_err(err)?;
    if path != grant.path || !path.is_dir() {
        return Err("CONFLICT: 图片目录已变化，请重新选择".into());
    }
    Ok(path)
}
fn image_name(name: &str) -> bool {
    !name.contains(['/', '\\'])
        && widgets::safe_path(name)
        && Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| IMAGE_FORMATS.contains(&value.to_ascii_lowercase().as_str()))
}
fn list_images(grant: &DirectoryGrant) -> AppResult<Value> {
    let root = checked_root(grant)?;
    let mut files = Vec::new();
    for (index, entry) in std::fs::read_dir(&root).map_err(err)?.enumerate() {
        if index >= ENTRY_LIMIT {
            return Err("LIMIT: 文件夹条目超过8192项，请选择较小的图片文件夹".into());
        }
        let entry = entry.map_err(err)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !image_name(&name) || !entry.file_type().map_err(err)?.is_file() {
            continue;
        }
        let path = entry.path().canonicalize().map_err(err)?;
        if path.parent() != Some(root.as_path()) {
            continue;
        }
        if files.len() >= IMAGE_COUNT_LIMIT {
            return Err("LIMIT: 单个相册最多4096张图片".into());
        }
        files.push(json!({"name":name,"size":entry.metadata().map_err(err)?.len()}));
    }
    files.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));
    Ok(
        json!({"name":root.file_name().map(|value|value.to_string_lossy()).unwrap_or_default(),"revision":grant.revision,"files":files}),
    )
}
fn read_image(grant: &DirectoryGrant, revision: &str, name: &str) -> AppResult<Value> {
    if revision != grant.revision {
        return Err("CONFLICT: 相册目录已重新选择，请刷新列表".into());
    }
    if !image_name(name) {
        return Err("INVALID_ARGUMENT: 图片名称无效".into());
    }
    let root = checked_root(grant)?;
    let path = root.join(name).canonicalize().map_err(err)?;
    if path.parent() != Some(root.as_path()) {
        return Err("CAPABILITY_DENIED: 图片不在授权目录".into());
    }
    let file = std::fs::File::open(&path).map_err(err)?;
    let metadata = file.metadata().map_err(err)?;
    if !metadata.is_file() || metadata.len() > IMAGE_LIMIT as u64 {
        return Err("LIMIT: 单张相册图片上限16 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(IMAGE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > IMAGE_LIMIT {
        return Err("LIMIT: 单张相册图片上限16 MiB".into());
    }
    Ok(
        json!({"name":name,"mime":mime_guess::from_path(path).first_or_octet_stream().to_string(),"data":STANDARD.encode(bytes)}),
    )
}

/// 复用documents.open的用户等待语义；directory:true时仅授权当前实例的只读相册目录。
pub async fn select_directory(app: &AppHandle, window: &str, token: &str) -> AppResult<Value> {
    let core = app.state::<Core>();
    widgets::require_capability(
        &core.store,
        &core.sessions,
        window,
        token,
        "media.directory",
    )?;
    let _guard = crate::documents::start_dialog(&core)?;
    let handle = app.clone();
    let label = window.to_owned();
    let session = token.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let mut picker = handle.dialog().file().set_title("选择轮播相册的图片文件夹");
        if let Some(owner) = handle.get_webview_window(&label) {
            picker = picker.set_parent(&owner);
        }
        let Some(directory) = picker.blocking_pick_folder() else {
            return Ok(Value::Null);
        };
        let core = handle.state::<Core>();
        let instance = widgets::require_capability(
            &core.store,
            &core.sessions,
            &label,
            &session,
            "media.directory",
        )?;
        let grant =
            remember_directory(&core.store, &instance, &directory.into_path().map_err(err)?)?;
        list_images(&grant)
    })
    .await
    .map_err(err)?
}

pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    if !["media.list", "media.read", "media.clear"].contains(&method) {
        return Err("CAPABILITY_DENIED: 未开放的相册操作".into());
    }
    let handle = app.clone();
    let label = window.to_owned();
    let session = token.to_owned();
    let method = method.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        let instance = widgets::require_capability(
            &core.store,
            &core.sessions,
            &label,
            &session,
            "media.directory",
        )?;
        if method == "media.clear" {
            core.store
                .db
                .lock()
                .map_err(err)?
                .execute(
                    "DELETE FROM app_settings WHERE key=?1",
                    [grant_key(&instance)],
                )
                .map_err(err)?;
            return Ok(Value::Null);
        }
        let Some(grant) = load_grant(&core.store, &instance)? else {
            return if method == "media.list" {
                Ok(Value::Null)
            } else {
                Err("NOT_FOUND: 请先选择图片文件夹".into())
            };
        };
        let result = if method == "media.list" {
            list_images(&grant)?
        } else {
            read_image(
                &grant,
                args["revision"]
                    .as_str()
                    .ok_or("INVALID_ARGUMENT: 缺少相册版本")?,
                args["name"]
                    .as_str()
                    .ok_or("INVALID_ARGUMENT: 缺少图片名称")?,
            )?
        };
        widgets::require_capability(
            &core.store,
            &core.sessions,
            &label,
            &session,
            "media.directory",
        )?;
        if load_grant(&core.store, &instance)?
            .is_none_or(|current| current.revision != grant.revision)
        {
            return Err("CONFLICT: 相册目录已变化，请刷新列表".into());
        }
        Ok(result)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{presets, store::Store, widgets};
    #[cfg(unix)]
    use serde_json::json;
    use std::path::PathBuf;

    fn sample() -> (Store, String, String, PathBuf) {
        let root = std::env::temp_dir().join(format!("souldesk-media-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.join("db")).unwrap();
        let package = widgets::stage_bytes(
            &store,
            presets::find("local.example.clock")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&store, &package).unwrap();
        let first = widgets::create_instance(&store, "local.example.clock").unwrap();
        let second = widgets::create_instance(&store, "local.example.clock").unwrap();
        let images = root.join("照片");
        std::fs::create_dir(&images).unwrap();
        (store, first, second, images)
    }

    #[test]
    fn selected_directory_is_private_persistent_and_instance_scoped() {
        let (store, first, second, images) = sample();
        std::fs::write(images.join("照片.PNG"), b"first-image").unwrap();
        let grant = remember_directory(&store, &first, &images).unwrap();
        assert!(load_grant(&store, &second).unwrap().is_none());
        assert!(!store
            .snapshot()
            .unwrap()
            .to_string()
            .contains(&serde_json::to_string(&grant.path).unwrap()));
        let reopened = Store::open(store.root.clone()).unwrap();
        assert_eq!(
            load_grant(&reopened, &first).unwrap().unwrap().revision,
            grant.revision
        );
        assert_eq!(
            read_image(&grant, &grant.revision, "照片.PNG").unwrap()["data"],
            "Zmlyc3QtaW1hZ2U="
        );
        widgets::remove_instance(&store, &widgets::Sessions::default(), &first).unwrap();
        assert!(load_grant(&store, &first).unwrap().is_none());
        assert!(remember_directory(&store, &first, &images).is_err());
        assert_eq!(
            std::fs::read(images.join("照片.PNG")).unwrap(),
            b"first-image"
        );
    }

    #[test]
    fn listing_only_exposes_immediate_images_and_reads_reject_other_paths_or_old_grants() {
        let (store, first, _, images) = sample();
        std::fs::write(images.join("b.jpg"), b"b").unwrap();
        std::fs::write(images.join("a.webp"), b"a").unwrap();
        std::fs::write(images.join("secret.txt"), b"private").unwrap();
        std::fs::create_dir(images.join("nested")).unwrap();
        std::fs::write(images.join("nested/x.png"), b"nested").unwrap();
        let old = remember_directory(&store, &first, &images).unwrap();
        let listing = list_images(&old).unwrap();
        assert_eq!(listing["files"].as_array().unwrap().len(), 2);
        assert_eq!(listing["files"][0]["name"], "a.webp");
        for path in [
            "../private.jpg",
            "nested/x.png",
            "C:\\secret.png",
            "secret.txt",
        ] {
            assert!(read_image(&old, &old.revision, path).is_err());
        }
        let current = remember_directory(&store, &first, &images).unwrap();
        assert!(read_image(&current, &old.revision, "b.jpg")
            .unwrap_err()
            .starts_with("CONFLICT"));
        let large = std::fs::File::create(images.join("large.png")).unwrap();
        large.set_len(IMAGE_LIMIT as u64 + 1).unwrap();
        assert!(read_image(&current, &current.revision, "large.png")
            .unwrap_err()
            .starts_with("LIMIT"));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_cannot_escape_the_selected_directory() {
        let (store, first, _, images) = sample();
        let outside = images.parent().unwrap().join("outside.png");
        std::fs::write(&outside, b"private").unwrap();
        std::os::unix::fs::symlink(&outside, images.join("link.png")).unwrap();
        let grant = remember_directory(&store, &first, &images).unwrap();
        assert_eq!(list_images(&grant).unwrap()["files"], json!([]));
        assert!(read_image(&grant, &grant.revision, "link.png").is_err());
    }
}
