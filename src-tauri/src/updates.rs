//! 更新仅由管理窗口发起。下载完成必须通过官方签名校验，安装前保存布局并校验一致备份。
use crate::{
    business,
    network::NetworkConfig,
    store::{err, now_ms, AppResult},
    windows, Core,
};
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct State {
    operation: tokio::sync::Mutex<()>,
    inner: Mutex<Inner>,
}
#[derive(Default)]
struct Inner {
    phase: String,
    candidate: Option<Update>,
    bytes: Option<Vec<u8>>,
    downloaded: u64,
    total: Option<u64>,
    checked_at: Option<u64>,
    error: Option<String>,
    backup: Option<String>,
}
pub fn snapshot(app: &AppHandle) -> AppResult<Value> {
    let core = app.state::<Core>();
    let state = core.updates.inner.lock().map_err(err)?;
    Ok(json!({
        "currentVersion": app.package_info().version.to_string(),
        "autoCheck": core.store.preference("update_auto_check").as_deref() != Some("false"),
        "phase": if state.phase.is_empty() { "idle" } else { &state.phase },
        "version": state.candidate.as_ref().map(|u| &u.version),
        "notes": state.candidate.as_ref().and_then(|u| u.body.as_ref()),
        "downloaded": state.downloaded, "total": state.total,
        "checkedAt": state.checked_at, "error": state.error, "backup": state.backup,
    }))
}
fn change(app: &AppHandle, action: impl FnOnce(&mut Inner)) -> AppResult<()> {
    action(&mut *app.state::<Core>().updates.inner.lock().map_err(err)?);
    app.emit_to("main", "host:changed", ()).map_err(err)
}
fn matching<'a>(candidate: &'a Option<Update>, expected: Option<&str>) -> AppResult<&'a Update> {
    candidate
        .as_ref()
        .filter(|u| expected == Some(u.version.as_str()))
        .ok_or("更新版本已变化，请重新检查并确认".into())
}
async fn check(app: &AppHandle) -> AppResult<()> {
    change(app, |s| {
        s.phase = "checking".into();
        s.error = None;
        s.candidate = None;
        s.bytes = None;
        s.downloaded = 0;
        s.total = None;
    })?;
    let raw = app
        .state::<Core>()
        .store
        .preference("network")
        .map(|s| serde_json::from_str::<Value>(&s).map_err(err))
        .transpose()?
        .unwrap_or(json!({"mode":"system"}));
    let mut builder = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .restart_after_install(true);
    builder = match business::resolve_network(&raw)? {
        NetworkConfig::Direct => builder.no_proxy(),
        NetworkConfig::Http { host, port } => {
            builder.proxy(format!("http://{host}:{port}").parse().map_err(err)?)
        }
        NetworkConfig::Socks5 { host, port } => {
            builder.proxy(format!("socks5h://{host}:{port}").parse().map_err(err)?)
        }
    };
    let update = builder.build().map_err(err)?.check().await.map_err(err)?;
    if let Some(update) = &update {
        if update.download_url.scheme() != "https" {
            return Err("更新下载地址必须使用HTTPS".into());
        }
    }
    change(app, |s| {
        s.phase = if update.is_some() {
            "available"
        } else {
            "latest"
        }
        .into();
        s.candidate = update;
        s.checked_at = Some(now_ms());
    })
}
async fn download(app: &AppHandle, expected: Option<&str>) -> AppResult<()> {
    let update = matching(
        &app.state::<Core>()
            .updates
            .inner
            .lock()
            .map_err(err)?
            .candidate,
        expected,
    )?
    .clone();
    change(app, |s| {
        s.phase = "downloading".into();
        s.downloaded = 0;
        s.total = None;
        s.error = None;
        s.bytes = None;
    })?;
    let mut last = std::time::Instant::now() - Duration::from_secs(1);
    let bytes = update
        .download(
            |chunk, total| {
                if let Ok(mut state) = app.state::<Core>().updates.inner.lock() {
                    state.downloaded += chunk as u64;
                    state.total = total;
                }
                if last.elapsed() >= Duration::from_millis(200) {
                    let _ = app.emit_to("main", "host:changed", ());
                    last = std::time::Instant::now();
                }
            },
            || {},
        )
        .await
        .map_err(err)?;
    change(app, |s| {
        s.phase = "downloaded".into();
        s.bytes = Some(bytes);
    })
}
async fn install(app: &AppHandle, expected: Option<&str>) -> AppResult<()> {
    let core = app.state::<Core>();
    // 开发与隔离运行只允许检查、下载，不允许替换用户正在使用的安装。
    if core.integration.is_some() || core.verification.is_some() || cfg!(debug_assertions) {
        return Err("请在正常安装版中安装更新；开发或隔离运行只支持检查与下载".into());
    }
    let (update, bytes) = {
        let state = core.updates.inner.lock().map_err(err)?;
        let update = matching(&state.candidate, expected)?.clone();
        let bytes = state.bytes.clone().ok_or("请先下载并通过签名校验")?;
        (update, bytes)
    };
    change(app, |s| {
        s.phase = "installing".into();
        s.error = None;
    })?;
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&handle, |app| {
            windows::save_all_geometry(app)?;
            let backup = app.state::<Core>().store.backup()?;
            change(app, |s| s.backup = Some(backup.display().to_string()))?;
            // 官方插件使用NSIS /UPDATE，不进入卸载/删除应用数据分支；启动失败会返回错误。
            update.install(bytes).map_err(err)
        })
    })
    .await
    .map_err(err)?
}
pub async fn command(
    app: AppHandle,
    action: &str,
    enabled: Option<bool>,
    expected: Option<&str>,
) -> AppResult<Value> {
    if action == "auto-check" {
        let enabled = enabled.ok_or("缺少自动检查设置")?;
        app.state::<Core>()
            .store
            .save_preference("update_auto_check", if enabled { "true" } else { "false" })?;
        let _ = app.emit_to("main", "host:changed", ());
        return snapshot(&app);
    }
    let core = app.state::<Core>();
    let _operation = core
        .updates
        .operation
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍后重试")?;
    let result = match action {
        "check" => check(&app).await,
        "download" => download(&app, expected).await,
        "install" => install(&app, expected).await,
        _ => Err("未知更新操作".into()),
    };
    if let Err(error) = result {
        change(&app, |s| {
            s.phase = "error".into();
            s.error = Some(error.clone());
        })?;
        return Err(error);
    }
    snapshot(&app)
}
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            let should_check = snapshot(&handle).is_ok_and(|s| {
                s["autoCheck"] == true
                    && ["idle", "latest", "error"].contains(&s["phase"].as_str().unwrap_or(""))
            });
            if should_check {
                let _ = command(handle.clone(), "check", None, None).await;
            }
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
            if handle
                .state::<Core>()
                .quitting
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_candidate_is_never_installable() {
        assert!(matching(&None, Some("99.0.0")).is_err());
    }
}
