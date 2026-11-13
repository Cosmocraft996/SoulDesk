//! 仅桌面探针使用的普通组件适配；复用包校验及 SDK 会话，不初始化正常 Core。
use crate::{
    store::{err, Store},
    widgets::{self, Preview, StagedPackage},
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tauri::{AppHandle, WebviewWindow};

const PANEL: &str = "desktop-probe-panel";
const LABELS: [&str; 2] = ["desktop-probe-clock", "desktop-probe-card"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    pub instance_id: String,
    pub desktop_label: String,
    pub location: String,
    pub visible: bool,
}
#[derive(Default)]
struct State {
    views: BTreeMap<String, View>,
    trials: HashMap<String, Arc<Mutex<Preview>>>,
    verified: HashSet<String>,
    ready: HashMap<String, u64>,
    acknowledgements: HashMap<String, u64>,
    revision: u64,
    events: Vec<Value>,
    automatic: Value,
    testing: bool,
}
pub(super) struct WidgetProbe {
    pub store: Store,
    sessions: widgets::Sessions,
    packages: Vec<StagedPackage>,
    sources: Vec<Value>,
    state: Mutex<State>,
    changed: Condvar,
}
impl WidgetProbe {
    pub fn new(root: PathBuf, paths: [PathBuf; 2]) -> Result<Self, String> {
        let store = Store::open(root)?;
        let mut packages = Vec::new();
        let mut sources = Vec::new();
        for (index, path) in paths.into_iter().enumerate() {
            if !path.is_absolute() || !path.is_file() {
                return Err("真实组件探针需要两个现存的绝对 ZIP 路径".into());
            }
            let bytes = std::fs::read(&path).map_err(err)?;
            let archive_hash = format!("{:x}", Sha256::digest(&bytes));
            let package = widgets::stage_bytes(&store, bytes)?;
            let expected = ["local.example.clock", "local.example.card"][index];
            if package.manifest.id != expected
                || package.manifest.capabilities != ["storage.instance"]
            {
                return Err(format!("仅接受本次原时钟/卡片普通包：{expected}"));
            }
            sources.push(json!({"path":path,"archiveSha256":archive_hash,"packageId":package.manifest.id,"version":package.manifest.version,"digest":package.digest}));
            packages.push(package);
        }
        Ok(Self {
            store,
            sessions: Default::default(),
            packages,
            sources,
            state: Mutex::new(State {
                testing: true,
                ..Default::default()
            }),
            changed: Condvar::new(),
        })
    }
    fn log(state: &mut State, kind: &str, details: Value) {
        if state.events.len() < 200 {
            state
                .events
                .push(json!({"timeMs":crate::store::now_ms(),"kind":kind,"details":details}));
        }
    }
    fn owner(state: &State, window: &str, instance: &str) -> Result<String, String> {
        let view = state
            .views
            .get(instance)
            .ok_or("SESSION_EXPIRED: 未登记的测试实例")?;
        if !view.visible {
            return Err("SESSION_EXPIRED: 展示已隐藏".into());
        }
        if view.location == "desktop" && window == view.desktop_label {
            Ok(format!("float-{instance}"))
        } else if view.location == "panel" && window == PANEL {
            Ok("main".into())
        } else {
            Err("CAPABILITY_DENIED: 当前窗口不是实例的主展示".into())
        }
    }
    fn mounted(state: &State, window: &str) -> Vec<String> {
        state
            .views
            .values()
            .filter(|v| {
                v.visible
                    && (v.location == "panel" && window == PANEL
                        || v.location == "desktop" && v.desktop_label == window)
            })
            .map(|v| v.instance_id.clone())
            .collect()
    }
    pub fn begin_trial(&self, index: usize) -> Result<String, String> {
        let package = self.packages.get(index).ok_or("不存在的测试包")?.clone();
        let label = format!("desktop-probe-trial-{index}");
        let preview = Arc::new(Mutex::new(Preview {
            settings: widgets::initial_settings(&package.manifest),
            storage: json!({}),
            package,
            ready_since: None,
            failure: None,
        }));
        self.state
            .lock()
            .map_err(err)?
            .trials
            .insert(label.clone(), preview);
        Ok(label)
    }
    pub fn trial_ready(&self, label: &str) -> Result<bool, String> {
        let state = self.state.lock().map_err(err)?;
        let trial = state
            .trials
            .get(label)
            .ok_or("试加载已结束")?
            .lock()
            .map_err(err)?;
        if let Some(error) = &trial.failure {
            return Err(format!("LOAD_FAILED: {error}"));
        }
        Ok(trial
            .ready_since
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(800)))
    }
    pub fn end_trial(&self, label: &str, index: usize, activate: bool) -> Result<(), String> {
        let mut state = self.state.lock().map_err(err)?;
        state.trials.remove(label);
        self.sessions
            .0
            .lock()
            .map_err(err)?
            .retain(|_, s| s.window != label);
        if activate {
            let package = self.packages.get(index).ok_or("不存在的测试包")?;
            widgets::activate_sessions(&self.store, &self.sessions, package)?;
            let id = widgets::create_instance(&self.store, &package.manifest.id)?;
            widgets::set_floating(&self.store, &self.sessions, &id, true)?;
            state.views.insert(
                id.clone(),
                View {
                    instance_id: id.clone(),
                    desktop_label: LABELS[index].into(),
                    location: "desktop".into(),
                    visible: true,
                },
            );
            Self::log(
                &mut state,
                "zip-trial-activated",
                json!({"instanceId":id,"packageId":package.manifest.id,"hiddenTrial":true}),
            );
        }
        Ok(())
    }
    pub fn open(
        &self,
        window: &str,
        instance: &str,
        digest: Option<&str>,
    ) -> Result<Value, String> {
        let mut state = self.state.lock().map_err(err)?;
        if let Some(trial) = state.trials.get(instance) {
            return widgets::open_preview(&self.sessions, window, instance, trial.clone());
        }
        let owner = Self::owner(&state, window, instance)?;
        let mut value =
            widgets::open_session_expected(&self.store, &self.sessions, &owner, instance, digest)?;
        value["verify"] = json!(!state.verified.contains(instance));
        Self::log(
            &mut state,
            "session-open",
            json!({"instanceId":instance,"window":window}),
        );
        Ok(value)
    }
    pub fn close(&self, window: &str, token: &str) -> Result<(), String> {
        let mut state = self.state.lock().map_err(err)?;
        let mut sessions = self.sessions.0.lock().map_err(err)?;
        if let Some(session) = sessions.get(token) {
            let allowed = if session.preview.is_some() {
                session.window == window
            } else {
                Self::owner(&state, window, &session.instance)
                    .is_ok_and(|owner| owner == session.window)
            };
            if allowed {
                let instance = session.instance.clone();
                sessions.remove(token);
                Self::log(
                    &mut state,
                    "session-close",
                    json!({"instanceId":instance,"window":window}),
                );
            }
        }
        Ok(())
    }
    pub fn request(
        &self,
        window: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> Result<Value, String> {
        let mut state = self.state.lock().map_err(err)?;
        let session = self
            .sessions
            .0
            .lock()
            .map_err(err)?
            .get(token)
            .cloned()
            .ok_or("SESSION_EXPIRED: 旧会话已释放")?;
        let owner = if session.preview.is_some() {
            if session.window != window {
                return Err("CAPABILITY_DENIED: 非对应试加载窗口".into());
            }
            window.to_owned()
        } else {
            Self::owner(&state, window, &session.instance)?
        };
        let value = widgets::gateway(
            &self.store,
            &self.sessions,
            &owner,
            token,
            method,
            args.clone(),
        )?;
        if session.preview.is_none() {
            if method == "runtime.ready" {
                *state.ready.entry(session.instance.clone()).or_default() += 1;
                Self::log(
                    &mut state,
                    "sdk-ready",
                    json!({"instanceId":session.instance,"window":window}),
                );
            }
            if method == "storage.set"
                && args["moduleExecuted"] == true
                && args["buttonSave"] == true
            {
                state.verified.insert(session.instance.clone());
                Self::log(
                    &mut state,
                    "original-package-proof",
                    json!({"instanceId":session.instance,"window":window,"moduleExecuted":args["moduleExecuted"],"buttonSave":args["buttonSave"],"hostTokenAbsent":args["hostTokenAbsent"],"directHostResult":args["directHostResult"],"origin":args["origin"],"background":args["background"],"denied":args["denied"],"standardsMode":args["standardsMode"]}),
                );
            }
            if method == "settings.update" {
                Self::log(
                    &mut state,
                    "settings-saved",
                    json!({"instanceId":session.instance,"settingsSha256":format!("{:x}",Sha256::digest(args.to_string()))}),
                );
            }
        }
        self.changed.notify_all();
        Ok(value)
    }
    pub fn fail_trial(&self, window: &str, message: &str) -> Result<(), String> {
        if let Some(trial) = self.state.lock().map_err(err)?.trials.get(window) {
            trial.lock().map_err(err)?.failure = Some(message.chars().take(1000).collect());
            self.changed.notify_all();
        }
        Ok(())
    }
    pub fn view(&self, id: &str) -> Result<View, String> {
        self.state
            .lock()
            .map_err(err)?
            .views
            .get(id)
            .cloned()
            .ok_or("未登记的测试实例".into())
    }
    pub fn ids(&self) -> Result<Vec<String>, String> {
        Ok(self
            .state
            .lock()
            .map_err(err)?
            .views
            .keys()
            .cloned()
            .collect())
    }
    pub fn change(&self, id: &str, location: &str, visible: bool) -> Result<(View, u64), String> {
        if !["desktop", "panel"].contains(&location) {
            return Err("无效的主展示位置".into());
        }
        let mut state = self.state.lock().map_err(err)?;
        let previous = state.views.get(id).cloned().ok_or("未登记的测试实例")?;
        // 与既有会话入口保持同一锁顺序；持久归属成功后才更新展示意图。
        widgets::set_floating(&self.store, &self.sessions, id, location == "desktop")?;
        let view = state.views.get_mut(id).ok_or("未登记的测试实例")?;
        view.location = location.into();
        view.visible = visible;
        state.revision = state.revision.checked_add(1).ok_or("展示代次溢出")?;
        let revision = state.revision;
        Self::log(
            &mut state,
            "view-changed",
            json!({"instanceId":id,"location":location,"visible":visible,"revision":revision,"oldSessionsRevoked":true}),
        );
        Ok((previous, revision))
    }
    pub fn acknowledge(
        &self,
        window: &str,
        revision: u64,
        mut instances: Vec<String>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().map_err(err)?;
        instances.sort();
        if ![PANEL, LABELS[0], LABELS[1]].contains(&window)
            || revision != state.revision
            || instances != Self::mounted(&state, window)
        {
            return Err("STALE_VIEW: 展示已变化".into());
        }
        state.acknowledgements.insert(window.into(), revision);
        self.changed.notify_all();
        Ok(())
    }
    pub fn wait_unmounted(&self, window: &str, revision: u64) -> Result<(), String> {
        let state = self.state.lock().map_err(err)?;
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |s| {
                s.acknowledgements.get(window).copied() != Some(revision)
            })
            .map_err(err)?;
        if state.acknowledgements.get(window).copied() != Some(revision) {
            return Err("LOAD_FAILED: 旧展示未确认卸载".into());
        }
        Ok(())
    }
    pub fn ready_count(&self, id: &str) -> Result<u64, String> {
        Ok(*self.state.lock().map_err(err)?.ready.get(id).unwrap_or(&0))
    }
    pub fn wait_ready(&self, id: &str, previous: u64, proof: bool) -> Result<(), String> {
        let state = self.state.lock().map_err(err)?;
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |s| {
                s.ready.get(id).copied().unwrap_or(0) <= previous
                    || proof && !s.verified.contains(id)
            })
            .map_err(err)?;
        if state.ready.get(id).copied().unwrap_or(0) <= previous
            || proof && !state.verified.contains(id)
        {
            return Err("LOAD_FAILED: 实际组件未完成 SDK 握手或原包自检".into());
        }
        Ok(())
    }
    pub fn settings_hash(&self, id: &str) -> Result<String, String> {
        let value: String = self
            .store
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT settings FROM widget_instances WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        Ok(format!("{:x}", Sha256::digest(value)))
    }
    pub fn snapshot(&self) -> Result<Value, String> {
        let state = self.state.lock().map_err(err)?;
        let sessions = self.sessions.0.lock().map_err(err)?;
        Ok(
            json!({"revision":state.revision,"instances":state.views.values().collect::<Vec<_>>(),"sessionCount":sessions.len(),"verifiedCount":state.verified.len(),"ready":state.ready,"testing":state.testing,"automatic":state.automatic}),
        )
    }
    pub fn evidence(&self) -> Result<Value, String> {
        let snapshot = self.snapshot()?;
        let state = self.state.lock().map_err(err)?;
        Ok(
            json!({"snapshot":snapshot,"sources":self.sources,"events":state.events,"dataRoot":self.store.root,"normalDatabaseOpened":false}),
        )
    }
    pub fn finish_automatic(&self, result: Value) -> Result<(), String> {
        let mut state = self.state.lock().map_err(err)?;
        state.testing = false;
        state.automatic = result;
        Ok(())
    }
    pub fn clear_sessions(&self) -> Result<(), String> {
        self.sessions.0.lock().map_err(err)?.clear();
        Ok(())
    }
    pub fn revoke_host(&self, window: &str) -> Result<(), String> {
        let state = self.state.lock().map_err(err)?;
        let ids: HashSet<_> = state
            .views
            .values()
            .filter(|v| {
                v.location == "panel" && window == PANEL
                    || v.location == "desktop" && v.desktop_label == window
            })
            .map(|v| v.instance_id.as_str())
            .collect();
        self.sessions
            .0
            .lock()
            .map_err(err)?
            .retain(|_, s| !ids.contains(s.instance.as_str()));
        Ok(())
    }
    pub fn hide_host(&self, window: &str) -> Result<Option<u64>, String> {
        if ![PANEL, LABELS[0], LABELS[1]].contains(&window) {
            return Err("CAPABILITY_DENIED: 非组件展示容器".into());
        }
        let ids = {
            let state = self.state.lock().map_err(err)?;
            Self::mounted(&state, window)
        };
        let mut revision = None;
        for id in ids {
            let view = self.view(&id)?;
            revision = Some(self.change(&id, &view.location, false)?.1);
        }
        Ok(revision)
    }
    pub fn wait_host_hidden(&self, window: &str) -> Result<(), String> {
        let state = self.state.lock().map_err(err)?;
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |s| {
                !Self::mounted(s, window).is_empty()
                    || s.acknowledgements.get(window).copied() != Some(s.revision)
            })
            .map_err(err)?;
        if !Self::mounted(&state, window).is_empty()
            || state.acknowledgements.get(window).copied() != Some(state.revision)
        {
            return Err("LOAD_FAILED: 系统关闭路径未完成展示卸载".into());
        }
        Ok(())
    }
}

