//! 三款历史后台组件的纯状态；不读取鼠标、时钟、数据库或系统通知。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SitConfig {
    pub enabled: bool,
    pub sit_interval: u32,
    pub break_interval: u32,
    pub mouse_check_interval: u32,
}
impl Default for SitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sit_interval: 30,
            break_interval: 5,
            mouse_check_interval: 5,
        }
    }
}
impl SitConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=120).contains(&self.sit_interval)
            || !(1..=60).contains(&self.break_interval)
            || !(1..=120).contains(&self.mouse_check_interval)
        {
            return Err("INVALID_ARGUMENT: 久坐/检测阈值须为1～120分钟，休息须为1～60分钟".into());
        }
        Ok(())
    }
}
/// 仅当前进程内的一次坐标基线，不序列化或输出轨迹。
#[derive(Default)]
pub(crate) struct SitUsage {
    previous: Option<(u64, (i32, i32))>,
    last_active_at: u64,
    used: u64,
}
impl SitUsage {
    pub fn used_ms(&self) -> u64 {
        self.used
    }
    pub fn has_sample(&self) -> bool {
        self.previous.is_some()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn sample(
        &mut self,
        at: u64,
        point: (i32, i32),
        config: &SitConfig,
        breaking: bool,
    ) -> bool {
        if !config.enabled || config.validate().is_err() {
            self.clear();
            return false;
        }
        let Some((previous, old_point)) = self.previous else {
            self.previous = Some((at, point));
            self.last_active_at = at;
            return false;
        };
        let elapsed = at.checked_sub(previous);
        // 10秒是原组件采样粒度；长间隙/倒退不按仍在电脑前累计。
        if elapsed.is_none_or(|delta| delta > 15000) {
            self.previous = Some((at, point));
            self.last_active_at = at;
            self.used = 0;
            return false;
        }
        let elapsed = elapsed.unwrap();
        if elapsed < 10000 {
            return false;
        }
        self.previous = Some((at, point));
        if breaking {
            self.used = 0;
            self.last_active_at = at;
            return false;
        }
        if at.saturating_sub(self.last_active_at)
            > u64::from(config.mouse_check_interval) * 60000 + 10000
        {
            self.used = 0;
        } else {
            self.used = self.used.saturating_add(elapsed);
        }
        if point != old_point {
            self.last_active_at = at;
        }
        if self.used >= u64::from(config.sit_interval) * 60000 {
            self.used = 0;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PhonePreset {
    Squidward,
    Tea,
    Pill,
    Study,
}
impl PhonePreset {
    pub fn advance_seconds(self) -> u8 {
        if self == Self::Squidward {
            7
        } else {
            0
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PhoneRule {
    pub preset: PhonePreset,
    pub enabled: bool,
    pub minute_of_day: u16,
    pub workdays: Vec<u8>,
}
#[derive(Clone, Copy)]
pub(crate) struct LocalMinute {
    pub day: i64,
    pub weekday: u8,
    pub minute: u16,
    pub second: u8,
}
impl PhoneRule {
    pub fn new(preset: PhonePreset) -> Self {
        Self {
            preset,
            enabled: false,
            minute_of_day: match preset {
                PhonePreset::Squidward => 1080,
                PhonePreset::Tea => 900,
                _ => 720,
            },
            workdays: vec![1, 2, 3, 4, 5],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        if self.minute_of_day >= 1440
            || self.workdays.len() > 7
            || self
                .workdays
                .iter()
                .any(|day| *day > 6 || !seen.insert(*day))
        {
            return Err("INVALID_ARGUMENT: 来电时间或工作日无效".into());
        }
        Ok(())
    }
    /// 精确匹配当次秒槽；不把重启/休眠后的时间当作原计划的实际到时。
    pub fn slot(&self, now: &LocalMinute) -> Result<Option<i64>, String> {
        self.validate()?;
        if now.weekday > 6 || now.minute >= 1440 || now.second >= 60 {
            return Err("DATA_UNAVAILABLE: 当前本地日期时间无效".into());
        }
        if !self.enabled {
            return Ok(None);
        }
        let advanced = u32::from(now.minute) * 60
            + u32::from(now.second)
            + u32::from(self.preset.advance_seconds());
        let tomorrow = advanced >= 86400;
        let target_weekday = (now.weekday + u8::from(tomorrow)) % 7;
        if advanced % 86400 != u32::from(self.minute_of_day) * 60
            || !self.workdays.contains(&target_weekday)
        {
            return Ok(None);
        }
        let day = now
            .day
            .checked_add(i64::from(tomorrow))
            .ok_or("DATA_UNAVAILABLE: 来电日期超出范围")?;
        Ok(Some(day))
    }
}
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhoneLedger {
    last_days: Vec<(PhonePreset, i64)>,
}
impl PhoneLedger {
    /// 同一预置/目标日只预留一次；调时与时间回退不得重放旧日槽。
    pub fn reserve(&mut self, preset: PhonePreset, day: i64) -> bool {
        if let Some((_, last)) = self.last_days.iter_mut().find(|(key, _)| *key == preset) {
            if day <= *last {
                return false;
            }
            *last = day;
            true
        } else {
            self.last_days.push((preset, day));
            true
        }
    }
}

#[derive(Default)]
pub(crate) struct IslandState {
    current: Option<IslandEvent>,
}
struct IslandEvent {
    id: String,
    source: String,
    created_at: u64,
    deadline: Option<u64>,
    hover_remaining: Option<u64>,
}
impl IslandState {
    pub fn current_id(&self) -> Option<&str> {
        self.current.as_ref().map(|event| event.id.as_str())
    }
    pub fn clear(&mut self) {
        self.current = None;
    }
    pub fn publish(&mut self, id: String, source: String, now: u64, duration: i64) {
        self.current = Some(IslandEvent {
            id,
            source,
            created_at: now,
            deadline: if duration > 0 {
                Some(now.saturating_add(duration as u64))
            } else {
                None
            },
            hover_remaining: None,
        });
    }
    pub fn action(&mut self, id: &str, now: u64) -> bool {
        if self.current.as_ref().is_some_and(|event| {
            event.id == id && now >= event.created_at && event.deadline.is_none_or(|at| now < at)
        }) {
            self.clear();
            true
        } else {
            false
        }
    }
    pub fn expire(&mut self, now: u64) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|event| event.deadline.is_some_and(|at| now >= at))
        {
            self.clear();
            true
        } else {
            false
        }
    }
    pub fn hover(&mut self, id: &str, now: u64, entered: bool) -> bool {
        let Some(event) = self
            .current
            .as_mut()
            .filter(|event| event.id == id && now >= event.created_at)
        else {
            return false;
        };
        if entered {
            if let Some(deadline) = event.deadline {
                if now >= deadline {
                    return false;
                }
                event.hover_remaining = Some(deadline - now);
                event.deadline = None;
            }
        } else if let Some(remaining) = event.hover_remaining.take() {
            event.deadline = Some(now.saturating_add(remaining));
        }
        true
    }
    pub fn remove_source(&mut self, source: &str) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|event| event.source == source)
        {
            self.clear();
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
    fn sit_model_disabled_does_not_keep_coordinates_or_count() {
        let mut state = SitUsage::default();
        let config = SitConfig::default();
        for at in [0, 10000, 20000] {
            assert!(!state.sample(at, (22, 33), &config, false));
        }
        assert_eq!(state.used_ms(), 0);
        assert!(!state.has_sample());
    }
    #[test]
    fn sit_model_uses_original_ten_second_mouse_activity_and_resets_after_due() {
        let mut state = SitUsage::default();
        let config = SitConfig {
            enabled: true,
            sit_interval: 1,
            ..Default::default()
        };
        assert!(!state.sample(0, (0, 0), &config, false));
        for n in 1..6 {
            assert!(!state.sample(n * 10000, (n as i32, 0), &config, false));
        }
        assert!(state.sample(60000, (6, 0), &config, false));
        assert_eq!(state.used_ms(), 0);
        assert!(!state.sample(70000, (7, 0), &config, false));
        assert_eq!(state.used_ms(), 10000);
    }
    #[test]
    fn sit_model_idle_break_and_sleep_gap_are_not_continuous_activity() {
        let config = SitConfig {
            enabled: true,
            sit_interval: 120,
            mouse_check_interval: 1,
            ..Default::default()
        };
        let mut state = SitUsage::default();
        state.sample(0, (1, 1), &config, false);
        for n in 1..=8 {
            state.sample(n * 10000, (1, 1), &config, false);
        }
        assert_eq!(state.used_ms(), 0);
        state.sample(90000, (2, 2), &config, false);
        state.sample(100000, (3, 3), &config, false);
        assert_eq!(state.used_ms(), 10000);
        state.sample(110000, (4, 4), &config, true);
        assert_eq!(state.used_ms(), 0);
        state.sample(120000, (5, 5), &config, false);
        state.sample(300000, (6, 6), &config, false);
        assert_eq!(state.used_ms(), 0);
        state.sample(299000, (7, 7), &config, false);
        assert_eq!(state.used_ms(), 0);
    }
    #[test]
    fn sit_config_has_original_ranges_and_requires_explicit_enable() {
        let c = SitConfig::default();
        assert!(!c.enabled);
        assert_eq!(
            (c.sit_interval, c.break_interval, c.mouse_check_interval),
            (30, 5, 5)
        );
        assert!(c.validate().is_ok());
        for field in ["sitInterval", "breakInterval", "mouseCheckInterval"] {
            let mut value = serde_json::to_value(&c).unwrap();
            value[field] = serde_json::json!(0);
            assert!(serde_json::from_value::<SitConfig>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut value = serde_json::to_value(&c).unwrap();
        value["url"] = serde_json::json!("https://example.com");
        assert!(serde_json::from_value::<SitConfig>(value).is_err());
    }
    #[test]
    fn phone_slots_use_target_day_and_advance_time_across_midnight() {
        let rule = PhoneRule {
            preset: PhonePreset::Squidward,
            enabled: true,
            minute_of_day: 0,
            workdays: vec![1],
        };
        let previous = LocalMinute {
            day: 100,
            weekday: 0,
            minute: 1439,
            second: 53,
        };
        let target = LocalMinute {
            day: 101,
            weekday: 1,
            minute: 0,
            second: 0,
        };
        assert_eq!(rule.slot(&previous).unwrap(), Some(101));
        assert_eq!(rule.slot(&target).unwrap(), None);
        let inactive = PhoneRule {
            workdays: vec![0],
            ..rule
        };
        assert_eq!(inactive.slot(&previous).unwrap(), None);
    }
    #[test]
    fn phone_slots_do_not_catch_up_after_time_has_passed_or_run_on_wrong_day() {
        let rule = PhoneRule {
            preset: PhonePreset::Tea,
            enabled: true,
            minute_of_day: 900,
            workdays: vec![1, 2, 3, 4, 5],
        };
        assert_eq!(
            rule.slot(&LocalMinute {
                day: 100,
                weekday: 1,
                minute: 900,
                second: 0
            })
            .unwrap(),
            Some(100)
        );
        assert_eq!(
            rule.slot(&LocalMinute {
                day: 100,
                weekday: 1,
                minute: 900,
                second: 1
            })
            .unwrap(),
            None
        );
        assert_eq!(
            rule.slot(&LocalMinute {
                day: 100,
                weekday: 0,
                minute: 900,
                second: 0
            })
            .unwrap(),
            None
        );
    }
    #[test]
    fn phone_ledger_reserves_before_delivery_and_rejects_clock_rollback_or_changed_time_same_day() {
        let mut ledger = PhoneLedger::default();
        assert!(ledger.reserve(PhonePreset::Tea, 100));
        assert!(!ledger.reserve(PhonePreset::Tea, 100));
        assert!(!ledger.reserve(PhonePreset::Tea, 99));
        assert!(ledger.reserve(PhonePreset::Pill, 100));
        let restored: PhoneLedger =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        let mut ledger = restored;
        assert!(!ledger.reserve(PhonePreset::Tea, 100));
        assert!(ledger.reserve(PhonePreset::Tea, 101));
    }
    #[test]
    fn phone_config_only_accepts_fixed_presets_and_valid_schedule() {
        for preset in [
            PhonePreset::Squidward,
            PhonePreset::Tea,
            PhonePreset::Pill,
            PhonePreset::Study,
        ] {
            let r = PhoneRule::new(preset);
            assert!(!r.enabled);
            assert!(r.validate().is_ok());
        }
        let mut r = PhoneRule::new(PhonePreset::Tea);
        r.workdays = vec![1, 1];
        assert!(r.validate().is_err());
        r.workdays = vec![7];
        assert!(r.validate().is_err());
        r.workdays = vec![1];
        r.minute_of_day = 1440;
        assert!(r.validate().is_err());
        assert!(serde_json::from_value::<PhoneRule>(
            serde_json::json!({"preset":"arbitrary","enabled":true,"minuteOfDay":1,"workdays":[]})
        )
        .is_err());
    }
    #[test]
    fn island_late_close_or_expired_action_cannot_affect_replacement() {
        let mut state = IslandState::default();
        state.publish("a".into(), "source-a".into(), 10, 1000);
        assert!(state.action("a", 20));
        state.publish("b".into(), "source-b".into(), 30, 1000);
        assert!(!state.action("a", 40));
        assert!(!state.action("b", 1030));
        assert_eq!(state.current_id(), Some("b"));
        assert!(state.expire(1030));
        assert!(state.current_id().is_none());
    }
    #[test]
    fn island_hover_pauses_positive_deadline_but_not_a_forever_call() {
        let mut state = IslandState::default();
        state.publish("a".into(), "source".into(), 0, 1000);
        assert!(state.hover("a", 500, true));
        assert!(!state.expire(2000));
        assert!(state.hover("a", 2000, false));
        assert!(!state.expire(2499));
        assert!(state.expire(2500));
        state.publish("call".into(), "source".into(), 3000, -1);
        assert!(!state.expire(100000));
        assert!(state.action("call", 100000));
    }
    #[test]
    fn island_remove_only_target_source_and_drop_all_on_disable() {
        let mut state = IslandState::default();
        state.publish("a".into(), "source-a".into(), 0, 1000);
        assert!(!state.remove_source("source-b"));
        assert!(state.remove_source("source-a"));
        assert!(state.current_id().is_none());
        state.publish("b".into(), "source-b".into(), 0, 1000);
        state.clear();
        assert!(!state.action("b", 1));
    }
}
