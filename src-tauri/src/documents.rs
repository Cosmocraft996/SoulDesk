//! 组件仅通过用户选择的文件打开、保存文档，不接收任意文件路径。
use crate::{
    store::{err, AppResult},
    widgets, Core,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

const OPEN_LIMIT: usize = 16 * 1024 * 1024;
const SAVE_LIMIT: usize = 32 * 1024 * 1024;
const FORMATS: &[&str] = &[
    "excalidraw",
    "excalidrawlib",
    "json",
    "png",
    "jpg",
    "jpeg",
    "svg",
    "gif",
    "webp",
    "avif",
    "heic",
    "heif",
    "bmp",
    "ico",
    "txt",
    "md",
    "yaml",
    "yml",
    "zip",
];

fn validate_options(value: &Value) -> AppResult<(String, Vec<String>)> {
    let name = value
        .get("fileName")
        .and_then(Value::as_str)
        .unwrap_or("未命名.json");
    if name.is_empty()
        || name.len() > 256
        || name.contains(['/', '\\', '<', '>', '"', '|', '?', '*'])
        || !widgets::safe_path(name)
    {
        return Err("INVALID_ARGUMENT: 文件名只能是普通名称，不能含路径".into());
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !FORMATS.contains(&extension.as_str()) {
        return Err("INVALID_ARGUMENT: 不支持的文件格式".into());
    }
    let formats: Vec<String> = value
        .get("extensions")
        .map(|v| serde_json::from_value(v.clone()).map_err(err))
        .transpose()?
        .unwrap_or_else(|| vec![extension]);
    if formats.is_empty()
        || formats.len() > FORMATS.len()
        || formats.iter().any(|ext| !FORMATS.contains(&ext.as_str()))
    {
        return Err("INVALID_ARGUMENT: 文件类型过滤条件无效".into());
    }
    Ok((name.into(), formats))
}
fn read_selected(path: &Path, limit: usize) -> AppResult<Vec<u8>> {
    let file = File::open(path).map_err(err)?;
    if !file.metadata().map_err(err)?.is_file()
        || file.metadata().map_err(err)?.len() > limit as u64
    {
        return Err("LIMIT: 选中文件超出读取上限或不是普通文件".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > limit {
        return Err("LIMIT: 选中文件超出读取上限".into());
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if !path.is_absolute() {
        return Err("INVALID_ARGUMENT: 文件选择必须返回绝对路径".into());
    }
    let parent = path.parent().ok_or("INVALID_ARGUMENT: 文件没有父目录")?;
    let temporary = parent.join(format!(".souldesk-save-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(err)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let result = written
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(err);
    // 只清理本次通过create_new创建的单个临时文件，绝不删除选中的目标。
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub(crate) struct DialogGuard<'a>(&'a Core);
impl Drop for DialogGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut visibility) = self.0.visibility.lock() {
            visibility.dialog = false;
        }
    }
}

pub(crate) fn start_dialog(core: &Core) -> AppResult<DialogGuard<'_>> {
    let mut visibility = core.visibility.lock().map_err(err)?;
    if visibility.dialog {
        return Err("BUSY: 请先完成当前文件选择或对话框".into());
    }
    visibility.dialog = true;
    Ok(DialogGuard(core))
}

pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    if method == "documents.open" && args.get("launcher").is_some() {
        return crate::launcher::choose(app, window, token, args).await;
    }
    if method == "documents.open" && args["directory"] == true {
        return crate::media::select_directory(app, window, token).await;
    }
    if !["documents.open", "documents.save"].contains(&method) {
        return Err("CAPABILITY_DENIED: 未开放的文档操作".into());
    }
    let save = method == "documents.save";
    let grants = [
        "documents.user",
        if save {
            "documents.save"
        } else {
            "documents.open"
        },
    ];
    let core = app.state::<Core>();
    widgets::with_any_capability(&core.store, &core.sessions, window, token, &grants, |_| {
        Ok(())
    })?;
    let (name, extensions) = validate_options(&args)?;
    let bytes = if save {
        let data = args["data"]
            .as_str()
            .ok_or("INVALID_ARGUMENT: 缺少文件内容")?;
        if data.len() > (SAVE_LIMIT + 2) / 3 * 4 {
            return Err("LIMIT: 保存文件超过32 MiB".into());
        }
        let decoded = STANDARD
            .decode(data)
            .map_err(|_| "INVALID_ARGUMENT: 文件编码无效")?;
        if decoded.len() > SAVE_LIMIT {
            return Err("LIMIT: 保存文件超过32 MiB".into());
        }
        Some(decoded)
    } else {
        None
    };
    let _guard = start_dialog(&core)?;
    let handle = app.clone();
    let label = window.to_owned();
    let session = token.to_owned();
    let multiple = args["multiple"].as_bool().unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let mut picker = handle.dialog().file().set_title(if save { "保存组件文档" } else { "打开组件文档" }).add_filter("组件文档", &extensions.iter().map(String::as_str).collect::<Vec<_>>());
        if let Some(owner) = handle.get_webview_window(&label) { picker = picker.set_parent(&owner); }
        if let Some(bytes) = bytes {
            let Some(selected) = picker.set_file_name(&name).blocking_save_file() else { return Ok(Value::Null); };
            let mut path = selected.into_path().map_err(err)?;
            if path.extension().is_none() { path.set_extension(&extensions[0]); }
            let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if !extensions.iter().any(|value| value == &ext) { return Err("INVALID_ARGUMENT: 保存扩展名与选定格式不符".into()); }
            let core = handle.state::<Core>();
            widgets::with_any_capability(&core.store, &core.sessions, &label, &session, &grants, |_| Ok(()))?;
            atomic_write(&path, &bytes)?;
            return Ok(json!({"name":path.file_name().map(|s|s.to_string_lossy()),"size":bytes.len()}));
        }
        let selected = if multiple { picker.blocking_pick_files() } else { picker.blocking_pick_file().map(|path| vec![path]) };
        let Some(selected) = selected else { return Ok(Value::Null); };
        if selected.len() > 32 { return Err("LIMIT: 一次最多打开32个文件".into()); }
        let core = handle.state::<Core>();
        widgets::with_any_capability(&core.store, &core.sessions, &label, &session, &grants, |_| Ok(()))?;
        let mut result = Vec::new(); let mut remaining = OPEN_LIMIT;
        for selected in selected {
            let path = selected.into_path().map_err(err)?;
            let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if !extensions.iter().any(|value| value == &ext) { return Err("INVALID_ARGUMENT: 打开扩展名与选定格式不符".into()); }
            let bytes = read_selected(&path, remaining)?; remaining -= bytes.len();
            result.push(json!({"name":path.file_name().map(|s|s.to_string_lossy()),"mime":mime_guess::from_path(&path).first_or_octet_stream().to_string(),"data":STANDARD.encode(&bytes)}));
        }
        widgets::with_any_capability(&core.store, &core.sessions, &label, &session, &grants, |_| Ok(()))?;
        Ok(json!(result))
    }).await.map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn document_options_only_allow_basename_and_known_formats() {
        assert_eq!(
            validate_options(
                &json!({"fileName":"画板.excalidraw","extensions":["excalidraw","png"]})
            )
            .unwrap(),
            (
                "画板.excalidraw".into(),
                vec!["excalidraw".into(), "png".into()]
            )
        );
        for name in ["../a.json", "C:\\a.json", "a/b.json", "CON.json", "a.json."] {
            assert!(validate_options(&json!({"fileName":name,"extensions":["json"]})).is_err());
        }
        for format in ["exe", "../png", "html", "js"] {
            assert!(validate_options(&json!({"fileName":"x.json","extensions":[format]})).is_err());
        }
        assert!(validate_options(&json!({"fileName":"x.exe","extensions":["png"]})).is_err());
    }

    #[test]
    fn file_reads_are_bounded_and_do_not_modify_source() {
        let root =
            std::env::temp_dir().join(format!("souldesk-document-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("原图.png");
        std::fs::write(&path, b"original").unwrap();
        assert_eq!(read_selected(&path, 8).unwrap(), b"original");
        assert!(read_selected(&path, 7).unwrap_err().contains("LIMIT"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(read_selected(&root, 100).is_err());
    }

    #[test]
    fn atomic_save_replaces_complete_file_and_failed_replace_preserves_existing_data() {
        let root =
            std::env::temp_dir().join(format!("souldesk-document-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.excalidraw");
        std::fs::write(&path, b"before").unwrap();
        atomic_write(&path, b"after-complete").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"after-complete");
        let blocked = root.join("occupied.png");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("retained.txt"), b"retained").unwrap();
        assert!(atomic_write(&blocked, b"must-not-replace").is_err());
        assert_eq!(
            std::fs::read(blocked.join("retained.txt")).unwrap(),
            b"retained"
        );
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            2,
            "失败后仅清理本次创建的临时文件"
        );
    }
}
