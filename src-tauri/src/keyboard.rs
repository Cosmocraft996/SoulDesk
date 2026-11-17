//! 显式启用的快捷键演示；不记录普通文字或输入历史。
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Key {
    key_code: u16,
    name: String,
    is_key_up: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub visible: bool,
    modifier_keys: Vec<Key>,
    normal_key: Option<Key>,
    caps_lock: Option<Key>,
    is_caps_lock_on: bool,
}
pub(crate) struct KeyboardState {
    value: Snapshot,
    // 只保留本轮组合的物理键码；不保存普通文字、时间线或历史组合。
    held: BTreeSet<u16>,
    deadline: Option<u64>,
}
fn modifier(code: u16) -> Option<(u16, &'static str)> {
    match code {
        16 | 160 | 161 => Some((160, "Shift")),
        17 | 162 | 163 => Some((162, "Ctrl")),
        18 | 164 | 165 => Some((164, "Alt")),
        91 | 92 => Some((91, "Win")),
        _ => None,
    }
}
impl KeyboardState {
    pub fn new(caps: bool) -> Self {
        Self {
            value: Snapshot {
                is_caps_lock_on: caps,
                ..Default::default()
            },
            held: BTreeSet::new(),
            deadline: None,
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.value.clone()
    }
    pub fn reconcile_released(&mut self, mut is_down: impl FnMut(u16) -> bool, now: u64) -> bool {
        let released = self
            .held
            .iter()
            .copied()
            .filter(|code| !is_down(*code))
            .collect::<Vec<_>>();
        let mut changed = false;
        for code in released {
            changed |= self.input(code, true, "", now);
        }
        changed
    }
    pub fn demo(&mut self, now: u64) {
        self.value.visible = true;
        self.value.normal_key = Some(Key {
            key_code: 0,
            name: "😎".into(),
            is_key_up: true,
        });
        self.deadline = Some(now.saturating_add(4000));
    }
    pub fn accepts(&self, code: u16, up: bool) -> bool {
        modifier(code).is_some()
            || code == 20
            || if up {
                self.held.contains(&code)
            } else {
                self.held.iter().any(|code| modifier(*code).is_some())
            }
    }
    pub fn input(&mut self, code: u16, up: bool, name: &str, now: u64) -> bool {
        if !self.accepts(code, up) {
            return false;
        }
        let previous = self.value.clone();
        if up {
            if !self.held.remove(&code) {
                return false;
            }
            if let Some((canonical, _)) = modifier(code) {
                let released = !self
                    .held
                    .iter()
                    .any(|key| modifier(*key).is_some_and(|m| m.0 == canonical));
                if let Some(key) = self
                    .value
                    .modifier_keys
                    .iter_mut()
                    .find(|key| key.key_code == canonical)
                {
                    key.is_key_up = released;
                }
            } else if code == 20 {
                if let Some(key) = self.value.caps_lock.as_mut() {
                    key.is_key_up = true;
                }
            } else if let Some(key) = self
                .value
                .normal_key
                .as_mut()
                .filter(|key| key.key_code == code)
            {
                key.is_key_up = true;
            }
            if self.held.is_empty() {
                self.deadline = Some(now.saturating_add(4000));
            }
        } else {
            let repeated = self.held.contains(&code);
            if !repeated
                && modifier(code).is_some()
                && !self.held.iter().any(|code| modifier(*code).is_some())
            {
                self.value.modifier_keys.clear();
                self.value.normal_key = None;
                self.value.caps_lock = None;
                self.held.retain(|code| *code == 20);
            }
            self.held.insert(code);
            self.deadline = None;
            self.value.visible = true;
            if let Some((canonical, label)) = modifier(code) {
                if let Some(key) = self
                    .value
                    .modifier_keys
                    .iter_mut()
                    .find(|key| key.key_code == canonical)
                {
                    key.is_key_up = false;
                } else {
                    self.value.modifier_keys.push(Key {
                        key_code: canonical,
                        name: label.into(),
                        is_key_up: false,
                    });
                }
            } else if code == 20 {
                if !repeated {
                    self.value.is_caps_lock_on = !self.value.is_caps_lock_on;
                }
                self.value.caps_lock = Some(Key {
                    key_code: code,
                    name: "Caps".into(),
                    is_key_up: false,
                });
            } else {
                self.value.normal_key = Some(Key {
                    key_code: code,
                    name: name.chars().take(32).collect(),
                    is_key_up: false,
                });
            }
        }
        previous != self.value
    }
    pub fn expire(&mut self, now: u64) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.value.visible = false;
            self.value.modifier_keys.clear();
            self.value.normal_key = None;
            self.value.caps_lock = None;
            self.held.clear();
            self.deadline = None;
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_missed_release_is_reconciled_before_plain_text_can_be_interpreted_as_a_combo() {
        let mut state = KeyboardState::new(false);
        state.input(162, false, "Ctrl", 0);
        state.input(67, false, "C", 1);
        assert!(state.reconcile_released(|_| false, 100));
        assert!(state.snapshot().modifier_keys[0].is_key_up);
        assert!(state.snapshot().normal_key.as_ref().unwrap().is_key_up);
        assert!(!state.input(65, false, "不得保留的普通输入", 101));
        assert_eq!(state.snapshot().normal_key.as_ref().unwrap().name, "C");
        assert!(state.expire(4100));
        assert!(state.snapshot().normal_key.is_none());
    }
    #[test]
    fn keyboard_start_example_expires_without_an_input_history() {
        let mut state = KeyboardState::new(false);
        state.demo(1000);
        assert_eq!(state.snapshot().normal_key.unwrap().name, "😎");
        assert!(!state.expire(4999));
        assert!(state.expire(5000));
        assert!(state.snapshot().normal_key.is_none());
    }
    #[test]
    fn keyboard_plain_text_never_becomes_a_snapshot() {
        let mut state = KeyboardState::new(false);
        for code in [65, 66, 48, 13, 32] {
            assert!(!state.input(code, false, "不应保留的普通输入", 0));
            assert!(!state.input(code, true, "不应保留的普通输入", 1));
        }
        assert!(!state.snapshot().visible);
        assert!(state.snapshot().modifier_keys.is_empty());
        assert!(state.snapshot().normal_key.is_none());
    }
    #[test]
    fn keyboard_combo_keeps_release_state_then_clears_after_four_seconds() {
        let mut state = KeyboardState::new(false);
        assert!(state.input(162, false, "Left Control", 100));
        state.input(67, false, "C", 110);
        assert_eq!(state.snapshot().modifier_keys[0].name, "Ctrl");
        assert_eq!(state.snapshot().normal_key.as_ref().unwrap().name, "C");
        assert!(!state.snapshot().normal_key.as_ref().unwrap().is_key_up);
        state.input(67, true, "C", 120);
        state.input(162, true, "Left Control", 150);
        assert!(state.snapshot().normal_key.as_ref().unwrap().is_key_up);
        assert!(!state.expire(4149));
        assert!(state.expire(4150));
        assert!(!state.snapshot().visible);
        assert!(state.snapshot().normal_key.is_none());
    }
    #[test]
    fn keyboard_left_right_modifiers_share_a_key_but_release_independently() {
        let mut state = KeyboardState::new(false);
        state.input(162, false, "left", 0);
        state.input(163, false, "right", 1);
        assert_eq!(state.snapshot().modifier_keys.len(), 1);
        state.input(162, true, "left", 2);
        assert!(!state.snapshot().modifier_keys[0].is_key_up);
        assert!(!state.expire(5000));
        state.input(163, true, "right", 5001);
        assert!(state.expire(9001));
    }
    #[test]
    fn keyboard_caps_uses_initial_state_and_does_not_toggle_on_repeat() {
        let mut state = KeyboardState::new(true);
        state.input(20, false, "Caps", 0);
        assert!(!state.snapshot().is_caps_lock_on);
        state.input(20, false, "Caps", 1);
        assert!(!state.snapshot().is_caps_lock_on);
        state.input(20, true, "Caps", 10);
        assert!(state.snapshot().caps_lock.as_ref().unwrap().is_key_up);
        assert!(state.expire(4010));
        state.input(20, false, "Caps", 4020);
        assert!(state.snapshot().is_caps_lock_on);
    }
    #[test]
    fn keyboard_old_normal_key_up_does_not_release_latest_key_or_leak_plain_text() {
        let mut state = KeyboardState::new(false);
        state.input(160, false, "Shift", 0);
        state.input(65, false, "A", 1);
        state.input(66, false, "B", 2);
        state.input(65, true, "A", 3);
        assert_eq!(state.snapshot().normal_key.as_ref().unwrap().name, "B");
        assert!(!state.snapshot().normal_key.as_ref().unwrap().is_key_up);
        state.input(66, true, "B", 4);
        state.input(160, true, "Shift", 5);
        state.expire(4005);
        state.input(67, false, "不应保存", 4006);
        assert!(state.snapshot().normal_key.is_none());
    }
}
