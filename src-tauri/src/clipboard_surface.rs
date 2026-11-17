//! 剪切板专用窗口的代次与动作归属；不创建窗口或访问系统。
use crate::{
    clipboard_actions::{Permit, Stage},
    clipboard_native::Target,
    store::AppResult,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    History,
    Search,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Opening,
    Visible,
    Acting,
    Hidden,
}
#[derive(Clone)]
pub(crate) struct Popup {
    pub instance: String,
    pub id: String,
    pub label: String,
    pub revision: u64,
    pub kind: Kind,
    pub owner: usize,
    pub target: Option<Target>,
    pub phase: Phase,
    pub focused: bool,
    pub hovered: bool,
    pub hide_at: Option<u64>,
    pub active: Option<String>,
    pub last: Option<String>,
    pub action_hidden: bool,
}
#[derive(Default)]
pub(crate) struct Book {
    pub entries: HashMap<String, Popup>,
    pub closing: Vec<String>,
}
pub(crate) fn role(label: &str, instance: &str) -> Option<Kind> {
    if uuid::Uuid::parse_str(instance).is_err() {
        return None;
    }
    for (kind, prefix) in [
        (Kind::History, "clipboard-history"),
        (Kind::Search, "clipboard-search"),
    ] {
        if let Some(id) = label.strip_prefix(&format!("{prefix}-{instance}-")) {
            if uuid::Uuid::parse_str(id).is_ok_and(|id_value| id_value.to_string() == id) {
                return Some(kind);
            }
        }
    }
    None
}
pub(crate) fn surface_method(label: &str, method: &str) -> AppResult<()> {
    if label.starts_with("clipboard-tool-") {
        return crate::clipboard_tools::method_allowed(label, method);
    }
    if label.starts_with("clipboard-")
        && !matches!(
            method,
            "runtime.ready"
                | "settings.get"
                | "settings.update"
                | "storage.get"
                | "storage.set"
                | "clipboard.status"
                | "clipboard.surface"
                | "clipboard.query"
                | "clipboard.get"
                | "clipboard.mutate"
                | "clipboard.peek"
                | "clipboard.actionStatus"
                | "clipboard.act"
                | "clipboard.hide"
                | "clipboard.hover"
                | "clipboard.search"
                | "clipboard.tool"
                | "clipboard.settings"
        )
    {
        return Err("CAPABILITY_DENIED: 剪切板专用窗口只接受固定组件交互".into());
    }
    Ok(())
}
impl Book {
    pub(crate) fn begin(
        &mut self,
        instance: &str,
        kind: Kind,
        revision: u64,
        target: Option<Target>,
        now: u64,
        automatic: bool,
    ) -> AppResult<Popup> {
        if uuid::Uuid::parse_str(instance).is_err() || automatic && kind != Kind::Search {
            return Err("INVALID_ARGUMENT: 剪切板呼出来源无效".into());
        }
        if automatic {
            if let Some(p) = self.entries.get_mut(instance).filter(|p| {
                p.kind == kind
                    && p.revision == revision
                    && matches!(p.phase, Phase::Opening | Phase::Visible)
            }) {
                if !p.focused && !p.hovered {
                    p.hide_at = Some(now.saturating_add(3000));
                }
                return Ok(p.clone());
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let prefix = if kind == Kind::History {
            "clipboard-history"
        } else {
            "clipboard-search"
        };
        let p = Popup {
            instance: instance.into(),
            label: format!("{prefix}-{instance}-{id}"),
            id,
            revision,
            kind,
            owner: 0,
            target,
            phase: Phase::Opening,
            focused: false,
            hovered: false,
            hide_at: None,
            active: None,
            last: None,
            action_hidden: false,
        };
        if let Some(old) = self.entries.insert(instance.into(), p.clone()) {
            self.closing.push(old.label);
        }
        Ok(p)
    }
    pub(crate) fn attach(&mut self, instance: &str, id: &str, owner: usize) -> AppResult<()> {
        let p = self
            .entries
            .get_mut(instance)
            .filter(|p| p.id == id && p.phase == Phase::Opening && owner != 0)
            .ok_or("PRESENTATION_EXPIRED: 剪切板窗口已替换或句柄无效")?;
        p.owner = owner;
        Ok(())
    }
    pub(crate) fn shown(&mut self, instance: &str, id: &str, now: u64) -> AppResult<()> {
        let p = self
            .entries
            .get_mut(instance)
            .filter(|p| p.id == id && p.owner != 0 && p.phase == Phase::Opening)
            .ok_or("PRESENTATION_EXPIRED: 剪切板显示已失效")?;
        p.phase = Phase::Visible;
        if p.kind == Kind::Search {
            p.hide_at = Some(now.saturating_add(3000));
        }
        Ok(())
    }
    pub(crate) fn current(&self, label: &str, instance: &str) -> bool {
        self.entries
            .get(instance)
            .is_some_and(|p| p.label == label && p.phase != Phase::Hidden)
    }
    pub(crate) fn close(&mut self, label: &str) {
        if let Some(p) = self.entries.values_mut().find(|p| p.label == label) {
            if p.phase != Phase::Hidden {
                self.closing.push(label.into());
            }
            p.phase = Phase::Hidden;
            p.target = None;
            p.owner = 0;
            p.active = None;
            p.last = None;
            p.hide_at = None;
            p.focused = false;
            p.hovered = false;
        }
    }
    pub(crate) fn interact(
        &mut self,
        label: &str,
        focused: bool,
        hovered: bool,
        now: u64,
    ) -> AppResult<()> {
        let p = self
            .entries
            .values_mut()
            .find(|p| p.label == label && p.phase != Phase::Hidden)
            .ok_or("PRESENTATION_EXPIRED: 剪切板交互窗口已失效")?;
        p.focused = focused;
        p.hovered = hovered;
        p.hide_at = if p.kind == Kind::Search && !focused && !hovered {
            Some(now.saturating_add(3000))
        } else {
            None
        };
        Ok(())
    }
    pub(crate) fn due(&self, now: u64) -> Vec<String> {
        let mut result = self
            .entries
            .values()
            .filter(|p| {
                p.kind == Kind::Search
                    && p.phase == Phase::Visible
                    && !p.focused
                    && !p.hovered
                    && p.hide_at.is_some_and(|time| time <= now)
            })
            .map(|p| p.label.clone())
            .collect::<Vec<_>>();
        result.sort();
        result
    }
    pub(crate) fn claim(
        &mut self,
        instance: &str,
        label: &str,
        presentation: &str,
        request: &str,
        revision: u64,
        stage: Stage,
        paste_enabled: bool,
    ) -> AppResult<Permit> {
        let p = self
            .entries
            .get_mut(instance)
            .filter(|p| {
                p.id == presentation
                    && p.label == label
                    && p.revision == revision
                    && p.kind == Kind::History
            })
            .ok_or("PRESENTATION_EXPIRED: 剪切板动作窗口已替换")?;
        if stage == Stage::Reserve {
            match p.phase {
                Phase::Visible => {
                    p.phase = Phase::Acting;
                    p.active = Some(request.into());
                    p.action_hidden = false;
                }
                Phase::Acting if p.active.as_deref() == Some(request) => {}
                Phase::Hidden if p.last.as_deref() == Some(request) => {}
                _ => return Err("PRESENTATION_EXPIRED: 不能从当前剪切板窗口发起动作".into()),
            }
        } else if p.phase != Phase::Acting || p.active.as_deref() != Some(request) {
            return Err("PRESENTATION_EXPIRED: 剪切板动作已撤销".into());
        }
        Ok(Permit {
            config_revision: revision,
            presentation_id: p.id.clone(),
            paste_enabled,
        })
    }
    pub(crate) fn action_hidden(
        &mut self,
        instance: &str,
        presentation: &str,
        request: &str,
    ) -> AppResult<()> {
        let p = self
            .entries
            .get_mut(instance)
            .filter(|p| {
                p.id == presentation
                    && p.phase == Phase::Acting
                    && p.active.as_deref() == Some(request)
            })
            .ok_or("PRESENTATION_EXPIRED: 待隐藏动作已失效")?;
        p.action_hidden = true;
        Ok(())
    }
    pub(crate) fn finish(&mut self, instance: &str, presentation: &str, request: &str) {
        if let Some(p) = self.entries.get_mut(instance).filter(|p| {
            p.id == presentation && p.phase == Phase::Acting && p.active.as_deref() == Some(request)
        }) {
            p.active = None;
            p.last = Some(request.into());
            p.phase = if p.action_hidden {
                Phase::Hidden
            } else {
                Phase::Visible
            };
        }
    }
    pub(crate) fn reconcile(&mut self, valid: &HashMap<String, u64>, hidden: bool) -> Vec<String> {
        let mut labels = Vec::new();
        for p in self.entries.values_mut() {
            if (hidden || valid.get(&p.instance) != Some(&p.revision)) && p.phase != Phase::Hidden {
                labels.push(p.label.clone());
                p.phase = Phase::Hidden;
                p.owner = 0;
                p.target = None;
                p.active = None;
                p.last = None;
                p.hide_at = None;
            }
        }
        self.entries
            .retain(|instance, _| valid.contains_key(instance));
        labels.sort();
        labels
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn open(book: &mut Book, id: &str, kind: Kind) -> Popup {
        let p = book.begin(id, kind, 1, None, 1000, false).unwrap();
        book.attach(id, &p.id, 42).unwrap();
        book.shown(id, &p.id, 1001).unwrap();
        p
    }
    #[test]
    fn clipboard_surface_exact_role_rejects_foreign_instance_and_malformed_generation() {
        let id = uid();
        let generation = uid();
        let h = format!("clipboard-history-{id}-{generation}");
        let s = format!("clipboard-search-{id}-{generation}");
        assert!(role(&h, &id) == Some(Kind::History));
        assert!(role(&s, &id) == Some(Kind::Search));
        assert!(role(&h, &uid()).is_none());
        assert!(role(&format!("{h}-extra"), &id).is_none());
        assert!(role(&format!("clipboard-history-{id}-bad"), &id).is_none());
    }
    #[test]
    fn clipboard_surface_new_generation_invalidates_old_window_attach_and_actions() {
        let mut book = Book::default();
        let id = uid();
        let first = open(&mut book, &id, Kind::History);
        assert!(book.current(&first.label, &id));
        let second = book
            .begin(&id, Kind::History, 1, None, 2000, false)
            .unwrap();
        assert_ne!(first.id, second.id);
        assert!(!book.current(&first.label, &id));
        assert!(book.attach(&id, &first.id, 44).is_err());
        assert!(book
            .claim(
                &id,
                &first.label,
                &first.id,
                &uid(),
                1,
                Stage::Reserve,
                true
            )
            .is_err());
        book.attach(&id, &second.id, 45).unwrap();
        book.shown(&id, &second.id, 2001).unwrap();
        assert!(book.current(&second.label, &id));
    }
    #[test]
    fn clipboard_surface_search_timeout_respects_focus_hover_and_automatic_refresh() {
        let mut book = Book::default();
        let id = uid();
        let p = open(&mut book, &id, Kind::Search);
        assert!(book.due(4000).is_empty());
        assert_eq!(book.due(4001), vec![p.label.clone()]);
        book.interact(&p.label, true, false, 2000).unwrap();
        assert!(book.due(10000).is_empty());
        let same = book.begin(&id, Kind::Search, 1, None, 10001, true).unwrap();
        assert_eq!(same.id, p.id);
        assert!(book.due(20000).is_empty());
        book.interact(&p.label, false, true, 20001).unwrap();
        assert!(book.due(30000).is_empty());
        book.interact(&p.label, false, false, 30001).unwrap();
        assert!(book.due(33000).is_empty());
        assert_eq!(book.due(33001), vec![p.label]);
    }
    #[test]
    fn clipboard_surface_hidden_and_configuration_replacement_reject_late_actions() {
        let mut book = Book::default();
        let id = uid();
        let p = open(&mut book, &id, Kind::History);
        book.close(&p.label);
        assert!(!book.current(&p.label, &id));
        assert!(book
            .claim(&id, &p.label, &p.id, &uid(), 1, Stage::Reserve, true)
            .is_err());
        let next = open(&mut book, &id, Kind::History);
        assert_eq!(
            book.reconcile(&HashMap::from([(id.clone(), 2)]), false),
            vec![next.label.clone()]
        );
        assert!(!book.current(&next.label, &id));
    }
    #[test]
    fn clipboard_surface_action_hiding_keeps_exact_request_until_finish_then_allows_only_replay() {
        let mut book = Book::default();
        let id = uid();
        let p = open(&mut book, &id, Kind::History);
        let request = uid();
        book.claim(&id, &p.label, &p.id, &request, 1, Stage::Reserve, true)
            .unwrap();
        book.action_hidden(&id, &p.id, &request).unwrap();
        book.claim(&id, &p.label, &p.id, &request, 1, Stage::Input, true)
            .unwrap();
        assert!(book
            .claim(&id, &p.label, &p.id, &uid(), 1, Stage::Input, true)
            .is_err());
        book.finish(&id, &p.id, &request);
        assert!(!book.current(&p.label, &id));
        book.claim(&id, &p.label, &p.id, &request, 1, Stage::Reserve, true)
            .unwrap();
        assert!(book
            .claim(&id, &p.label, &p.id, &uid(), 1, Stage::Reserve, true)
            .is_err());
        assert!(book
            .claim(&id, &p.label, &p.id, &request, 1, Stage::BeforeCopy, true)
            .is_err());
    }
    #[test]
    fn clipboard_surface_copy_without_hiding_returns_to_visible_and_regular_close_revokes_active_action(
    ) {
        let mut book = Book::default();
        let id = uid();
        let p = open(&mut book, &id, Kind::History);
        let request = uid();
        book.claim(&id, &p.label, &p.id, &request, 1, Stage::Reserve, false)
            .unwrap();
        book.finish(&id, &p.id, &request);
        assert!(book.current(&p.label, &id));
        let request = uid();
        book.claim(&id, &p.label, &p.id, &request, 1, Stage::Reserve, true)
            .unwrap();
        book.close(&p.label);
        assert!(book
            .claim(&id, &p.label, &p.id, &request, 1, Stage::Finish, true)
            .is_err());
    }
    #[test]
    fn clipboard_surface_reconcile_global_hide_or_removed_instance_does_not_affect_other_valid_window(
    ) {
        let mut book = Book::default();
        let a = uid();
        let b = uid();
        let first = open(&mut book, &a, Kind::History);
        let second = open(&mut book, &b, Kind::Search);
        assert_eq!(
            book.reconcile(&HashMap::from([(b.clone(), 1)]), false),
            vec![first.label]
        );
        assert!(book.current(&second.label, &b));
        assert_eq!(
            book.reconcile(&HashMap::from([(b.clone(), 1)]), true),
            vec![second.label]
        );
        assert!(book.due(u64::MAX).is_empty());
    }
    #[test]
    fn clipboard_surface_does_not_allow_generic_window_storage_system_or_foreign_ai_actions() {
        let label = format!("clipboard-history-{}-{}", uid(), uid());
        for method in [
            "runtime.ready",
            "clipboard.status",
            "clipboard.surface",
            "clipboard.act",
            "clipboard.hide",
            "clipboard.hover",
            "settings.get",
            "storage.get",
        ] {
            surface_method(&label, method).unwrap();
        }
        for method in [
            "view.openDetails",
            "keyboard.configure",
            "ai.begin",
            "library.bind",
            "documents.open",
            "system.metrics",
            "window.close",
        ] {
            assert!(surface_method(&label, method).is_err());
        }
        surface_method("float-normal", "market.poll").unwrap();
    }
}