#[tauri::command]
pub(super) fn desktop_widget_open_session(
    app: AppHandle,
    window: WebviewWindow,
    instance: String,
    expected_digest: Option<String>,
) -> Result<Value, String> {
    crate::desktop_probe::widget_state(&app)?.open(
        window.label(),
        &instance,
        expected_digest.as_deref(),
    )
}
#[tauri::command]
pub(super) fn desktop_widget_close_session(
    app: AppHandle,
    window: WebviewWindow,
    token: String,
) -> Result<(), String> {
    crate::desktop_probe::widget_state(&app)?.close(window.label(), &token)
}
#[tauri::command]
pub(super) fn desktop_widget_widget_request(
    app: AppHandle,
    window: WebviewWindow,
    token: String,
    method: String,
    args: Value,
) -> Result<Value, String> {
    crate::desktop_probe::widget_state(&app)?.request(window.label(), &token, &method, args)
}
#[tauri::command]
pub(super) fn desktop_widget_trial_failed(
    app: AppHandle,
    window: WebviewWindow,
    message: String,
) -> Result<(), String> {
    crate::desktop_probe::widget_state(&app)?.fail_trial(window.label(), &message)
}
#[tauri::command]
pub(super) fn desktop_widget_view(
    app: AppHandle,
    window: WebviewWindow,
    revision: u64,
    instances: Vec<String>,
) -> Result<(), String> {
    crate::desktop_probe::widget_state(&app)?.acknowledge(window.label(), revision, instances)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (WidgetProbe, String, String, PathBuf) {
        let root = std::env::temp_dir().join(format!("souldesk-widget-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        for id in ["local.example.clock", "local.example.card"] {
            let manifest = crate::presets::find(id)
                .unwrap()
                .unwrap()
                .archive()
                .unwrap();
            let staged = widgets::stage_bytes(&store, manifest).unwrap();
            widgets::activate(&store, &staged).unwrap();
        }
        let mut state = State::default();
        let mut ids = Vec::new();
        for (i, id) in ["local.example.clock", "local.example.card"]
            .iter()
            .enumerate()
        {
            let instance = widgets::create_instance(&store, id).unwrap();
            widgets::set_floating(&store, &widgets::Sessions::default(), &instance, true).unwrap();
            state.views.insert(
                instance.clone(),
                View {
                    instance_id: instance.clone(),
                    desktop_label: LABELS[i].into(),
                    location: "desktop".into(),
                    visible: true,
                },
            );
            ids.push(instance);
        }
        (
            WidgetProbe {
                store,
                sessions: Default::default(),
                packages: vec![],
                sources: vec![],
                state: Mutex::new(state),
                changed: Condvar::new(),
            },
            ids[0].clone(),
            ids[1].clone(),
            root,
        )
    }
    #[test]
    fn closing_a_host_releases_its_session_and_requires_a_fresh_recovery_handshake() {
        let (probe, clock, card, _root) = fixture();
        let old = probe.open(LABELS[0], &clock, None).unwrap();
        let other = probe.open(LABELS[1], &card, None).unwrap();
        let hash = probe.settings_hash(&clock).unwrap();
        let revision = probe.hide_host(LABELS[0]).unwrap().unwrap();
        assert!(!probe.view(&clock).unwrap().visible);
        assert_eq!(probe.snapshot().unwrap()["sessionCount"], 1);
        assert!(probe
            .request(
                LABELS[0],
                old["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .is_err());
        assert!(probe
            .request(
                LABELS[1],
                other["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .is_ok());
        probe.acknowledge(LABELS[0], revision, vec![]).unwrap();
        probe.change(&clock, "desktop", true).unwrap();
        let fresh = probe.open(LABELS[0], &clock, None).unwrap();
        assert_ne!(fresh["token"], old["token"]);
        assert_eq!(probe.settings_hash(&clock).unwrap(), hash);
    }
    #[test]
    fn transfer_revokes_old_owner_and_failure_restore_keeps_settings() {
        let (probe, clock, _, _root) = fixture();
        assert!(probe.open(PANEL, &clock, None).is_err());
        assert!(probe.open(LABELS[1], &clock, None).is_err());
        let old = probe.open(LABELS[0], &clock, None).unwrap();
        probe
            .request(
                LABELS[0],
                old["token"].as_str().unwrap(),
                "settings.update",
                json!({"title":"保留配置","showSeconds":false}),
            )
            .unwrap();
        let hash = probe.settings_hash(&clock).unwrap();
        let (previous, _) = probe.change(&clock, "panel", true).unwrap();
        assert!(probe
            .request(
                LABELS[0],
                old["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .is_err());
        assert!(probe.open(LABELS[0], &clock, None).is_err());
        let new = probe.open(PANEL, &clock, None).unwrap();
        assert_eq!(probe.snapshot().unwrap()["sessionCount"], 1);
        probe
            .change(&clock, &previous.location, previous.visible)
            .unwrap();
        assert!(probe
            .request(
                PANEL,
                new["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .is_err());
        let restored = probe.open(LABELS[0], &clock, None).unwrap();
        probe
            .close(LABELS[0], old["token"].as_str().unwrap())
            .unwrap();
        assert!(probe
            .request(
                LABELS[0],
                restored["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .is_ok());
        assert_eq!(probe.settings_hash(&clock).unwrap(), hash);
    }
    #[test]
    fn hiding_releases_only_the_target_and_rejects_forged_or_stale_acknowledgements() {
        let (probe, clock, card, _root) = fixture();
        let old = probe.open(LABELS[0], &clock, None).unwrap();
        let card_session = probe.open(LABELS[1], &card, None).unwrap();
        let (_, revision) = probe.change(&clock, "desktop", false).unwrap();
        assert_eq!(probe.snapshot().unwrap()["sessionCount"], 1);
        assert!(probe.open(LABELS[0], &clock, None).is_err());
        assert!(probe
            .request(
                LABELS[0],
                old["token"].as_str().unwrap(),
                "runtime.ready",
                Value::Null
            )
            .is_err());
        assert!(probe
            .request(
                LABELS[1],
                card_session["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .is_ok());
        assert!(probe
            .acknowledge(LABELS[0], revision, vec![clock.clone()])
            .is_err());
        assert!(probe.acknowledge("main", revision, vec![]).is_err());
        assert!(probe.acknowledge(LABELS[0], revision + 1, vec![]).is_err());
        probe.acknowledge(LABELS[0], revision, vec![]).unwrap();
        probe.wait_unmounted(LABELS[0], revision).unwrap();
    }
}
