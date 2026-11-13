//! 组件先在无前台激活的隐藏容器试加载，成功后才切换持久登记。
use crate::{
    presets,
    store::{err, AppResult},
    widgets::{self, Preview, StagedPackage},
    windows, Core,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

pub async fn install(app: &AppHandle, path: &Path) -> AppResult<widgets::Manifest> {
    let path = path.to_owned();
    install_with(app, move |store| widgets::stage(store, &path)).await
}

pub async fn ensure_preset(app: &AppHandle, id: &str) -> AppResult<()> {
    let Some(preset) = presets::find(id)? else {
        return Ok(());
    };
    let installed: bool = app
        .state::<Core>()
        .store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM widget_packages WHERE id=?1)",
            [id],
            |row| row.get(0),
        )
        .map_err(err)?;
    // 已登记的包（包括手动导入的新版）优先，预置资源不覆盖用户版本或配置。
    if !installed {
        install_with(app, move |store| {
            widgets::stage_bytes(store, preset.archive()?)
        })
        .await?;
    }
    Ok(())
}

async fn install_with(
    app: &AppHandle,
    stage: impl FnOnce(&crate::store::Store) -> AppResult<StagedPackage> + Send + 'static,
) -> AppResult<widgets::Manifest> {
    if app.state::<Core>().installing.swap(true, Ordering::SeqCst) {
        return Err("CONFLICT: 已有组件正在导入".into());
    }
    let handle = app.clone();
    let staged = tauri::async_runtime::spawn_blocking(move || stage(&handle.state::<Core>().store))
        .await
        .map_err(err)
        .and_then(|r| r);
    let result = match staged {
        Ok(staged) => try_install(app, staged).await,
        Err(e) => Err(e),
    };
    app.state::<Core>()
        .installing
        .store(false, Ordering::SeqCst);
    result
}

async fn try_install(app: &AppHandle, staged: StagedPackage) -> AppResult<widgets::Manifest> {
    if staged.unchanged() {
        return widgets::activate_sessions(
            &app.state::<Core>().store,
            &app.state::<Core>().sessions,
            &staged,
        );
    }
    let mut contexts = vec![(widgets::initial_settings(&staged.manifest), json!({}))];
    for (_, settings, storage) in &staged.instances {
        contexts.push((
            widgets::compatible_settings(&staged.manifest.settings_schema, settings)?,
            serde_json::from_str(storage).map_err(err)?,
        ));
    }
    for (settings, storage) in contexts {
        trial_once(app, &staged, settings, storage).await?;
    }
    widgets::activate_sessions(
        &app.state::<Core>().store,
        &app.state::<Core>().sessions,
        &staged,
    )
}

async fn trial_once(
    app: &AppHandle,
    staged: &StagedPackage,
    settings: Value,
    storage: Value,
) -> AppResult<()> {
    let label = format!("trial-{}", uuid::Uuid::new_v4());
    let preview = Arc::new(Mutex::new(Preview {
        settings,
        storage,
        package: staged.clone(),
        ready_since: None,
        failure: None,
    }));
    app.state::<Core>()
        .trials
        .lock()
        .map_err(err)?
        .insert(label.clone(), preview.clone());
    let handle = app.clone();
    let trial_label = label.clone();
    let built = tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&handle, |app| {
            windows::build(
                app,
                &trial_label,
                format!("index.html?trial={trial_label}"),
                320.,
                240.,
            )
            .map(|_| ())
        })
    })
    .await
    .map_err(err)
    .and_then(|r| r);
    let result = async {
        built?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let state = preview.lock().map_err(err)?;
                if let Some(error) = &state.failure {
                    return Err(format!("LOAD_FAILED: 试加载失败，原版已保留：{error}"));
                }
                if state
                    .ready_since
                    .is_some_and(|time| time.elapsed() >= Duration::from_millis(800))
                {
                    break;
                }
            }
            if Instant::now() >= deadline {
                return Err("LOAD_FAILED: 组件未在10秒内完成消息桥试加载，原版已保留".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
    .await;
    if result.is_err() && app.state::<Core>().verification.is_some() {
        if let Some(window) = app.get_webview_window(&label) {
            let handle = app.clone();
            let report_label = label.clone();
            let _=window.eval_with_callback("({href:location.href,ready:document.readyState,hostTokenPresent:typeof window.__SOULDESK_HOST_TOKEN__==='string',text:document.body?.innerText.slice(0,1500),frames:Array.from(document.querySelectorAll('iframe')).map(f=>f.src)})",move|result|{
                if let Ok(value)=serde_json::from_str::<Value>(&result){if let Ok(mut reports)=handle.state::<Core>().view_reports.lock(){reports.insert(report_label.clone(),value);}}
            });
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    app.state::<Core>()
        .trials
        .lock()
        .map_err(err)?
        .remove(&label);
    app.state::<Core>()
        .sessions
        .0
        .lock()
        .map_err(err)?
        .retain(|_, s| s.window != label);
    if let Err(error) = windows::destroy_window(app, &label) {
        return Err(format!("试加载容器关闭失败：{error}"));
    }
    result
}

#[tauri::command]
pub fn trial_failed(
    window: tauri::WebviewWindow,
    core: tauri::State<'_, Core>,
    message: String,
) -> AppResult<()> {
    if let Some(preview) = core.trials.lock().map_err(err)?.get(window.label()) {
        preview.lock().map_err(err)?.failure = Some(message.chars().take(1000).collect());
    }
    Ok(())
}

pub fn session(core: &Core, window: &str, instance: &str) -> AppResult<Option<Value>> {
    let trials = core.trials.lock().map_err(err)?;
    let Some(preview) = trials.get(instance) else {
        return Ok(None);
    };
    widgets::open_preview(&core.sessions, window, instance, preview.clone()).map(Some)
}
