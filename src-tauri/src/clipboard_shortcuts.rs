//! 剪切板快捷键注册与事件归属；通过驱动隔离真实全局注册。
use crate::store::AppResult;
use std::collections::HashMap;
use tauri_plugin_global_shortcut::Shortcut;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    pub instance: String,
    pub revision: u64,
    pub shortcut: String,
}
#[derive(Clone, Default)]
pub(crate) struct Status {
    pub registered: bool,
    pub error: Option<String>,
}
pub(crate) trait Driver {
    fn register(&mut self, key: Shortcut) -> AppResult<()>;
    fn unregister(&mut self, key: Shortcut) -> AppResult<()>;
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Main,
    Clipboard(Spec),
    Ignore,
}
#[derive(Default)]
pub(crate) struct Registry {
    active: HashMap<u32, (Spec, Shortcut)>,
    failed: HashMap<String, (u64, u32, String)>,
    retired: HashMap<u32, Shortcut>,
}
impl Registry {
    pub(crate) fn sync(
        &mut self,
        specs: &[Spec],
        primary: &str,
        driver: &mut impl Driver,
    ) -> HashMap<String, Status> {
        let primary = primary.parse::<Shortcut>().ok().map(|k| k.id());
        let mut states = HashMap::new();
        let mut desired = HashMap::<u32, (Spec, Shortcut)>::new();
        let mut sorted = specs.to_vec();
        sorted.sort_by(|a, b| {
            let current = |s: &Spec| {
                self.active
                    .values()
                    .any(|(owner, _)| owner.instance == s.instance && owner.shortcut == s.shortcut)
            };
            current(b)
                .cmp(&current(a))
                .then_with(|| a.instance.cmp(&b.instance))
        });
        self.failed
            .retain(|id, _| specs.iter().any(|s| s.instance == *id));
        for spec in sorted {
            let Ok(key) = spec.shortcut.parse::<Shortcut>() else {
                states.insert(
                    spec.instance,
                    Status {
                        registered: false,
                        error: Some("快捷键格式无效".into()),
                    },
                );
                continue;
            };
            if primary == Some(key.id()) {
                states.insert(
                    spec.instance,
                    Status {
                        registered: false,
                        error: Some("快捷键与主面板冲突，未抢占".into()),
                    },
                );
                continue;
            }
            if desired.contains_key(&key.id()) {
                states.insert(
                    spec.instance,
                    Status {
                        registered: false,
                        error: Some("快捷键与其他剪切板实例冲突，未抢占".into()),
                    },
                );
                continue;
            }
            desired.insert(key.id(), (spec, key));
        }
        let remove = self
            .active
            .iter()
            .filter(|(id, (old, _))| {
                desired
                    .get(id)
                    .is_none_or(|(s, _)| s.instance != old.instance)
            })
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        let mut warnings = HashMap::new();
        for id in remove {
            if let Some((old, key)) = self.active.remove(&id) {
                if driver.unregister(key).is_err() {
                    self.retired.insert(id, key);
                    warnings.insert(
                        old.instance,
                        "旧快捷键释放失败，旧事件已禁用，退出时再释放".to_string(),
                    );
                }
            }
        }
        for (id, (spec, key)) in desired {
            let mut state = Status::default();
            if let Some((old, _)) = self.active.get_mut(&id) {
                *old = spec.clone();
                state.registered = true;
            } else if self.retired.contains_key(&id) {
                state.error = Some("快捷键尚有待释放的旧登记，请退出后重开".into());
            } else if let Some((_, _, error)) = self
                .failed
                .get(&spec.instance)
                .filter(|(revision, key, _)| *revision == spec.revision && *key == id)
            {
                state.error = Some(error.clone());
            } else {
                match driver.register(key) {
                    Ok(()) => {
                        self.active.insert(id, (spec.clone(), key));
                        self.failed.remove(&spec.instance);
                        state.registered = true;
                    }
                    Err(_) => {
                        let error = "快捷键注册失败，可能已被其他应用占用；修改或保存配置后重试"
                            .to_string();
                        self.failed
                            .insert(spec.instance.clone(), (spec.revision, id, error.clone()));
                        state.error = Some(error);
                    }
                }
            }
            if let Some(warning) = warnings.remove(&spec.instance) {
                state.error = Some(warning);
            }
            states.insert(spec.instance, state);
        }
        states
    }
    pub(crate) fn route(&self, event: u32, primary: &str) -> Route {
        if let Some((spec, _)) = self.active.get(&event) {
            return Route::Clipboard(spec.clone());
        }
        if primary.parse::<Shortcut>().is_ok_and(|k| k.id() == event) {
            Route::Main
        } else {
            Route::Ignore
        }
    }
    pub(crate) fn release(&mut self, driver: &mut impl Driver) -> AppResult<()> {
        let mut keys = std::mem::take(&mut self.retired);
        for (id, (_, key)) in std::mem::take(&mut self.active) {
            keys.insert(id, key);
        }
        self.failed.clear();
        let mut failed = false;
        for (id, key) in keys {
            if driver.unregister(key).is_err() {
                self.retired.insert(id, key);
                failed = true;
            }
        }
        if failed {
            Err("SHORTCUT_ERROR: 部分剪切板快捷键释放失败，事件已禁用".into())
        } else {
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const MAIN: &str = "Ctrl+Alt+Space";
    fn key(text: &str) -> u32 {
        text.parse::<Shortcut>().unwrap().id()
    }
    fn spec(id: &str, revision: u64, text: &str) -> Spec {
        Spec {
            instance: id.into(),
            revision,
            shortcut: text.into(),
        }
    }
    #[derive(Default)]
    struct Fake {
        registered: Vec<u32>,
        released: Vec<u32>,
        fail: bool,
        release_fail: bool,
    }
    impl Driver for Fake {
        fn register(&mut self, k: Shortcut) -> AppResult<()> {
            self.registered.push(k.id());
            if self.fail {
                Err("fixture key in use".into())
            } else {
                Ok(())
            }
        }
        fn unregister(&mut self, k: Shortcut) -> AppResult<()> {
            self.released.push(k.id());
            if self.release_fail {
                Err("fixture release denied".into())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn clipboard_shortcuts_disabled_never_registers_and_primary_is_routed_only_for_exact_key() {
        let mut r = Registry::default();
        let mut io = Fake::default();
        assert!(r.sync(&[], MAIN, &mut io).is_empty());
        assert!(io.registered.is_empty());
        assert!(io.released.is_empty());
        assert_eq!(r.route(key(MAIN), MAIN), Route::Main);
        assert_eq!(r.route(key("Super+Alt+V"), MAIN), Route::Ignore);
    }
    #[test]
    fn clipboard_shortcuts_duplicate_keys_keep_existing_owner_without_repeated_registration() {
        let mut r = Registry::default();
        let mut io = Fake::default();
        let b = spec("b", 1, "Super+Alt+V");
        r.sync(&[b.clone()], MAIN, &mut io);
        let a = spec("a", 1, "Super+Alt+V");
        let states = r.sync(&[a.clone(), b.clone()], MAIN, &mut io);
        assert!(states["b"].registered);
        assert!(!states["a"].registered);
        assert!(states["a"].error.as_ref().unwrap().contains("冲突"));
        assert_eq!(io.registered.len(), 1);
        assert_eq!(r.route(key(&a.shortcut), MAIN), Route::Clipboard(b));
    }
    #[test]
    fn clipboard_shortcuts_primary_conflict_or_invalid_key_does_not_take_over_any_existing_registration(
    ) {
        let mut r = Registry::default();
        let mut io = Fake::default();
        let states = r.sync(
            &[spec("same", 1, MAIN), spec("bad", 1, "not a shortcut")],
            MAIN,
            &mut io,
        );
        assert!(!states["same"].registered);
        assert!(states["same"].error.is_some());
        assert!(states["bad"].error.is_some());
        assert!(io.registered.is_empty());
        assert_eq!(r.route(key(MAIN), MAIN), Route::Main);
    }
    #[test]
    fn clipboard_shortcuts_registration_failure_is_visible_and_retries_only_after_configuration_changes(
    ) {
        let mut r = Registry::default();
        let mut io = Fake {
            fail: true,
            ..Default::default()
        };
        let a = spec("a", 1, "Super+Alt+V");
        let state = r.sync(&[a.clone()], MAIN, &mut io);
        assert!(!state["a"].registered);
        assert!(state["a"].error.is_some());
        r.sync(&[a.clone()], MAIN, &mut io);
        assert_eq!(io.registered.len(), 1);
        assert_eq!(r.route(key(&a.shortcut), MAIN), Route::Ignore);
        io.fail = false;
        let next = spec("a", 2, "Super+Alt+V");
        assert!(r.sync(&[next.clone()], MAIN, &mut io)["a"].registered);
        assert_eq!(io.registered.len(), 2);
        assert_eq!(r.route(key(&next.shortcut), MAIN), Route::Clipboard(next));
    }
    #[test]
    fn clipboard_shortcuts_removal_unregisters_only_its_key_and_config_revision_updates_current_route(
    ) {
        let mut r = Registry::default();
        let mut io = Fake::default();
        let a = spec("a", 1, "Super+Alt+V");
        let b = spec("b", 1, "Super+Alt+S");
        r.sync(&[a.clone(), b.clone()], MAIN, &mut io);
        let new_b = spec("b", 2, "Super+Alt+S");
        r.sync(&[new_b.clone()], MAIN, &mut io);
        assert_eq!(io.released, vec![key(&a.shortcut)]);
        assert_eq!(io.registered.len(), 2);
        assert_eq!(r.route(key(&b.shortcut), MAIN), Route::Clipboard(new_b));
        r.release(&mut io).unwrap();
        assert!(io.released.contains(&key(&b.shortcut)));
        assert_eq!(r.route(key(&b.shortcut), MAIN), Route::Ignore);
    }
    #[test]
    fn clipboard_shortcuts_failed_release_revokes_old_event_route_and_keeps_new_key_independent() {
        let mut r = Registry::default();
        let mut io = Fake::default();
        let a = spec("a", 1, "Super+Alt+V");
        r.sync(&[a.clone()], MAIN, &mut io);
        io.release_fail = true;
        let next = spec("a", 2, "Super+Alt+S");
        r.sync(&[next.clone()], MAIN, &mut io);
        assert_eq!(r.route(key(&a.shortcut), MAIN), Route::Ignore);
        assert_eq!(r.route(key(&next.shortcut), MAIN), Route::Clipboard(next));
        io.release_fail = false;
        r.release(&mut io).unwrap();
        assert!(
            io.released
                .iter()
                .filter(|&&k| k == key(&a.shortcut))
                .count()
                >= 2
        );
    }
}
