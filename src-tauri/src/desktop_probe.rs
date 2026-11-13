//! 专用入口的平台探针；不初始化 Core、数据库、行情或正常应用插件。
use crate::desktop_geometry::{panel_geometry, PanelGeometry, WorkArea};
use crate::store::err;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const CLOCK: &str = "desktop-probe-clock";
const CARD: &str = "desktop-probe-card";
const PANEL: &str = "desktop-probe-panel";

pub(super) fn widget_state(
    app: &AppHandle,
) -> Result<Arc<crate::desktop_widgets::WidgetProbe>, String> {
    app.state::<Shared>()
        .widgets
        .clone()
        .ok_or("当前不是实际组件探针".into())
}
fn notify_widgets(app: &AppHandle) -> Result<(), String> {
    let widgets = widget_state(app)?;
    let snapshot = widgets.snapshot()?;
    for label in [CLOCK, CARD, PANEL] {
        if app.get_webview_window(label).is_some() {
            app.emit_to(label, "desktop-probe:widgets", &snapshot)
                .map_err(err)?;
        }
    }
    Ok(())
}
fn load_widget_packages(app: &AppHandle) -> Result<(), String> {
    let widgets = widget_state(app)?;
    for index in 0..2 {
        let foreground = native::foreground();
        let label = widgets.begin_trial(index)?;
        let result = (|| {
            let window = build_window(app, &label, 400., 320.)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !widgets.trial_ready(&label)? {
                if std::time::Instant::now() >= deadline {
                    return Err("LOAD_FAILED: 隐藏原包试加载超时".to_owned());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if window.is_visible().map_err(err)? || native::foreground() != foreground {
                return Err("隐藏试加载改变了窗口可见性或前台焦点".into());
            }
            Ok(())
        })();
        let closed = crate::windows::destroy_window(app, &label);
        widgets.end_trial(&label, index, result.is_ok() && closed.is_ok())?;
        closed?;
        result?;
        checkpoint(
            app,
            "widget-zip-activated",
            json!({"index":index,"hiddenTrialDestroyed":true}),
        )?;
    }
    Ok(())
}
fn change_widget_view(
    app: &AppHandle,
    id: &str,
    location: &str,
    visible: bool,
    inject_failure: bool,
) -> Result<(), String> {
    let widgets = widget_state(app)?;
    let previous = widgets.view(id)?;
    let ready_before = widgets.ready_count(id)?;
    let (_, revision) = widgets.change(id, &previous.location, false)?;
    let old_host = if previous.location == "desktop" {
        previous.desktop_label.as_str()
    } else {
        PANEL
    };
    let result = (|| {
        if previous.location == "desktop" && previous.visible {
            native::hide(&app.get_webview_window(old_host).ok_or("原桌面容器缺失")?)?;
        }
        notify_widgets(app)?;
        if previous.visible {
            widgets.wait_unmounted(old_host, revision)?;
        }
        let (_, target_revision) = widgets.change(id, location, visible)?;
        notify_widgets(app)?;
        if visible {
            let host = if location == "desktop" {
                previous.desktop_label.as_str()
            } else {
                PANEL
            };
            native::show(&app.get_webview_window(host).ok_or("目标展示容器缺失")?)?;
            widgets.wait_unmounted(host, target_revision)?;
            if inject_failure {
                checkpoint(
                    app,
                    "widget-target-failure-injected",
                    json!({"instanceId":id,"target":host,"targetMounted":true}),
                )?;
                return Err("EXPECTED_DESTINATION_FAILURE: 目标展示已挂载后的受控故障".to_owned());
            }
            widgets.wait_ready(id, ready_before, false)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let target = widgets.view(id)?;
        let mut target_removed = !target.visible;
        if target.visible {
            let host = if target.location == "desktop" {
                target.desktop_label.as_str()
            } else {
                PANEL
            };
            let (_, revision) = widgets.change(id, &target.location, false)?;
            if target.location == "desktop" {
                native::hide(&app.get_webview_window(host).ok_or("目标桌面容器缺失")?)?;
            }
            notify_widgets(app)?;
            widgets.wait_unmounted(host, revision)?;
            target_removed = true;
        }
        let rollback_ready = widgets.ready_count(id)?;
        widgets.change(id, &previous.location, previous.visible)?;
        notify_widgets(app)?;
        if previous.visible {
            native::show(
                &app.get_webview_window(old_host)
                    .ok_or("恢复原展示容器失败")?,
            )?;
            widgets.wait_ready(id, rollback_ready, false)?;
        }
        checkpoint(
            app,
            "widget-view-restored",
            json!({"instanceId":id,"expectedFailure":inject_failure,"reason":error,"targetUnmounted":target_removed}),
        )?;
        return Err(error);
    }
    checkpoint(
        app,
        "widget-view-changed",
        json!({"instanceId":id,"location":location,"visible":visible,"oldViewAcknowledged":previous.visible}),
    )
}
fn automatic_widgets(app: &AppHandle) -> Result<(), String> {
    let widgets = widget_state(app)?;
    let ids = widgets.ids()?;
    for id in &ids {
        widgets.wait_ready(id, 0, true)?;
    }
    let clock = ids
        .iter()
        .find(|id| widgets.view(id).is_ok_and(|v| v.desktop_label == CLOCK))
        .ok_or("时钟实例缺失")?;
    let card = ids
        .iter()
        .find(|id| widgets.view(id).is_ok_and(|v| v.desktop_label == CARD))
        .ok_or("卡片实例缺失")?;
    let hashes = ids
        .iter()
        .map(|id| widgets.settings_hash(id))
        .collect::<Result<Vec<_>, _>>()?;
    let probe = app.state::<Shared>();
    let _guard = probe.actions.lock().map_err(err)?;
    let count = widgets.ready_count(clock)?;
    change_widget_view(app, clock, "desktop", false, false)?;
    if widgets.snapshot()?["sessionCount"] != 1 {
        return Err("隐藏后会话未按实例释放".into());
    }
    change_widget_view(app, clock, "desktop", true, false)?;
    widgets.wait_ready(clock, count, false)?;
    for destination in ["panel", "desktop"] {
        let count = widgets.ready_count(card)?;
        change_widget_view(app, card, destination, true, false)?;
        widgets.wait_ready(card, count, false)?;
    }
    for (source, destination) in [("desktop", "panel"), ("panel", "desktop")] {
        if widgets.view(card)?.location != source {
            change_widget_view(app, card, source, true, false)?;
        }
        let count = widgets.ready_count(card)?;
        let error = change_widget_view(app, card, destination, true, true)
            .err()
            .ok_or("受控故障未被拒绝")?;
        if !error.starts_with("EXPECTED_DESTINATION_FAILURE:") {
            return Err(error);
        }
        widgets.wait_ready(card, count, false)?;
        if widgets.view(card)?.location != source || widgets.snapshot()?["sessionCount"] != 2 {
            return Err("切换失败后未恢复唯一主展示".into());
        }
        if destination == "desktop"
            && app
                .get_webview_window(CARD)
                .ok_or("卡片容器缺失")?
                .is_visible()
                .map_err(err)?
        {
            return Err("恢复侧栏后仍残留可见的桌面空容器".into());
        }
    }
    change_widget_view(app, card, "desktop", true, false)?;
    let count = widgets.ready_count(clock)?;
    // 真实 WM_CLOSE 路径由事件回调取得动作锁；等待时释放当前锁。
    drop(_guard);
    app.get_webview_window(CLOCK)
        .ok_or("时钟容器缺失")?
        .close()
        .map_err(err)?;
    widgets.wait_host_hidden(CLOCK)?;
    let _guard = probe.actions.lock().map_err(err)?;
    if widgets.snapshot()?["sessionCount"] != 1
        || app
            .get_webview_window(CLOCK)
            .ok_or("时钟容器缺失")?
            .is_visible()
            .map_err(err)?
    {
        return Err("系统关闭路径没有隐藏并释放目标会话".into());
    }
    widget_host_visibility(app, CLOCK, true)?;
    widgets.wait_ready(clock, count, false)?;
    let after = ids
        .iter()
        .map(|id| widgets.settings_hash(id))
        .collect::<Result<Vec<_>, _>>()?;
    if hashes != after {
        return Err("展示切换或恢复改变了实例配置".into());
    }
    let result = json!({"passed":true,"realZipAndSdk":true,"hiddenSessionReleased":true,"desktopPanelRoundtrip":true,"failedTransferRestored":true,"postMountFailureBothDirections":true,"systemCloseSessionReleased":true,"settingsPreserved":true,"validMainSessions":2});
    widgets.finish_automatic(result.clone())?;
    notify_widgets(app)?;
    checkpoint(app, "widget-automatic", result)
}
fn widget_host_visibility(app: &AppHandle, host: &str, visible: bool) -> Result<(), String> {
    let widgets = widget_state(app)?;
    for id in widgets.ids()? {
        let view = widgets.view(&id)?;
        let owner = if view.location == "desktop" {
            view.desktop_label.as_str()
        } else {
            PANEL
        };
        if owner == host && view.visible != visible {
            let before = widgets.ready_count(&id)?;
            change_widget_view(app, &id, &view.location, visible, false)?;
            if visible {
                widgets.wait_ready(&id, before, false)?;
            }
        }
    }
    Ok(())
}
fn hide_probe_host(app: &AppHandle, label: &str) -> Result<(), String> {
    let probe = app.state::<Shared>();
    if let Some(widgets) = &probe.widgets {
        if let Some(revision) = widgets.hide_host(label)? {
            native::hide(&app.get_webview_window(label).ok_or("展示容器缺失")?)?;
            notify_widgets(app)?;
            widgets.wait_unmounted(label, revision)?;
        }
    }
    native::hide(&app.get_webview_window(label).ok_or("展示容器缺失")?)?;
    checkpoint(
        app,
        "host-hidden",
        json!({"label":label,"widgetSessionsRevoked":probe.widgets.is_some()}),
    )
}

fn report_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.file_stem().is_none()
        || path.extension().and_then(|v| v.to_str()) != Some("json")
    {
        return Err("报告必须是显式绝对路径的 JSON 文件".into());
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("报告路径不能包含上级目录跳转".into());
    }
    Ok(path.with_extension(""))
}
fn authorize(label: &str, panel_only: bool) -> Result<(), String> {
    if label == PANEL || (!panel_only && [CLOCK, CARD].contains(&label)) {
        Ok(())
    } else {
        Err("CAPABILITY_DENIED: 非本次探针窗口".into())
    }
}
fn change_ratio(current: &mut PanelGeometry, area: WorkArea, ratio: f64) -> Result<(), String> {
    let next = panel_geometry(area, ratio, 280.0)?;
    *current = next;
    Ok(())
}

fn validate_tint(opacity: f64) -> Result<(), String> {
    if opacity.is_finite() && (0.0..=1.0).contains(&opacity) {
        Ok(())
    } else {
        Err("背景遮罩必须是 0 到 1 的有限数".into())
    }
}
fn validate_material_observation(
    expected_epoch: Option<u64>,
    expected_tint: Option<f64>,
    epoch: u64,
    tint: f64,
) -> Result<(), String> {
    if expected_epoch == Some(epoch) && expected_tint == Some(tint) {
        Ok(())
    } else {
        Err("STALE_APPEARANCE: 外观已变化，请按当前效果重新观察并登记".into())
    }
}
fn invalidate_material(report: &mut Value, reason: &str) {
    report["appearanceEpoch"] = json!(report["appearanceEpoch"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(1));
    report["observations"]["acrylic"] = json!({"result":"unverified","source":reason});
}

fn finish_report(report: &mut Value, remaining: &[usize], exit_code: i32) {
    report["cleanup"] = json!({"remainingOwnedHandles":remaining,"globalShortcutsRegistered":0,"normalDatabaseOpened":false,"exitCode":exit_code});
    let events = report["events"].as_array().cloned().unwrap_or_default();
    let ready = [CLOCK, CARD, PANEL].iter().all(|label| {
        events
            .iter()
            .any(|e| e["kind"] == "ready" && e["details"]["caller"] == *label)
    });
    let focus = events
        .iter()
        .any(|e| e["kind"] == "startup" && e["details"]["focusUnchanged"] == true);
    let restored = events.iter().enumerate().any(|(index, hidden)| {
        hidden["kind"] == "hide-fixtures"
            && events[index + 1..].iter().any(|shown| {
                shown["kind"] == "restore-fixtures"
                    && shown["details"]["foregroundBefore"].is_number()
                    && shown["details"]["foregroundBefore"] == shown["details"]["foregroundAfter"]
                    && [CLOCK, CARD].iter().all(|label| {
                        let (Some(a), Some(b)) =
                            (event_window(hidden, label), event_window(shown, label))
                        else {
                            return false;
                        };
                        a["visible"] == false
                            && b["visible"] == true
                            && a["hwnd"].is_u64()
                            && a["hwnd"] == b["hwnd"]
                    })
            })
    });
    let rebuilt =
        events.iter().enumerate().any(|(save_index, saved)| {
            if saved["kind"] != "ratio-saved" {
                return false;
            }
            let Some(ratio) = saved["details"]["geometry"]["requested_ratio"].as_f64() else {
                return false;
            };
            let Some(old) = event_window(saved, PANEL) else {
                return false;
            };
            events
                .iter()
                .enumerate()
                .skip(save_index + 1)
                .any(|(create_index, created)| {
                    if created["kind"] != "panel-created"
                        || created["details"]["savedRatio"].as_f64() != Some(ratio)
                    {
                        return false;
                    }
                    let Ok(area) =
                        serde_json::from_value::<WorkArea>(created["details"]["workArea"].clone())
                    else {
                        return false;
                    };
                    let Ok(expected) = panel_geometry(area, ratio, 280.0) else {
                        return false;
                    };
                    let Some(new) = event_window(created, PANEL) else {
                        return false;
                    };
                    if !new["hwnd"].is_u64()
                        || new["hwnd"] == old["hwnd"]
                        || !rect_matches(new, expected)
                    {
                        return false;
                    }
                    events.iter().enumerate().skip(create_index + 1).any(
                        |(rebuild_index, rebuild)| {
                            rebuild["kind"] == "rebuild-panel"
                                && event_window(rebuild, PANEL).is_some_and(|w| {
                                    w["hwnd"] == new["hwnd"] && rect_matches(w, expected)
                                })
                                && events[rebuild_index + 1..].iter().any(|ready| {
                                    ready["kind"] == "ready"
                                        && ready["details"]["caller"] == PANEL
                                        && event_window(ready, PANEL).is_some_and(|w| {
                                            w["hwnd"] == new["hwnd"] && rect_matches(w, expected)
                                        })
                                })
                        },
                    )
                })
        });
    let automatic = ready && focus && restored && rebuilt;
    let keys = [
        "desktop",
        "cover",
        "winD",
        "taskSwitcher",
        "input",
        "acrylic",
        "panelGeometry",
        "pin",
    ];
    let passed = exit_code == 0
        && remaining.is_empty()
        && automatic
        && report["appearanceEpoch"].is_u64()
        && report["observations"]["acrylic"]["appearanceEpoch"] == report["appearanceEpoch"]
        && (report["appearance"]["material"] == "system-acrylic"
            || (report["appearance"]["material"] == "accent-acrylic"
                && report["appearance"]["apiAccepted"] == true))
        && report["nativeErrors"]
            .as_array()
            .is_some_and(|v| v.is_empty())
        && keys
            .iter()
            .all(|key| report["observations"][key]["result"] == "pass");
    report["automaticEvidenceComplete"] = json!(automatic);
    report["platformPassed"] = json!(passed);
    report["status"] = json!(if passed {
        "platform-observed-pass"
    } else {
        "finished-with-unverified-or-failed-items"
    });
}

fn event_window<'a>(event: &'a Value, label: &str) -> Option<&'a Value> {
    event["windows"]
        .as_array()?
        .iter()
        .find(|w| w["label"] == label)
}
fn rect_matches(window: &Value, expected: PanelGeometry) -> bool {
    let r = &window["rect"];
    r["left"] == expected.left
        && r["top"] == expected.top
        && r["width"] == expected.width
        && r["height"] == expected.height
}
fn attachment_valid(
    alive: bool,
    child: bool,
    parent_matches: bool,
    topmost: bool,
    pinned: bool,
) -> Result<(), String> {
    if alive && parent_matches && child != pinned && topmost == pinned {
        Ok(())
    } else {
        Err("原生窗口存活、父窗口或层级与当前展示意图不一致".into())
    }
}

struct Probe {
    path: PathBuf,
    root: PathBuf,
    token: String,
    report: Mutex<Value>,
    geometry: Mutex<Option<PanelGeometry>>,
    actions: Mutex<()>,
    quitting: AtomicBool,
    startup_complete: AtomicBool,
    card_pinned: AtomicBool,
    tint_opacity: Mutex<f64>,
    handles: Mutex<Vec<usize>>,
    resource_known: Mutex<std::collections::BTreeMap<u32, u64>>,
    resource_previous: Mutex<Option<(std::time::Instant, Vec<Value>)>>,
    widgets: Option<Arc<crate::desktop_widgets::WidgetProbe>>,
}
type Shared = Arc<Probe>;

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let temporary = path
        .parent()
        .ok_or("保存路径缺少父目录")?
        .join(format!(".souldesk-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(err)?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(err)?)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(err)
}

// 调用者持有窗口动作锁；先停采样，再在事件循环仍工作时销毁并确认本次窗口。
fn finish_windows(app: &AppHandle, mut code: i32) {
    let probe = app.state::<Shared>();
    if probe.quitting.swap(true, Ordering::AcqRel) {
        return;
    }
    for label in app.webview_windows().into_keys() {
        if let Err(e) = crate::windows::destroy_window(app, &label) {
            failure(app, &e);
            code = 1;
        }
    }
    if let Some(widgets) = &probe.widgets {
        if let Err(e) = widgets.clear_sessions() {
            failure(app, &e);
            code = 1;
        }
    }
    if let Err(e) = checkpoint(app, "windows-destroyed", json!({"exitCode":code})) {
        eprintln!("退出报告写入失败：{e}");
        code = 1;
    }
    app.exit(code);
}

fn checkpoint(app: &AppHandle, kind: &str, mut details: Value) -> Result<(), String> {
    let probe = app.state::<Shared>();
    if kind == "sample" {
        let sampled = (|| -> Result<(), String> {
            let processes =
                crate::view_probe::processes(&mut *probe.resource_known.lock().map_err(err)?)?;
            let now = std::time::Instant::now();
            let mut previous = probe.resource_previous.lock().map_err(err)?;
            if let Some((time, before)) = previous.as_ref() {
                details["cpuPercentOneCore"] = json!(crate::view_probe::cpu_percent(
                    before,
                    &processes,
                    now.duration_since(*time).as_millis() as u64
                )?);
            }
            details["privateMiB"] = json!(
                processes
                    .iter()
                    .filter_map(|p| p["privateBytes"].as_u64())
                    .sum::<u64>() as f64
                    / 1048576.0
            );
            details["processes"] = json!(processes);
            *previous = Some((now, processes));
            Ok(())
        })();
        if let Err(error) = sampled {
            details["resourceError"] = json!(error);
        }
    }
    let windows: Vec<_> = app
        .webview_windows()
        .values()
        .map(|w| native::describe(w))
        .collect();
    let widget_evidence = probe.widgets.as_ref().map(|w| w.evidence()).transpose()?;
    let mut report = probe.report.lock().map_err(err)?;
    if let Some(evidence) = widget_evidence {
        report["widgetProbe"] = evidence;
    }
    let appearance = report["appearance"].clone();
    let appearance_epoch = report["appearanceEpoch"].clone();
    let events = report["events"].as_array_mut().ok_or("探针记录结构损坏")?;
    if events.len() < 500 {
        events.push(json!({"timeMs":crate::store::now_ms(),"kind":kind,"details":details,"appearance":appearance,"appearanceEpoch":appearance_epoch,"foreground":native::foreground(),"windows":windows}));
    }
    report["windows"] = json!(windows);
    report["windowCount"] = json!(app.webview_windows().len());
    report["geometry"] = json!(*probe.geometry.lock().map_err(err)?);
    write_json(&probe.path, &report)
}

fn failure(app: &AppHandle, error: &str) {
    eprintln!("桌面探针：{error}");
    if let Ok(mut report) = app.state::<Shared>().report.lock() {
        if let Some(errors) = report["nativeErrors"].as_array_mut() {
            errors.push(json!(error));
        }
    }
    if let Err(e) = checkpoint(app, "error", json!(error)) {
        eprintln!("探针报告保存失败：{e}");
    }
}

fn work_area(app: &AppHandle) -> Result<WorkArea, String> {
    let current = app
        .get_webview_window(PANEL)
        .map(|w| w.current_monitor())
        .transpose()
        .map_err(err)?
        .flatten();
    let monitor = current
        .or(app.primary_monitor().map_err(err)?)
        .ok_or("没有可用显示器")?;
    let area = monitor.work_area();
    Ok(WorkArea {
        left: area.position.x,
        top: area.position.y,
        width: area.size.width,
        height: area.size.height,
        scale_factor: monitor.scale_factor(),
    })
}

fn saved_ratio(root: &Path) -> Result<f64, String> {
    let path = root.join("panel.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(err)?["ratio"]
            .as_f64()
            .ok_or("已保存的比例无效".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0.4),
        Err(e) => Err(err(e)),
    }
}

fn build_window(
    app: &AppHandle,
    label: &str,
    width: f64,
    height: f64,
) -> Result<WebviewWindow, String> {
    let probe = app.state::<Shared>();
    let kind = if label.starts_with("desktop-probe-trial-") {
        "trial"
    } else if label == CLOCK {
        "clock"
    } else if label == CARD {
        "card"
    } else {
        "panel"
    };
    let directory = probe.root.join("webview").join(label);
    let material = probe.report.lock().map_err(err)?["appearance"]["material"]
        .as_str()
        .unwrap_or("system-acrylic")
        .to_owned();
    if label == PANEL
        && !["accent-acrylic", "system-acrylic", "transparent-baseline"]
            .contains(&material.as_str())
    {
        return Err("当前材质状态不确定，请先重新选择材质".into());
    }
    std::fs::create_dir_all(&directory).map_err(err)?;
    let mut builder = WebviewWindowBuilder::new(app,label,WebviewUrl::App(format!("index.html?desktopProbe={kind}&instance={label}").into()))
        .title(format!("SoulDesk 桌面探针 · {kind}"))
        .inner_size(width,height).decorations(false).resizable(false).shadow(false)
        .skip_taskbar(true).visible(false).focused(false).transparent(true).theme(Some(tauri::Theme::Dark))
        .background_color(tauri::window::Color(0,0,0,0)).data_directory(directory)
        .initialization_script(format!("if(window===window.top)Object.defineProperty(window,'__SOULDESK_HOST_TOKEN__',{{value:{}}});",json!(probe.token)))
        .initialization_script_for_all_frames("if(window!==window.top)document.addEventListener('pointerdown',event=>{if(event.isTrusted&&event.button===0){const origin=new URLSearchParams(location.search).get('hostOrigin');if(origin)parent.postMessage({type:'souldesk:user-pointer',protocol:1},origin);}},true);");
    if label == PANEL {
        builder = builder.always_on_top(true);
        if material == "system-acrylic" {
            use tauri::window::{Effect, EffectsBuilder};
            builder = builder.effects(EffectsBuilder::new().effect(Effect::Acrylic).build());
        }
    }
    let window = builder.build().map_err(err)?;
    native::prepare(&window)?;
    probe
        .handles
        .lock()
        .map_err(err)?
        .push(native::handle(&window)?);
    if label == PANEL && material == "accent-acrylic" {
        native::persistent_acrylic(&window)?;
        probe.report.lock().map_err(err)?["appearance"]["apiAccepted"] = json!(true);
    }
    Ok(window)
}

fn build_panel(app: &AppHandle, area: WorkArea) -> Result<(), String> {
    let probe = app.state::<Shared>();
    let ratio = saved_ratio(&probe.root)?;
    let geometry = panel_geometry(area, ratio, 280.0)?;
    let window = build_window(
        app,
        PANEL,
        geometry.width as f64 / area.scale_factor,
        geometry.height as f64 / area.scale_factor,
    )?;
    native::place(
        &window,
        geometry.left,
        geometry.top,
        geometry.width,
        geometry.height,
        true,
    )?;
    *probe.geometry.lock().map_err(err)? = Some(geometry);
    let appearance = {
        let mut report = probe.report.lock().map_err(err)?;
        invalidate_material(&mut report, "panel-created");
        report["appearance"].clone()
    };
    checkpoint(
        app,
        "panel-created",
        json!({"workArea":area,"savedRatio":ratio,"appearance":appearance}),
    )
}

fn start_windows(app: &AppHandle) -> Result<(), String> {
    let probe = app.state::<Shared>();
    if probe.widgets.is_some() {
        load_widget_packages(app)?;
    }
    let _guard = probe.actions.lock().map_err(err)?;
    let area = work_area(app)?;
    let before = native::foreground();
    let real = probe.widgets.is_some();
    for (label, y) in [(CLOCK, 32.0), (CARD, if real { 390.0 } else { 270.0 })] {
        let window = build_window(
            app,
            label,
            if real { 400.0 } else { 300.0 },
            if real { 320.0 } else { 208.0 },
        )?;
        native::attach(
            &window,
            area.left + (32.0 * area.scale_factor) as i32,
            area.top + (y * area.scale_factor) as i32,
        )?;
    }
    build_panel(app, area)?;
    checkpoint(
        app,
        "startup",
        json!({"foregroundBefore":before,"foregroundAfter":native::foreground(),"focusUnchanged":before==native::foreground()}),
    )
}

#[tauri::command]
async fn desktop_probe_set_ratio(
    window: WebviewWindow,
    app: AppHandle,
    ratio: f64,
) -> Result<PanelGeometry, String> {
    authorize(window.label(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
        let probe = app.state::<Shared>();
        let _guard = probe.actions.lock().map_err(err)?;
        let result = (|| -> Result<PanelGeometry, String> {
            let area = work_area(&app)?;
            let previous = probe.geometry.lock().map_err(err)?.ok_or("面板未初始化")?;
            let mut next = previous;
            change_ratio(&mut next, area, ratio)?;
            native::place(&window, next.left, next.top, next.width, next.height, true)?;
            if let Err(error) = write_json(&probe.root.join("panel.json"), &json!({"ratio":ratio}))
            {
                let rollback = native::place(
                    &window,
                    previous.left,
                    previous.top,
                    previous.width,
                    previous.height,
                    true,
                );
                return Err(format!("比例保存失败：{error}；恢复原几何：{rollback:?}"));
            }
            *probe.geometry.lock().map_err(err)? = Some(next);
            checkpoint(
                &app,
                "ratio-saved",
                json!({"workArea":area,"geometry":next}),
            )?;
            Ok(next)
        })();
        if let Err(error) = &result {
            failure(&app, error);
        }
        result
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn desktop_probe_set_tint(
    window: WebviewWindow,
    app: AppHandle,
    opacity: f64,
) -> Result<Value, String> {
    authorize(window.label(), true)?;
    validate_tint(opacity)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = app.state::<Shared>();
        let _guard = p.actions.lock().map_err(err)?;
        let changed = {
            let mut tint = p.tint_opacity.lock().map_err(err)?;
            let changed = *tint != opacity;
            *tint = opacity;
            changed
        };
        let epoch;
        {
            let mut report = p.report.lock().map_err(err)?;
            report["appearance"]["tintOpacity"] = json!(opacity);
            if changed {
                invalidate_material(&mut report, "appearance-changed");
            }
            epoch = report["appearanceEpoch"].as_u64().unwrap_or(0);
        }
        let result = checkpoint(&app, "tint-changed", json!({"tintOpacity":opacity}));
        if let Err(error) = &result {
            failure(&app, error);
        }
        result?;
        Ok(json!({"opacity":opacity,"appearanceEpoch":epoch,"changed":changed}))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn desktop_probe_action(
    window: WebviewWindow,
    app: AppHandle,
    action: String,
    surface: Option<Value>,
) -> Result<Value, String> {
    authorize(window.label(), false)?;
    if !["ready", "open-panel", "snapshot"].contains(&action.as_str()) {
        authorize(window.label(), true)?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let probe = app.state::<Shared>();
        let _guard = probe.actions.lock().map_err(err)?;
        let result = (|| -> Result<Value,String> {
        let before = native::foreground();
        match action.as_str() {
            "ready" => {},
            "snapshot" => {},
            "surface" => {
                let data=surface.ok_or("缺少透明层诊断")?;
                if data.to_string().len()>4096 {return Err("透明层诊断超出大小限制".into());}
                probe.report.lock().map_err(err)?["surface"]=data;
            },
            "transparent-baseline" | "restore-material" => {
                let transparent=action=="transparent-baseline";
                let previous=probe.report.lock().map_err(err)?["appearance"]["material"].as_str().unwrap_or("unknown").to_owned();
                {
                    let mut report=probe.report.lock().map_err(err)?;
                    invalidate_material(&mut report,"material-changing");
                    report["appearance"]["material"]=json!("unknown");
                    report["appearance"]["requestedMaterial"]=json!(if transparent{"transparent-baseline"}else{"system-acrylic"});
                }
                if previous=="accent-acrylic" {native::clear_accent(&window)?;}
                if transparent {window.set_effects(None).map_err(err)?;}
                else {
                    use tauri::window::{EffectsBuilder,Effect};
                    window.set_effects(EffectsBuilder::new().effect(Effect::Acrylic).build()).map_err(err)?;
                }
                let opacity=if transparent {0.0}else{0.28};
                *probe.tint_opacity.lock().map_err(err)?=opacity;
                let mut report=probe.report.lock().map_err(err)?;
                report["appearance"]=json!({"material":if transparent{"transparent-baseline"}else{"system-acrylic"},"tintOpacity":opacity});
            },
            "persistent-material" => {
                {
                    let mut report=probe.report.lock().map_err(err)?;
                    invalidate_material(&mut report,"material-changing");
                    report["appearance"]["material"]=json!("unknown");
                    report["appearance"]["requestedMaterial"]=json!("accent-acrylic");
                }
                native::persistent_acrylic(&window)?;
                *probe.tint_opacity.lock().map_err(err)?=0.12;
                let mut report=probe.report.lock().map_err(err)?;
                report["appearance"]=json!({"material":"accent-acrylic","tintOpacity":0.12,"apiAccepted":true,"undocumented":true});
            },
            "open-panel" => {
                if let Some(panel) = app.get_webview_window(PANEL) { native::show(&panel)?; }
                else { build_panel(&app,work_area(&app)?)?; }
                if probe.widgets.is_some() { widget_host_visibility(&app,PANEL,true)?; }
            },
            "hide-panel" => { hide_probe_host(&app,window.label())?; },
            "rebuild-panel" => {
                let material=probe.report.lock().map_err(err)?["appearance"]["material"].as_str().unwrap_or("unknown").to_owned();
                if !["accent-acrylic","system-acrylic","transparent-baseline"].contains(&material.as_str()) {return Err("材质状态不确定，先重新选择材质再重建".into());}
                let area = work_area(&app)?;
                if let Some(widgets)=&probe.widgets {widgets.revoke_host(PANEL)?;}
                crate::windows::destroy_window(&app,PANEL)?;
                build_panel(&app,area)?;
            },
            "hide-fixtures" | "restore-fixtures" => {
                for label in [CLOCK,CARD] {
                    if probe.widgets.is_some() {
                        widget_host_visibility(&app,label,action=="restore-fixtures")?;
                        continue;
                    }
                    let w = app.get_webview_window(label).ok_or("夹具窗口缺失")?;
                    if action == "hide-fixtures" { native::hide(&w)?; } else { native::show(&w)?; }
                }
            },
            "widget-move-clock" | "widget-move-card" | "widget-test-failure" => {
                let widgets=widget_state(&app)?;
                let label=if action=="widget-move-clock"{CLOCK}else{CARD};
                let id=widgets.ids()?.into_iter().find(|id|widgets.view(id).is_ok_and(|v|v.desktop_label==label)).ok_or("测试实例缺失")?;
                let view=widgets.view(&id)?;
                let target=if view.location=="desktop"{"panel"}else{"desktop"};
                let before=widgets.ready_count(&id)?;
                if action=="widget-test-failure" {
                    let error=change_widget_view(&app,&id,target,true,true).err().ok_or("受控故障未被拒绝")?;
                    if !error.starts_with("EXPECTED_DESTINATION_FAILURE:"){return Err(error);}
                } else {change_widget_view(&app,&id,target,true,false)?;}
                widgets.wait_ready(&id,before,false)?;
            },
            "toggle-pin" => {
                if let Some(widgets)=&probe.widgets {
                    let desktop=widgets.ids()?.iter().any(|id|widgets.view(id).is_ok_and(|v|v.desktop_label==CARD&&v.location=="desktop"&&v.visible));
                    if !desktop {return Err("请先将卡片恢复到桌面，再切换置顶".into());}
                }
                let w = app.get_webview_window(CARD).ok_or("卡片窗口缺失")?;
                let pinned=native::toggle_pin(&w)?;
                probe.card_pinned.store(pinned,Ordering::Release);
            },
            "finish" => {
                checkpoint(&app,"finish-requested",json!({"foregroundBefore":before}))?;
                finish_windows(&app,0);
                return Ok(Value::Null);
            },
            _ => return Err("未知探针操作".into()),
        }
        checkpoint(&app,&action,json!({"caller":window.label(),"foregroundBefore":before,"foregroundAfter":native::foreground()}))?;
        let geometry = *probe.geometry.lock().map_err(err)?;
        let observations = probe.report.lock().map_err(err)?["observations"].clone();
        let tint_opacity=*probe.tint_opacity.lock().map_err(err)?;
        let epoch=probe.report.lock().map_err(err)?["appearanceEpoch"].as_u64().unwrap_or(0);
        let appearance=probe.report.lock().map_err(err)?["appearance"].clone();
        let widgets=probe.widgets.as_ref().map(|w|w.snapshot()).transpose()?;
        Ok(json!({"geometry":geometry,"windowCount":app.webview_windows().len(),"report":probe.path,"observations":observations,"tintOpacity":tint_opacity,"appearanceEpoch":epoch,"appearance":appearance,"widgets":widgets}))
        })();
        if let Err(error)=&result { failure(&app,error); }
        result
    }).await.map_err(err)?
}

#[tauri::command]
async fn desktop_widget_activate(window: WebviewWindow, app: AppHandle) -> Result<Value, String> {
    if ![CLOCK, CARD, PANEL].contains(&window.label()) {
        return Err("CAPABILITY_DENIED: 非组件展示容器".into());
    }
    if window.label() == PANEL {
        return Ok(json!({"attempted":false,"reason":"top-level-panel-uses-normal-activation"}));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let probe = app.state::<Shared>();
        let _guard = probe.actions.lock().map_err(err)?;
        if probe.quitting.load(Ordering::Acquire) {
            return Err("探针正在结束".into());
        }
        let result = native::activate_clicked(&window)?;
        checkpoint(
            &app,
            "desktop-click-input",
            json!({"label":window.label(),"result":result}),
        )?;
        Ok(result)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn desktop_probe_observe(
    window: WebviewWindow,
    app: AppHandle,
    key: String,
    result: String,
    expected_epoch: Option<u64>,
    expected_tint: Option<f64>,
) -> Result<(), String> {
    authorize(window.label(), true)?;
    if ![
        "desktop",
        "cover",
        "winD",
        "taskSwitcher",
        "input",
        "acrylic",
        "panelGeometry",
        "pin",
        "widgets",
        "widgetInput",
    ]
    .contains(&key.as_str())
        || !["pass", "fail", "unverified"].contains(&result.as_str())
    {
        return Err("观察项目或结果无效".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let p=app.state::<Shared>();
        let _guard=p.actions.lock().map_err(err)?;
        let tint=*p.tint_opacity.lock().map_err(err)?;
        {
            let mut report=p.report.lock().map_err(err)?;
            let epoch=report["appearanceEpoch"].as_u64().unwrap_or(0);
            if key=="acrylic" {
                validate_material_observation(expected_epoch,expected_tint,epoch,tint)?;
                if !["system-acrylic","accent-acrylic"].contains(&report["appearance"]["material"].as_str().unwrap_or("unknown")) {return Err("当前不在可确认的磨砂状态".into());}
            }
            report["observations"][&key]=json!({"result":result,"source":"manual-ui","timeMs":crate::store::now_ms(),"appearanceEpoch":epoch,"tintOpacity":tint});
        }
        checkpoint(&app,"manual-observation",json!({"key":key,"result":result}))
    }).await.map_err(err)?
}

#[cfg(windows)]
pub fn run(path: PathBuf, packages: Option<[PathBuf; 2]>) -> Result<(), String> {
    if tauri::is_dev() {
        return Err(
            "探针必须使用嵌入资源：先 build:web，再运行 node scripts/desktop.mjs desktop".into(),
        );
    }
    let root = report_root(&path)?;
    if path.exists() || root.exists() {
        return Err("报告或隔离目录已存在，请使用新的运行标识".into());
    }
    std::fs::create_dir_all(path.parent().ok_or("报告缺少父目录")?).map_err(err)?;
    // 只创建本次专属目录，不打开正常应用数据根目录。
    std::fs::create_dir(&root).map_err(err)?;
    let real_widgets = packages.is_some();
    let widget_probe = packages
        .map(|paths| {
            crate::desktop_widgets::WidgetProbe::new(root.join("widget-data"), paths).map(Arc::new)
        })
        .transpose()?;
    let probe = Arc::new(Probe {
        path,
        root,
        token: uuid::Uuid::new_v4().to_string(),
        geometry: Mutex::new(None),
        actions: Mutex::new(()),
        quitting: AtomicBool::new(false),
        startup_complete: AtomicBool::new(false),
        card_pinned: AtomicBool::new(false),
        tint_opacity: Mutex::new(0.12),
        handles: Mutex::new(Vec::new()),
        resource_known: Mutex::new(Default::default()),
        resource_previous: Mutex::new(None),
        widgets: widget_probe,
        report: Mutex::new(
            json!({"runId":uuid::Uuid::new_v4(),"phase":if real_widgets{"desktop-widget-probe"}else{"desktop-platform-probe"},"content":if real_widgets{"external-clock-card-zip"}else{"static-fixtures-not-widget-packages"},"normalDatabaseOpened":false,"system":native::system(),"events":[],"nativeErrors":[],"observations":{},"appearance":{"material":"accent-acrylic","tintOpacity":0.12,"apiAccepted":false,"undocumented":true},"platformPassed":false,"status":"starting"}),
        ),
    });
    let mut context = tauri::generate_context!();
    // 启动脚本启用 tauri/custom-protocol 嵌入已构建资源，探针不依赖开发服务器。
    context.config_mut().build.dev_url = None;
    let app = tauri::Builder::default()
        .manage(probe.clone())
        .register_uri_scheme_protocol("widget", |ctx, request| {
            let result = widget_state(ctx.app_handle()).and_then(|w| crate::widgets::resource(&w.store, request.uri().path()));
            let (status, data, mime) = match result { Ok((b,m)) => (200,b,m), Err(_) => (404,b"component resource unavailable".to_vec(),"text/plain".into()) };
            tauri::http::Response::builder().status(status).header("Content-Type",mime).header("X-Content-Type-Options","nosniff")
                .header("Content-Security-Policy","default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
                .header("Cache-Control","no-store").body(data).unwrap()
        })
        .invoke_handler(|invoke| {
            let allowed = invoke
                .message
                .webview_ref()
                .try_state::<Shared>()
                .is_some_and(|p| {
                    invoke
                        .message
                        .headers()
                        .get("x-souldesk-host")
                        .and_then(|v| v.to_str().ok())
                        == Some(p.token.as_str())
                });
            if !allowed {
                invoke
                    .resolver
                    .reject("CAPABILITY_DENIED: 仅本次探针顶层容器可调用");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                desktop_probe_set_ratio,
                desktop_probe_set_tint,
                desktop_probe_action,
                desktop_probe_observe,
                crate::desktop_widgets::desktop_widget_open_session,
                crate::desktop_widgets::desktop_widget_close_session,
                crate::desktop_widgets::desktop_widget_widget_request,
                crate::desktop_widgets::desktop_widget_trial_failed,
                crate::desktop_widgets::desktop_widget_view,
                desktop_widget_activate
            ];
            handler(invoke)
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let monitors = app.available_monitors()?;
            {
                let p = app.state::<Shared>();
                let mut report = p.report.lock().map_err(err)?;
                report["monitors"] = json!(monitors);
                report["dataRoot"] = json!(p.root);
                report["webviewRoot"] = json!(p.root.join("webview"));
            }
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = start_windows(&handle) {
                    failure(&handle, &e);
                    let p = handle.state::<Shared>();
                    let _guard = p.actions.lock();
                    finish_windows(&handle, 1);
                } else {
                    handle.state::<Shared>().startup_complete.store(true, Ordering::Release);
                    if handle.state::<Shared>().widgets.is_none() { return; }
                    if let Err(e) = automatic_widgets(&handle) {
                        if let Ok(widgets) = widget_state(&handle) {
                            let _ = widgets.finish_automatic(json!({"passed":false,"error":e}));
                        }
                        failure(&handle, &e);
                        let p = handle.state::<Shared>();
                        let _guard = p.actions.lock();
                        finish_windows(&handle, 1);
                    }
                }
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // 只枚举本进程窗口，不注册系统钩子或全局快捷键。
                for tick in 0..1800 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if handle.state::<Shared>().quitting.load(Ordering::Acquire) {
                        return;
                    }
                    if tick % 10 == 0 {
                        if let Err(e) =
                            checkpoint(&handle, "sample", json!({"elapsedSeconds":tick+1}))
                        {
                            failure(&handle, &e);
                        }
                    }
                    let app = handle.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        let p = app.state::<Shared>();
                        let Ok(_guard) = p.actions.try_lock() else {
                            return;
                        };
                        if p.quitting.load(Ordering::Acquire) {
                            return;
                        }
                        if !p.startup_complete.load(Ordering::Acquire) {
                            return;
                        }
                        let result = (|| -> Result<(), String> {
                            let previous = *p.geometry.lock().map_err(err)?;
                            if let (Some(panel), Some(previous)) =
                                (app.get_webview_window(PANEL), previous)
                            {
                                let next = panel_geometry(
                                    work_area(&app)?,
                                    previous.requested_ratio,
                                    280.0,
                                )?;
                                if previous != next {
                                    native::place(
                                        &panel,
                                        next.left,
                                        next.top,
                                        next.width,
                                        next.height,
                                        false,
                                    )?;
                                    *p.geometry.lock().map_err(err)? = Some(next);
                                    checkpoint(&app, "display-recalculated", json!(next))?;
                                }
                            }
                            for label in [CLOCK, CARD] {
                                let w = app
                                    .get_webview_window(label)
                                    .ok_or_else(|| format!("{label} 窗口意外消失"))?;
                                native::check_attachment(
                                    &w,
                                    label == CARD && p.card_pinned.load(Ordering::Acquire),
                                )?;
                            }
                            Ok(())
                        })();
                        if let Err(e) = result {
                            failure(&app, &e);
                            finish_windows(&app, 1);
                        }
                    })
                    .await
                    .ok();
                }
                failure(&handle, "探针达到 30 分钟上限，未确认项目保持未验证");
                tauri::async_runtime::spawn_blocking(move || {
                    let p = handle.state::<Shared>();
                    let _guard = p.actions.lock();
                    finish_windows(&handle, 0);
                });
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                if window.label() == PANEL {
                    let app = window.app_handle().clone();
                    let probe = app.state::<Shared>();
                    // 事件循环上只尝试读取；不等待动作锁、不修改材质或窗口焦点。
                    let appearance = probe.report.try_lock().ok().map(|report| {
                        json!({"appearance":report["appearance"],"appearanceEpoch":report["appearanceEpoch"]})
                    });
                    let details = json!({"focused":focused,"eventTimeMs":crate::store::now_ms(),"foregroundAtEvent":native::foreground(),"stateAtEvent":appearance});
                    tauri::async_runtime::spawn_blocking(move || {
                        let probe = app.state::<Shared>();
                        let Ok(_guard) = probe.actions.lock() else { return; };
                        if !probe.quitting.load(Ordering::Acquire) {
                            if let Err(e) = checkpoint(&app, "panel-focus", details) {
                                failure(&app, &e);
                            }
                        }
                    });
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                let label = window.label().to_owned();
                tauri::async_runtime::spawn_blocking(move || {
                    let probe = app.state::<Shared>();
                    let Ok(_guard) = probe.actions.lock() else { return; };
                    if !probe.quitting.load(Ordering::Acquire) {
                        if let Err(e) = hide_probe_host(&app, &label) { failure(&app,&e); }
                    }
                });
            }
        })
        .build(context)
        .map_err(err)?;
    let exit_code = app.run_return(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() || !app.state::<Shared>().quitting.load(Ordering::Acquire) {
                api.prevent_exit();
            }
        }
    });
    let mut report = probe.report.lock().map_err(err)?;
    let live: Vec<_> = probe
        .handles
        .lock()
        .map_err(err)?
        .iter()
        .copied()
        .filter(|h| native::owned_alive(*h))
        .collect();
    finish_report(&mut report, &live, exit_code);
    if real_widgets {
        let passed = report["widgetProbe"]["snapshot"]["automatic"]["passed"] == true
            && report["observations"]["widgets"]["result"] == "pass"
            && report["observations"]["widgetInput"]["result"] == "pass"
            && live.is_empty()
            && exit_code == 0
            && report["nativeErrors"]
                .as_array()
                .is_some_and(|e| e.is_empty())
            && report["widgetProbe"]["snapshot"]["sessionCount"] == 0;
        report["widgetIntegrationPassed"] = json!(passed);
        report["status"] = json!(if passed {
            "widget-integration-observed-pass"
        } else {
            "widget-integration-incomplete-or-failed"
        });
    }
    write_json(&probe.path, &report)?;
    if exit_code != 0 {
        return Err(format!(
            "探针以 {exit_code} 结束，详见报告 {}",
            probe.path.display()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn run(_path: PathBuf, _packages: Option<[PathBuf; 2]>) -> Result<(), String> {
    Err("桌面平台探针仅支持 Windows".into())
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT},
        Graphics::Gdi::ScreenToClient,
        UI::{
            HiDpi::{GetDpiForWindow, GetWindowDpiAwarenessContext},
            WindowsAndMessaging::*,
        },
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    pub fn handle(w: &WebviewWindow) -> Result<usize, String> {
        Ok(w.hwnd().map_err(err)?.0 as usize)
    }
    pub fn foreground() -> usize {
        unsafe { GetForegroundWindow() as usize }
    }
    fn keyboard_state(h: HWND) -> Value {
        let query = |thread| {
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let ok = unsafe { GetGUIThreadInfo(thread, &mut info) };
            if ok == 0 {
                return json!({"thread":thread,"querySucceeded":false});
            }
            json!({"thread":thread,"querySucceeded":true,"active":info.hwndActive as usize,"focus":info.hwndFocus as usize,"focusClass":class(info.hwndFocus),"focusPid":pid(info.hwndFocus),"focusWithinHost":info.hwndFocus==h||unsafe{IsChild(h,info.hwndFocus)}!=0})
        };
        json!({"foreground":foreground(),"foregroundQueue":query(0),"hostQueue":query(unsafe{GetWindowThreadProcessId(h,std::ptr::null_mut())})})
    }
    fn activate_click(h: HWND) -> Result<Value, String> {
        use windows_sys::Win32::{
            System::Threading::GetCurrentThreadId,
            UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SetFocus, VK_LBUTTON, VK_RBUTTON},
        };
        let skip = |reason| Ok(json!({"attempted":false,"reason":reason}));
        unsafe {
            if IsWindow(h) == 0
                || pid(h) != std::process::id()
                || IsWindowVisible(h) == 0
                || GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD == 0
            {
                return skip("not-visible-owned-desktop-child");
            }
            if GetWindowThreadProcessId(h, std::ptr::null_mut()) != GetCurrentThreadId() {
                return Err("点击激活必须在窗口所属线程执行".into());
            }
            let physical = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
                VK_RBUTTON
            } else {
                VK_LBUTTON
            };
            if GetAsyncKeyState(physical as i32) >= 0 {
                return skip("pointer-no-longer-down");
            }
            let mut point = POINT::default();
            if GetCursorPos(&mut point) == 0 {
                return Err(winerr("读取点击位置失败"));
            }
            let hit = WindowFromPoint(point);
            if hit != h && IsChild(h, hit) == 0 {
                return skip("pointer-outside-this-widget");
            }
            let root = GetAncestor(h, GA_ROOT);
            if root.is_null() || root != GetAncestor(desktop()?, GA_ROOT) {
                return skip("desktop-parent-mismatch");
            }
            let before = keyboard_state(h);
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(GetCurrentThreadId(), &mut info) == 0 {
                return Err(winerr("读取组件键盘焦点失败"));
            }
            let target = info.hwndFocus;
            if target != h && IsChild(h, target) == 0 {
                return skip("widget-has-not-received-native-focus");
            }
            let already_foreground = GetForegroundWindow() == root;
            let accepted = already_foreground || SetForegroundWindow(root) != 0;
            if accepted {
                SetFocus(target);
            }
            let after = keyboard_state(h);
            Ok(
                json!({"attempted":true,"accepted":accepted,"alreadyForeground":already_foreground,"root":root as usize,"before":before,"after":after,"keyboardInWidget":after["foregroundQueue"]["focusWithinHost"]==true}),
            )
        }
    }
    pub fn activate_clicked(w: &WebviewWindow) -> Result<Value, String> {
        let raw = handle(w)?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        w.run_on_main_thread(move || {
            let _ = sender.send(activate_click(raw as HWND));
        })
        .map_err(err)?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(err)?
    }
    #[cfg(test)]
    #[test]
    fn hidden_widget_cannot_request_foreground_activation() {
        struct Owned(HWND);
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let h = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("SoulDesk hidden input boundary").as_ptr(),
                WS_POPUP,
                0,
                0,
                120,
                80,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!h.is_null());
        let owned = Owned(h);
        let before = foreground();
        assert_eq!(activate_click(owned.0).unwrap()["attempted"], false);
        assert_eq!(
            activate_click(std::ptr::null_mut()).unwrap()["attempted"],
            false
        );
        assert_eq!(foreground(), before);
        assert_eq!(unsafe { IsWindowVisible(owned.0) }, 0);
    }
    pub fn verify_dpi(child: usize, parent: usize) -> Result<(), String> {
        use windows_sys::Win32::UI::HiDpi::AreDpiAwarenessContextsEqual;
        if child == 0
            || parent == 0
            || unsafe { AreDpiAwarenessContextsEqual(child as _, parent as _) } == 0
        {
            Err(format!(
                "桌面与探针 DPI 模式不兼容：child={child},parent={parent}；拒绝跨进程附着"
            ))
        } else {
            Ok(())
        }
    }
    pub fn prepare(w: &WebviewWindow) -> Result<(), String> {
        let h = handle(w)? as HWND;
        unsafe {
            let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            windows_sys::Win32::Foundation::SetLastError(0);
            let old = SetWindowLongPtrW(
                h,
                GWL_EXSTYLE,
                ((style & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW) as isize,
            );
            if old == 0 && windows_sys::Win32::Foundation::GetLastError() != 0 {
                return Err(winerr("设置工具窗口身份失败"));
            }
        }
        Ok(())
    }
    fn class(h: HWND) -> String {
        let mut v = [0u16; 128];
        let n = unsafe { GetClassNameW(h, v.as_mut_ptr(), 128) };
        String::from_utf16_lossy(&v[..n.max(0) as usize])
    }
    fn pid(h: HWND) -> u32 {
        let mut p = 0;
        unsafe { GetWindowThreadProcessId(h, &mut p) };
        p
    }
    fn winerr(operation: &str) -> String {
        format!("{operation}：{}", std::io::Error::last_os_error())
    }
    fn desktop() -> Result<HWND, String> {
        unsafe extern "system" fn find(h: HWND, data: LPARAM) -> i32 {
            let shell = GetShellWindow();
            if pid(h) == pid(shell)
                && ["Progman", "WorkerW"].contains(&class(h).as_str())
                && !FindWindowExW(
                    h,
                    std::ptr::null_mut(),
                    wide("SHELLDLL_DefView").as_ptr(),
                    std::ptr::null(),
                )
                .is_null()
            {
                *(data as *mut usize) = h as usize;
                0
            } else {
                1
            }
        }
        let mut result = 0usize;
        unsafe { EnumWindows(Some(find), &mut result as *mut usize as LPARAM) };
        if result == 0 {
            Err("未找到 Explorer 桌面视图的宿主；未改用浮窗".into())
        } else {
            Ok(result as HWND)
        }
    }
    fn bind_child(h: HWND, parent: HWND) -> Result<(), String> {
        unsafe {
            let child_dpi = GetWindowDpiAwarenessContext(h);
            let parent_dpi = GetWindowDpiAwarenessContext(parent);
            // 不改变进程 DPI 模式；句柄与缩放记录供判断跨进程附着风险。
            verify_dpi(child_dpi as usize, parent_dpi as usize)?;
            // 必须在仍是顶层窗口时移除置顶；转成 WS_CHILD 后再调用不会清掉该标志。
            if GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD == 0
                && SetWindowPos(
                    h,
                    HWND_NOTOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                ) == 0
            {
                return Err(winerr("恢复桌面非置顶层级失败"));
            }
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            windows_sys::Win32::Foundation::SetLastError(0);
            let old = SetWindowLongPtrW(h, GWL_STYLE, ((style & !WS_POPUP) | WS_CHILD) as isize);
            if old == 0 && windows_sys::Win32::Foundation::GetLastError() != 0 {
                return Err(winerr("设置子窗口样式失败"));
            }
            SetParent(h, parent);
            if GetParent(h) != parent {
                return Err(winerr("桌面附着失败"));
            }
        }
        Ok(())
    }
    pub fn attach(w: &WebviewWindow, x: i32, y: i32) -> Result<(), String> {
        let h = handle(w)? as HWND;
        bind_child(h, desktop()?)?;
        let size = w.inner_size().map_err(err)?;
        place(w, x, y, size.width, size.height, true)?;
        check_attachment(w, false)
    }
    pub fn place(
        w: &WebviewWindow,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        show: bool,
    ) -> Result<(), String> {
        let h = handle(w)? as HWND;
        let mut point = POINT { x, y };
        unsafe {
            let child = GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD != 0;
            if child && ScreenToClient(GetParent(h), &mut point) == 0 {
                return Err(winerr("转换桌面坐标失败"));
            }
            let insert = if child {
                HWND_TOP
            } else if w.label() == PANEL {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            };
            if SetWindowPos(
                h,
                insert,
                point.x,
                point.y,
                width as i32,
                height as i32,
                SWP_NOACTIVATE | SWP_FRAMECHANGED | if show { SWP_SHOWWINDOW } else { 0 },
            ) == 0
            {
                return Err(winerr("设置窗口几何失败"));
            }
        }
        Ok(())
    }
    pub fn show(w: &WebviewWindow) -> Result<(), String> {
        unsafe {
            ShowWindow(handle(w)? as HWND, SW_SHOWNOACTIVATE);
        };
        Ok(())
    }
    pub fn hide(w: &WebviewWindow) -> Result<(), String> {
        // 原生附着后的样式由本模块拥有，不让 Tao 的旧窗口标志覆盖 WS_CHILD。
        unsafe {
            ShowWindow(handle(w)? as HWND, SW_HIDE);
        }
        Ok(())
    }
    pub fn check_attachment(w: &WebviewWindow, pinned: bool) -> Result<(), String> {
        let h = handle(w)? as HWND;
        unsafe {
            attachment_valid(
                IsWindow(h) != 0 && pid(h) == std::process::id(),
                GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD != 0,
                if pinned {
                    GetParent(h).is_null()
                } else {
                    GetParent(h) == desktop()?
                },
                GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0,
                pinned,
            )
            .map_err(|e| {
                format!(
                    "{} 层级校验失败（预期置顶={pinned}）：{e}；实际 {}",
                    w.label(),
                    describe(w)
                )
            })?;
        }
        Ok(())
    }
    pub fn toggle_pin(w: &WebviewWindow) -> Result<bool, String> {
        let h = handle(w)? as HWND;
        let mut rect = RECT::default();
        unsafe {
            if GetWindowRect(h, &mut rect) == 0 {
                return Err(winerr("读取卡片位置失败"));
            }
            if GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD == 0 {
                attach(w, rect.left, rect.top)?;
                return Ok(false);
            }
            windows_sys::Win32::Foundation::SetLastError(0);
            SetParent(h, std::ptr::null_mut());
            if windows_sys::Win32::Foundation::GetLastError() != 0 {
                return Err(winerr("卡片脱离桌面失败"));
            }
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            windows_sys::Win32::Foundation::SetLastError(0);
            let old = SetWindowLongPtrW(h, GWL_STYLE, ((style & !WS_CHILD) | WS_POPUP) as isize);
            if old == 0 && windows_sys::Win32::Foundation::GetLastError() != 0 {
                return Err(winerr("设置卡片置顶样式失败"));
            }
            if SetWindowPos(
                h,
                HWND_TOPMOST,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
            ) == 0
            {
                return Err(winerr("卡片置顶失败"));
            }
        }
        check_attachment(w, true)?;
        Ok(true)
    }
    pub fn describe(w: &WebviewWindow) -> Value {
        let Ok(raw) = handle(w) else {
            return json!({"label":w.label(),"error":"无原生句柄"});
        };
        let h = raw as HWND;
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(h, &mut rect);
            json!({"label":w.label(),"hwnd":raw,"pid":pid(h),"parent":GetParent(h) as usize,"parentClass":class(GetParent(h)),"class":class(h),"style":GetWindowLongPtrW(h,GWL_STYLE),"exStyle":GetWindowLongPtrW(h,GWL_EXSTYLE),"child":GetWindowLongPtrW(h,GWL_STYLE) as u32&WS_CHILD!=0,"topmost":GetWindowLongPtrW(h,GWL_EXSTYLE) as u32&WS_EX_TOPMOST!=0,"visible":IsWindowVisible(h)!=0,"minimized":IsIconic(h)!=0,"dpi":GetDpiForWindow(h),"dpiContext":GetWindowDpiAwarenessContext(h) as usize,"parentDpiContext":GetWindowDpiAwarenessContext(GetParent(h)) as usize,"dwm":dwm_state(h),"keyboard":keyboard_state(h),"rect":{"left":rect.left,"top":rect.top,"width":rect.right-rect.left,"height":rect.bottom-rect.top}})
        }
    }
    pub fn owned_alive(h: usize) -> bool {
        unsafe { IsWindow(h as HWND) != 0 && pid(h as HWND) == std::process::id() }
    }
    fn dwm_state(h: HWND) -> Value {
        use windows_sys::Win32::Graphics::Dwm::*;
        let mut backdrop = 0u32;
        let mut dark = 0u32;
        let mut composition = 0;
        unsafe {
            let backdrop_hr = DwmGetWindowAttribute(
                h,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &mut backdrop as *mut _ as _,
                4,
            );
            let dark_hr = DwmGetWindowAttribute(
                h,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &mut dark as *mut _ as _,
                4,
            );
            let composition_hr = DwmIsCompositionEnabled(&mut composition);
            json!({"backdropQueryHresult":backdrop_hr,"backdropType":if backdrop_hr==0{Some(backdrop)}else{None},"darkMode":if dark_hr==0{Some(dark)}else{None},"compositionEnabled":if composition_hr==0{Some(composition!=0)}else{None}})
        }
    }
    pub fn persistent_acrylic(w: &WebviewWindow) -> Result<(), String> {
        apply_accent(handle(w)? as HWND)
    }
    fn apply_accent(h: HWND) -> Result<(), String> {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE};
        if unsafe { IsWindow(h) } == 0 || pid(h) != std::process::id() {
            return Err("拒绝向失效或其他进程窗口应用材质".into());
        }
        let none = 1u32;
        let hr = unsafe {
            DwmSetWindowAttribute(
                h,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &none as *const _ as _,
                4,
            )
        };
        if hr < 0 {
            return Err(format!("关闭系统背景材质失败：HRESULT={hr:#x}"));
        }
        set_accent(h, true)
    }
    pub fn clear_accent(w: &WebviewWindow) -> Result<(), String> {
        set_accent(handle(w)? as HWND, false)
    }
    // ABI 参考 window-vibrancy 0.8.1 src/windows.rs（MIT），许可见 docs/licenses/。
    // Copyright 2019-2022 Tauri Programme within The Commons Conservancy
    // 入口未公开；只对本次探针窗口调用，检查句柄、导出入口及真实返回值。
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct CompositionAttribute {
        attribute: u32,
        data: *mut std::ffi::c_void,
        size: usize,
    }
    fn set_accent(h: HWND, enabled: bool) -> Result<(), String> {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        if unsafe { IsWindow(h) } == 0 || pid(h) != std::process::id() {
            return Err("拒绝向失效或其他进程窗口应用材质".into());
        }
        unsafe {
            let module = GetModuleHandleW(wide("user32.dll").as_ptr());
            if module.is_null() {
                return Err(winerr("读取窗口合成模块失败"));
            }
            let entry = GetProcAddress(module, c"SetWindowCompositionAttribute".as_ptr() as _)
                .ok_or("系统没有窗口合成入口，候选不可用")?;
            let set: unsafe extern "system" fn(HWND, *mut CompositionAttribute) -> i32 =
                std::mem::transmute(entry);
            let mut policy = AccentPolicy {
                state: if enabled { 4 } else { 0 },
                flags: 0,
                gradient: 0x011c1c1c,
                animation: 0,
            };
            let mut data = CompositionAttribute {
                attribute: 0x13,
                data: &mut policy as *mut _ as _,
                size: std::mem::size_of_val(&policy),
            };
            windows_sys::Win32::Foundation::SetLastError(0);
            if set(h, &mut data) == 0 {
                return Err(winerr("窗口合成请求被拒绝"));
            }
        }
        Ok(())
    }
    pub fn system() -> Value {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
            .ok();
        let get = |name: &str| {
            key.as_ref()
                .and_then(|k| k.get_value::<String, _>(name).ok())
        };
        let transparency = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
            .ok()
            .and_then(|k| k.get_value::<u32, _>("EnableTransparency").ok());
        json!({"os":"windows","build":get("CurrentBuildNumber"),"displayVersion":get("DisplayVersion"),"pid":std::process::id(),"tauri":"2.12.5","attachment":"existing-shell-defview-parent-child-experiment","material":"per-window-accent-candidate","transparencySetting":transparency})
    }

    #[cfg(test)]
    #[test]
    fn accent_candidate_checks_handles_and_does_not_change_foreground() {
        assert!(apply_accent(std::ptr::null_mut()).is_err());
        struct Owned(HWND);
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let h = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("SoulDesk hidden material test").as_ptr(),
                WS_POPUP,
                0,
                0,
                160,
                120,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!h.is_null());
        let owned = Owned(h);
        let before = foreground();
        // 此断言只证明 API 接受请求及未改变前台，不能证明肉眼磨砂效果。
        apply_accent(owned.0).unwrap();
        assert_eq!(foreground(), before);
        assert_eq!(unsafe { IsWindowVisible(owned.0) }, 0);
    }
    #[cfg(test)]
    #[test]
    fn pin_to_child_roundtrip_releases_the_actual_topmost_style() {
        struct Owned(HWND);
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        // 仅本测试的两个不可见窗口，不附着 Explorer、不显示或激活窗口。
        let make = |extra| {
            let h = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | extra,
                    wide("STATIC").as_ptr(),
                    wide("SoulDesk hidden native test").as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    160,
                    120,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!h.is_null());
            Owned(h)
        };
        let parent = make(0);
        let child = make(WS_EX_TOPMOST);
        unsafe {
            assert_ne!(
                SetWindowPos(
                    child.0,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
                ),
                0
            );
        }
        bind_child(child.0, parent.0).unwrap();
        unsafe {
            assert_eq!(GetParent(child.0), parent.0);
            assert_ne!(GetWindowLongPtrW(child.0, GWL_STYLE) as u32 & WS_CHILD, 0);
            assert_eq!(
                GetWindowLongPtrW(child.0, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            assert_eq!(IsWindowVisible(child.0), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_or_missing_material_context_cannot_overwrite_current_observation() {
        assert!(validate_material_observation(Some(4), Some(0.12), 4, 0.12).is_ok());
        assert!(validate_material_observation(Some(3), Some(0.12), 4, 0.12).is_err());
        assert!(validate_material_observation(Some(4), Some(0.28), 4, 0.12).is_err());
        assert!(validate_material_observation(None, Some(0.12), 4, 0.12).is_err());
        assert!(validate_material_observation(Some(4), None, 4, 0.12).is_err());
    }
    #[test]
    fn tint_rejects_non_finite_and_out_of_range_values() {
        for opacity in [0.0, 0.28, 1.0] {
            assert!(validate_tint(opacity).is_ok());
        }
        for opacity in [-0.01, 1.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(validate_tint(opacity).is_err());
        }
    }
    #[test]
    fn report_write_preserves_an_existing_similarly_named_temporary_file() {
        let root = std::env::temp_dir().join(format!("souldesk-report-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let report = root.join("report.json");
        let sentinel = root.join("report.json.tmp");
        std::fs::write(&sentinel, b"existing-user-file").unwrap();
        write_json(&report, &json!({"complete":false})).unwrap();
        let preserved = std::fs::read(&sentinel).ok();
        std::fs::remove_file(&report).unwrap();
        if sentinel.exists() {
            std::fs::remove_file(&sentinel).unwrap();
        }
        std::fs::remove_dir(&root).unwrap();
        assert_eq!(preserved, Some(b"existing-user-file".to_vec()));
    }
    #[test]
    fn missing_or_unexpected_window_layer_cannot_pass() {
        assert!(attachment_valid(true, true, true, false, false).is_ok());
        assert!(attachment_valid(true, false, true, true, true).is_ok());
        for state in [
            (false, true, true, false, false),
            (true, true, false, false, false),
            (true, true, true, true, false),
            (true, false, true, false, false),
            (true, true, true, true, true),
            (true, false, false, true, true),
        ] {
            assert!(attachment_valid(state.0, state.1, state.2, state.3, state.4).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn incompatible_dpi_is_rejected_before_cross_process_parenting() {
        use windows_sys::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
        };
        let per_monitor = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 as usize;
        let system = DPI_AWARENESS_CONTEXT_SYSTEM_AWARE as usize;
        assert!(native::verify_dpi(per_monitor, per_monitor).is_ok());
        assert!(native::verify_dpi(per_monitor, system).is_err());
        assert!(native::verify_dpi(0, per_monitor).is_err());
    }
    #[test]
    fn event_names_without_actual_transitions_do_not_complete_automatic_evidence() {
        let mut report = json!({"nativeErrors":[],"observations":{},"events":[
            {"kind":"startup","details":{"focusUnchanged":true}},
            {"kind":"ready","details":{"caller":CLOCK}},{"kind":"ready","details":{"caller":CARD}},{"kind":"ready","details":{"caller":PANEL}},
            {"kind":"ratio-saved"},{"kind":"rebuild-panel"},{"kind":"restore-fixtures","details":{"foregroundBefore":123,"foregroundAfter":123}}
        ]});
        finish_report(&mut report, &[], 0);
        assert_eq!(report["automaticEvidenceComplete"], false);
    }
    #[test]
    fn incomplete_visual_focus_or_cleanup_evidence_cannot_pass() {
        let mut report = json!({"nativeErrors":[],"events":[],"observations":{},"appearanceEpoch":0,"appearance":{"material":"system-acrylic"}});
        finish_report(&mut report, &[], 0);
        assert_eq!(report["platformPassed"], false);
        assert_eq!(report["cleanup"]["remainingOwnedHandles"], json!([]));
        assert_ne!(report["status"], "starting");
        for key in [
            "desktop",
            "cover",
            "winD",
            "taskSwitcher",
            "input",
            "acrylic",
            "panelGeometry",
            "pin",
        ] {
            report["observations"][key] = json!({"result":"pass","appearanceEpoch":0});
        }
        // 只有人工勾选也不能替代窗口就绪、后台无焦点恢复和几何重建证据。
        finish_report(&mut report, &[], 0);
        assert_eq!(report["platformPassed"], false);
        let fixtures = |visible| {
            json!([
                {"label":CLOCK,"hwnd":11,"visible":visible},
                {"label":CARD,"hwnd":12,"visible":visible}
            ])
        };
        let panel = |hwnd| json!([{"label":PANEL,"hwnd":hwnd,"rect":{"left":960,"top":0,"width":960,"height":1040}}]);
        report["events"] = json!([
            {"kind":"startup","details":{"focusUnchanged":true}},
            {"kind":"ready","details":{"caller":CLOCK}},
            {"kind":"ready","details":{"caller":CARD}},
            {"kind":"ready","details":{"caller":PANEL}},
            {"kind":"hide-fixtures","windows":fixtures(false)},
            {"kind":"restore-fixtures","details":{"foregroundBefore":123,"foregroundAfter":123},"windows":fixtures(true)},
            {"kind":"ratio-saved","details":{"geometry":{"requested_ratio":0.5}},"windows":panel(21)},
            {"kind":"panel-created","details":{"savedRatio":0.5,"workArea":{"left":0,"top":0,"width":1920,"height":1040,"scale_factor":1.0}},"windows":panel(22)},
            {"kind":"rebuild-panel","windows":panel(22)},
            {"kind":"ready","details":{"caller":PANEL},"windows":panel(22)}
        ]);
        finish_report(&mut report, &[], 0);
        assert_eq!(report["platformPassed"], true);
        let mut stale = report.clone();
        stale["appearanceEpoch"] = json!(1);
        finish_report(&mut stale, &[], 0);
        assert_eq!(stale["platformPassed"], false);
        let mut candidate = report.clone();
        candidate["appearance"] = json!({"material":"accent-acrylic","apiAccepted":true});
        finish_report(&mut candidate, &[], 0);
        assert_eq!(candidate["platformPassed"], true);
        candidate["appearance"]["apiAccepted"] = json!(false);
        finish_report(&mut candidate, &[], 0);
        assert_eq!(candidate["platformPassed"], false);
        let mut clear = report.clone();
        clear["appearance"]["material"] = json!("transparent-baseline");
        finish_report(&mut clear, &[], 0);
        assert_eq!(clear["platformPassed"], false, "透明基线不能替代磨砂验收");
        for missing in [4, 6, 7, 8, 9] {
            let mut incomplete = report.clone();
            incomplete["events"].as_array_mut().unwrap().remove(missing);
            finish_report(&mut incomplete, &[], 0);
            assert_eq!(
                incomplete["platformPassed"], false,
                "缺少第 {missing} 项证据"
            );
        }
        report["observations"]["acrylic"]["result"] = json!("unverified");
        finish_report(&mut report, &[], 0);
        assert_eq!(report["platformPassed"], false);
        report["observations"]["acrylic"]["result"] = json!("pass");
        finish_report(&mut report, &[456], 0);
        assert_eq!(report["platformPassed"], false);
        finish_report(&mut report, &[], 1);
        assert_eq!(report["platformPassed"], false);
        report["events"][0]["details"]["focusUnchanged"] = json!(false);
        finish_report(&mut report, &[], 0);
        assert_eq!(report["platformPassed"], false);
    }
    #[test]
    fn report_requires_explicit_absolute_json_file() {
        for bad in [
            PathBuf::from("relative.json"),
            PathBuf::from("C:relative.json"),
            PathBuf::from("C:\\"),
            PathBuf::from("C:\\temp\\report.txt"),
        ] {
            assert!(report_root(&bad).is_err(), "{}", bad.display());
        }
        let report = std::env::temp_dir().join("desktop-probe-test.json");
        assert_eq!(report_root(&report).unwrap(), report.with_extension(""));
    }
    #[test]
    fn window_identity_cannot_be_claimed_by_normal_views() {
        for label in [
            "main",
            "float-a",
            "edge",
            "trial-a",
            "desktop-probe-panel-fake",
            "",
        ] {
            assert!(authorize(label, false).is_err());
        }
        for label in [CLOCK, CARD] {
            assert!(authorize(label, false).is_ok());
            assert!(authorize(label, true).is_err());
        }
        assert!(authorize(PANEL, true).is_ok());
    }
    #[test]
    fn invalid_ratio_preserves_previous_geometry() {
        let area = WorkArea {
            left: 0,
            top: 0,
            width: 1920,
            height: 1040,
            scale_factor: 1.0,
        };
        let mut current = panel_geometry(area, 0.4, 280.0).unwrap();
        for ratio in [0.0, -1.0, f64::NAN, f64::INFINITY, 1.1] {
            assert!(change_ratio(&mut current, area, ratio).is_err());
            assert_eq!(current.width, 768);
        }
        change_ratio(&mut current, area, 0.5).unwrap();
        assert_eq!(current.width, 960);
    }
}
