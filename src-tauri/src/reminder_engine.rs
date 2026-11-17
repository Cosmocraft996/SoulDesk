//! 灵动通知、久坐与来电的核心协调；原生输入通过显式回调注入。
use crate::{
    reminder_state::{
        IslandState, LocalMinute, PhoneLedger, PhonePreset, PhoneRule, SitConfig, SitUsage,
    },
    reminder_store::{self, InstanceConfig, Source},
    store::{err, AppResult, Store},
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) struct Clock {
    pub wall_ms: u64,
    pub monotonic_ms: u64,
    pub local: LocalMinute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NoticeKind {
    Info,
    Success,
    Error,
    Warning,
    Reminder,
    AdvanceCountdown,
    Countdown,
    Call,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Demo {
    Info,
    Success,
    Error,
    Warning,
    Reminder,
    AdvanceCountdown,
    Countdown,
    Call,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Action {
    Dismiss,
    Confirm,
    Countdown,
    Hangup,
    Answer,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    Notice,
    Rest,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub surface: Surface,
    pub owner: String,
    pub event: String,
    pub generation: u64,
    pub height: u32,
}
impl Target {
    pub fn label(&self) -> String {
        format!(
            "{}-{}-{}",
            if self.surface == Surface::Notice {
                "island"
            } else {
                "break"
            },
            self.owner,
            self.generation
        )
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notice {
    pub id: String,
    pub source: String,
    pub kind: NoticeKind,
    pub title: String,
    pub message: String,
    pub preset: Option<PhonePreset>,
    pub target_at: Option<u64>,
    pub created_at: u64,
    pub duration: i64,
    pub mute: bool,
    pub phase: String,
    #[serde(skip)]
    pub source_revision: u64,
    #[serde(skip)]
    preview: bool,
    #[serde(skip)]
    break_seconds: Option<u32>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rest {
    pub id: String,
    pub source: String,
    pub deadline: u64,
    pub started_at: u64,
    #[serde(skip)]
    source_revision: u64,
}
#[derive(Default)]
pub(crate) struct Engine {
    configs: Vec<InstanceConfig>,
    provider: Option<(String, u64)>,
    notice: Option<Notice>,
    rest: Option<Rest>,
    island: IslandState,
    usages: HashMap<String, (u64, SitUsage)>,
    last_cursor: Option<u64>,
    notice_generation: u64,
    rest_generation: u64,
    hidden: bool,
    errors: HashMap<String, String>,
    configuration_errors: HashMap<String, String>,
    delivery: HashMap<String, String>,
}
impl Engine {
    pub fn notification(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }
    pub fn rest(&self) -> Option<&Rest> {
        self.rest.as_ref()
    }
    pub fn target(&self, surface: Surface) -> Option<Target> {
        if self.hidden {
            return None;
        }
        match surface {
            Surface::Notice => {
                let notice = self.notice.as_ref()?;
                let (owner, _) = self.provider.as_ref()?;
                Some(Target {
                    surface,
                    owner: owner.clone(),
                    event: notice.id.clone(),
                    generation: self.notice_generation,
                    height: match notice.kind {
                        NoticeKind::Reminder => 160,
                        NoticeKind::Call | NoticeKind::AdvanceCountdown => 88,
                        _ => 64,
                    },
                })
            }
            Surface::Rest => {
                let rest = self.rest.as_ref()?;
                Some(Target {
                    surface,
                    owner: rest.source.clone(),
                    event: rest.id.clone(),
                    generation: self.rest_generation,
                    height: 0,
                })
            }
        }
    }
    pub fn status(&self, id: &str) -> Value {
        json!({"usedMilliseconds":self.usages.get(id).map(|(_,usage)|usage.used_ms()).unwrap_or(0),"providerAvailable":self.provider.is_some(),"lastError":self.configuration_errors.get(id).or_else(||self.errors.get(id)),"delivery":self.delivery.get(id),"breakActive":self.rest.as_ref().is_some_and(|rest|rest.source==id)})
    }
    pub fn clear(&mut self) {
        self.clear_notice();
        self.clear_rest();
        self.usages.clear();
        self.last_cursor = None;
    }
    pub fn cancel(&mut self, surface: Surface) {
        if surface == Surface::Notice {
            self.clear_notice();
        } else {
            self.clear_rest();
        }
    }
    fn clear_notice(&mut self) {
        self.notice = None;
        self.island.clear();
        self.notice_generation = self.notice_generation.wrapping_add(1);
    }
    fn clear_rest(&mut self) {
        if let Some(rest) = self.rest.take() {
            if let Some((_, usage)) = self.usages.get_mut(&rest.source) {
                usage.clear();
            }
        }
        self.rest_generation = self.rest_generation.wrapping_add(1);
    }
    pub fn invalidate(&mut self, id: &str) {
        self.usages.remove(id);
        if self.provider.as_ref().is_some_and(|(owner, _)| owner == id)
            || self
                .notice
                .as_ref()
                .is_some_and(|notice| notice.source == id)
        {
            self.clear_notice();
        }
        if self.rest.as_ref().is_some_and(|rest| rest.source == id) {
            self.clear_rest();
        }
    }
    fn sync(&mut self, store: &Store, clock: Clock) -> AppResult<()> {
        let scan = reminder_store::configurations(&*store.db.lock().map_err(err)?)?;
        self.configs = scan.valid;
        self.configuration_errors = scan.failures;
        let next = self
            .provider
            .as_ref()
            .and_then(|(owner, _)| {
                self.configs.iter().find(|c| {
                    c.source == Source::Island && c.id == *owner && c.config["enabled"] == true
                })
            })
            .or_else(|| {
                self.configs
                    .iter()
                    .find(|c| c.source == Source::Island && c.config["enabled"] == true)
            })
            .map(|c| (c.id.clone(), c.revision));
        if next != self.provider {
            self.clear_notice();
            self.provider = next;
        }
        let valid = |id: &str, revision: u64, preview: bool| {
            self.configs.iter().any(|c| {
                c.id == id && c.revision == revision && (preview || c.config["enabled"] == true)
            })
        };
        let invalid_notice = self
            .notice
            .as_ref()
            .is_some_and(|n| !valid(&n.source, n.source_revision, n.preview));
        let invalid_rest = self.rest.as_ref().is_some_and(|r| {
            !valid(&r.source, r.source_revision, false)
                || clock.wall_ms < r.started_at
                || clock.wall_ms >= r.deadline
        });
        if invalid_notice {
            self.clear_notice();
        }
        if invalid_rest {
            self.clear_rest();
        }
        self.usages.retain(|id, (revision, _)| {
            self.configs.iter().any(|c| {
                c.source == Source::Sit
                    && c.id == *id
                    && c.revision == *revision
                    && c.config["enabled"] == true
            })
        });
        if self.island.expire(clock.monotonic_ms) {
            self.clear_notice();
        }
        Ok(())
    }
    fn mark(
        &mut self,
        store: &Store,
        id: &str,
        state: &str,
        error: Option<String>,
        now: u64,
    ) -> AppResult<()> {
        let value = json!({"state":state,"error":error,"at":now});
        store.db.lock().map_err(err)?.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("reminder.delivery/{id}"),value.to_string()]).map_err(err)?;
        self.remember(id, state, error);
        Ok(())
    }
    fn remember(&mut self, id: &str, state: &str, error: Option<String>) {
        self.delivery.insert(id.into(), state.into());
        if let Some(error) = error {
            self.errors.insert(id.into(), error);
        } else {
            self.errors.remove(id);
        }
    }
    fn publish(&mut self, store: &Store, notice: Notice, clock: Clock) -> AppResult<()> {
        let id = notice.source.clone();
        let result = self.publish_saved(store, notice, clock);
        if let Err(error) = &result {
            if !error.starts_with("DATA_UNAVAILABLE:") {
                self.remember(
                    &id,
                    "delivery-failed",
                    Some(format!("提醒递送未保存：{error}")),
                );
            }
        }
        result
    }
    fn publish_saved(&mut self, store: &Store, notice: Notice, clock: Clock) -> AppResult<()> {
        if self.hidden || self.provider.is_none() {
            let message = if self.hidden {
                "隐藏全部期间未显示专用提醒"
            } else {
                "请添加并显式启用灵动通知容器"
            };
            self.mark(
                store,
                &notice.source,
                "not-shown",
                Some(message.into()),
                clock.wall_ms,
            )?;
            return Err(format!("DATA_UNAVAILABLE: {message}"));
        }
        let old_source = self.notice.as_ref().map(|old| old.source.clone());
        {
            // 替换与排队必须共同提交，失败不能留下“已替换但旧来电仍显示”的状态。
            let mut db = store.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            for (id, state) in old_source
                .as_deref()
                .map(|id| (id, "replaced"))
                .into_iter()
                .chain(std::iter::once((notice.source.as_str(), "queued")))
            {
                let value = json!({"state":state,"error":Value::Null,"at":clock.wall_ms});
                tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("reminder.delivery/{id}"),value.to_string()]).map_err(err)?;
            }
            tx.commit().map_err(err)?;
        }
        if let Some(id) = old_source {
            self.remember(&id, "replaced", None);
        }
        self.remember(&notice.source, "queued", None);
        self.notice_generation = self.notice_generation.wrapping_add(1);
        self.island.publish(
            notice.id.clone(),
            notice.source.clone(),
            clock.monotonic_ms,
            notice.duration,
        );
        self.notice = Some(notice);
        Ok(())
    }
    fn phone_notice(
        id: &str,
        revision: u64,
        preset: PhonePreset,
        clock: Clock,
        preview: bool,
        mute: bool,
    ) -> Notice {
        let (title, contact) = match preset {
            PhonePreset::Squidward => ("下班提醒", "章鱼哥"),
            PhonePreset::Tea => ("饮茶提醒", "饮茶哥"),
            PhonePreset::Pill => ("吃药提醒", "潘金莲"),
            PhonePreset::Study => ("学习提醒", "成龙"),
        };
        Notice {
            id: Uuid::new_v4().to_string(),
            source: id.into(),
            source_revision: revision,
            kind: NoticeKind::Call,
            title: title.into(),
            message: contact.into(),
            preset: Some(preset),
            target_at: None,
            created_at: clock.wall_ms,
            duration: -1,
            mute,
            phase: "playing".into(),
            preview,
            break_seconds: None,
        }
    }
    pub fn tick(
        &mut self,
        store: &Store,
        clock: Clock,
        hidden: bool,
        cursor: &mut impl FnMut() -> AppResult<(i32, i32)>,
    ) -> AppResult<()> {
        self.hidden = hidden;
        self.sync(store, clock)?;
        if hidden {
            self.clear_notice();
            self.clear_rest();
        }
        let sits = self
            .configs
            .iter()
            .filter(|c| c.source == Source::Sit && c.config["enabled"] == true)
            .map(|c| (c.id.clone(), c.revision, c.config.clone()))
            .collect::<Vec<_>>();
        if sits.is_empty() {
            self.usages.clear();
            self.last_cursor = None;
        } else if self.last_cursor.is_none_or(|last| {
            clock
                .monotonic_ms
                .checked_sub(last)
                .is_none_or(|delta| delta >= 10000)
        }) {
            self.last_cursor = Some(clock.monotonic_ms);
            let point = cursor();
            for (id, revision, config) in sits {
                let config: SitConfig = serde_json::from_value(config).map_err(err)?;
                let usage = &mut self
                    .usages
                    .entry(id.clone())
                    .or_insert_with(|| (revision, SitUsage::default()))
                    .1;
                match point.as_ref() {
                    Ok(point) => {
                        let breaking = self.rest.as_ref().is_some_and(|rest| rest.source == id);
                        let due = usage.sample(clock.monotonic_ms, *point, &config, breaking);
                        self.errors.remove(&id);
                        if due {
                            let notice = Notice {
                                id: Uuid::new_v4().to_string(),
                                source: id.clone(),
                                source_revision: revision,
                                kind: NoticeKind::Reminder,
                                title: "久坐提醒".into(),
                                message: format!("您已经连续使用电脑{}分钟", config.sit_interval),
                                preset: None,
                                target_at: None,
                                created_at: clock.wall_ms,
                                duration: 10000,
                                mute: true,
                                phase: String::new(),
                                preview: false,
                                break_seconds: Some(config.break_interval * 60),
                            };
                            if let Err(error) = self.publish(store, notice, clock) {
                                self.errors.entry(id.clone()).or_insert(error);
                            }
                        }
                    }
                    Err(error) => {
                        usage.clear();
                        self.mark(
                            store,
                            &id,
                            "input-unavailable",
                            Some(format!("鼠标状态读取失败：{error}")),
                            clock.wall_ms,
                        )?;
                    }
                }
            }
        }
        let phones = self
            .configs
            .iter()
            .filter(|c| c.source == Source::Phone && c.config["enabled"] == true)
            .map(|c| (c.id.clone(), c.revision, c.config.clone()))
            .collect::<Vec<_>>();
        for (id, revision, config) in phones {
            let rules: Vec<PhoneRule> =
                serde_json::from_value(config["rules"].clone()).map_err(err)?;
            let due = (|| -> AppResult<Vec<PhonePreset>> {
                let mut db = store.db.lock().map_err(err)?;
                let tx = db.transaction().map_err(err)?;
                let key = format!("reminder.phone-ledger/{id}");
                let raw: Option<String> = tx
                    .query_row(
                        "SELECT value FROM app_settings WHERE key=?1",
                        [&key],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(err)?;
                let mut ledger = match raw {
                    Some(raw) if raw.len() <= 4096 => serde_json::from_str::<PhoneLedger>(&raw)
                        .map_err(|_| "DATA_UNAVAILABLE: 来电日槽损坏，未重播")?,
                    Some(_) => return Err("DATA_UNAVAILABLE: 来电日槽超限".into()),
                    None => PhoneLedger::default(),
                };
                let mut due = Vec::new();
                for rule in rules {
                    if let Some(day) = rule.slot(&clock.local)? {
                        if ledger.reserve(rule.preset, day) {
                            due.push(rule.preset);
                        }
                    }
                }
                if !due.is_empty() {
                    tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,serde_json::to_string(&ledger).map_err(err)?]).map_err(err)?;
                }
                tx.commit().map_err(err)?;
                Ok(due)
            })();
            let due = match due {
                Ok(due) => due,
                Err(error) if error.starts_with("DATA_UNAVAILABLE:") => {
                    self.invalidate(&id);
                    self.remember(&id, "source-failed", Some(error));
                    continue;
                }
                Err(error) => return Err(error),
            };
            for preset in due {
                if let Err(error) = self.publish(
                    store,
                    Self::phone_notice(&id, revision, preset, clock, false, false),
                    clock,
                ) {
                    self.errors.entry(id.clone()).or_insert(error);
                }
            }
        }
        Ok(())
    }
    pub fn demo(&mut self, store: &Store, id: &str, demo: Demo, clock: Clock) -> AppResult<()> {
        self.sync(store, clock)?;
        let revision = self
            .configs
            .iter()
            .find(|c| c.id == id && c.source == Source::Island && c.config["enabled"] == true)
            .map(|c| c.revision)
            .ok_or("CAPABILITY_DENIED: 请先启用当前灵动通知组件")?;
        if matches!(demo, Demo::Call) {
            return self.publish(
                store,
                Self::phone_notice(id, revision, PhonePreset::Squidward, clock, true, true),
                clock,
            );
        }
        let kind = match demo {
            Demo::Info => NoticeKind::Info,
            Demo::Success => NoticeKind::Success,
            Demo::Error => NoticeKind::Error,
            Demo::Warning => NoticeKind::Warning,
            Demo::Reminder => NoticeKind::Reminder,
            Demo::AdvanceCountdown => NoticeKind::AdvanceCountdown,
            Demo::Countdown => NoticeKind::Countdown,
            Demo::Call => unreachable!(),
        };
        self.publish(
            store,
            Notice {
                id: Uuid::new_v4().to_string(),
                source: id.into(),
                source_revision: revision,
                kind,
                title: if kind == NoticeKind::Reminder {
                    "久坐提醒"
                } else {
                    "通知演示"
                }
                .into(),
                message: if kind == NoticeKind::Reminder {
                    "您已经连续使用电脑45分钟"
                } else {
                    "您好"
                }
                .into(),
                preset: None,
                target_at: matches!(kind, NoticeKind::AdvanceCountdown | NoticeKind::Countdown)
                    .then_some(clock.wall_ms + 60000),
                created_at: clock.wall_ms,
                duration: if kind == NoticeKind::Countdown {
                    60000
                } else {
                    10000
                },
                mute: true,
                phase: String::new(),
                preview: false,
                break_seconds: None,
            },
            clock,
        )
    }
    pub fn preview(
        &mut self,
        store: &Store,
        id: &str,
        preset: PhonePreset,
        clock: Clock,
    ) -> AppResult<()> {
        self.sync(store, clock)?;
        let revision = self
            .configs
            .iter()
            .find(|c| c.id == id && c.source == Source::Phone)
            .map(|c| c.revision)
            .ok_or("CAPABILITY_DENIED: 来电组件不存在")?;
        self.publish(
            store,
            Self::phone_notice(id, revision, preset, clock, true, false),
            clock,
        )
    }
    pub(crate) fn validate_target(
        &mut self,
        store: &Store,
        target: &Target,
        id: &str,
        clock: Clock,
    ) -> AppResult<()> {
        self.sync(store, clock)?;
        if self.target(target.surface).as_ref() != Some(target) || target.event != id {
            return Err("SESSION_EXPIRED: 此通知或窗口代次已失效".into());
        }
        Ok(())
    }
    pub fn act(
        &mut self,
        store: &Store,
        target: &Target,
        id: &str,
        action: Action,
        clock: Clock,
    ) -> AppResult<()> {
        self.validate_target(store, target, id, clock)?;
        if target.surface == Surface::Rest {
            if !matches!(action, Action::Dismiss) {
                return Err("INVALID_ARGUMENT: 休息页只允许关闭".into());
            }
            self.clear_rest();
            return Ok(());
        }
        let notice = self.notice.clone().ok_or("SESSION_EXPIRED: 通知已关闭")?;
        let rest_seconds = match action {
            Action::Answer if notice.kind == NoticeKind::Call => {
                if let Some(notice) = self.notice.as_mut() {
                    notice.phase = "playing".into();
                }
                return Ok(());
            }
            Action::Countdown if notice.kind == NoticeKind::AdvanceCountdown => {
                let mut notice = notice;
                notice.id = Uuid::new_v4().to_string();
                notice.kind = NoticeKind::Countdown;
                notice.duration = notice
                    .target_at
                    .ok_or("倒计时缺少目标")?
                    .saturating_sub(clock.wall_ms) as i64;
                if notice.duration <= 0 {
                    let saved = self.mark(store, &notice.source, "expired", None, clock.wall_ms);
                    self.clear_notice();
                    saved?;
                    return Err("DATA_UNAVAILABLE: 倒计时已结束".into());
                }
                return self.publish(store, notice, clock);
            }
            Action::Confirm if notice.kind == NoticeKind::Reminder => notice.break_seconds,
            Action::Dismiss => None,
            Action::Hangup if notice.kind == NoticeKind::Call => None,
            _ => return Err("INVALID_ARGUMENT: 此通知无对应动作".into()),
        };
        // 先保存确认结果，失败时不打开全屏或丢失当前通知。
        self.mark(store, &notice.source, "dismissed", None, clock.wall_ms)?;
        if let Some(seconds) = rest_seconds {
            self.rest_generation = self.rest_generation.wrapping_add(1);
            self.rest = Some(Rest {
                id: Uuid::new_v4().to_string(),
                source: notice.source.clone(),
                source_revision: notice.source_revision,
                started_at: clock.wall_ms,
                deadline: clock.wall_ms.saturating_add(u64::from(seconds) * 1000),
            });
            if let Some((_, usage)) = self.usages.get_mut(&notice.source) {
                usage.clear();
            }
        }
        self.clear_notice();
        Ok(())
    }
    pub fn hover(
        &mut self,
        store: &Store,
        target: &Target,
        id: &str,
        entered: bool,
        clock: Clock,
    ) -> AppResult<()> {
        self.validate_target(store, target, id, clock)?;
        if target.surface != Surface::Notice || !self.island.hover(id, clock.monotonic_ms, entered)
        {
            return Err("SESSION_EXPIRED: 通知已失效".into());
        }
        Ok(())
    }
    pub fn shown(&mut self, store: &Store, target: &Target, clock: Clock) -> AppResult<()> {
        self.validate_target(store, target, &target.event, clock)?;
        let source = if target.surface == Surface::Notice {
            self.notice.as_ref().unwrap().source.clone()
        } else {
            target.owner.clone()
        };
        self.mark(store, &source, "shown", None, clock.wall_ms)
    }
    pub fn failed(
        &mut self,
        store: &Store,
        target: &Target,
        error: String,
        clock: Clock,
    ) -> AppResult<()> {
        self.sync(store, clock)?;
        if self.target(target.surface).as_ref() != Some(target) {
            return Ok(());
        }
        let source = if target.surface == Surface::Notice {
            self.notice.as_ref().unwrap().source.clone()
        } else {
            target.owner.clone()
        };
        self.mark(store, &source, "display-failed", Some(error), clock.wall_ms)?;
        if target.surface == Surface::Notice {
            self.clear_notice();
        } else {
            self.clear_rest();
        }
        Ok(())
    }
    pub fn audio(
        &mut self,
        store: &Store,
        target: &Target,
        id: &str,
        state: &str,
        clock: Clock,
    ) -> AppResult<()> {
        self.validate_target(store, target, id, clock)?;
        let notice = self
            .notice
            .as_ref()
            .filter(|n| n.kind == NoticeKind::Call)
            .ok_or("INVALID_ARGUMENT: 当前不是语音来电")?;
        let source = notice.source.clone();
        match state {
            "playing" => {
                self.mark(store, &source, "audio-playing", None, clock.wall_ms)?;
            }
            "blocked" | "failed" => {
                self.mark(
                    store,
                    &source,
                    "audio-failed",
                    Some("音频播放失败或被系统阻止，请手动重试".into()),
                    clock.wall_ms,
                )?;
            }
            "ended" => {
                self.mark(store, &source, "audio-ended", None, clock.wall_ms)?;
                self.clear_notice();
                return Ok(());
            }
            _ => return Err("INVALID_ARGUMENT: 音频状态无效".into()),
        }
        if let Some(notice) = self.notice.as_mut() {
            notice.phase = state.into();
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        reminder_store::{self, Source},
        store::{now_ms, Store},
        widgets,
    };
    use serde_json::{json, Value};
    #[test]
    fn reminders_one_corrupt_config_cannot_cancel_a_healthy_current_notification() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let bad = add(&s, Source::Sit);
        enable(&s, &sessions, &island, Source::Island);
        let mut engine = Engine::default();
        engine.demo(&s, &island, Demo::Call, clock(1)).unwrap();
        let old = engine.notification().unwrap().id.clone();
        s.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,'broken')",
                [format!("reminder.source/{bad}")],
            )
            .unwrap();
        assert!(
            engine
                .tick(&s, clock(1000), false, &mut || panic!())
                .is_ok(),
            "一个坏实例不能中断整批"
        );
        assert_eq!(engine.notification().unwrap().id, old);
        assert!(engine.status(&bad)["lastError"]
            .as_str()
            .unwrap()
            .contains("配置"));
    }
    #[test]
    fn reminders_one_corrupt_phone_ledger_cannot_prevent_other_phone_delivery() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let a = add(&s, Source::Phone);
        let b = add(&s, Source::Phone);
        let mut ids = [a, b];
        ids.sort();
        let bad = &ids[0];
        let good = &ids[1];
        enable(&s, &sessions, &island, Source::Island);
        for id in &ids {
            let mut config = Source::Phone.default_config();
            config["enabled"] = json!(true);
            config["rules"][1]["enabled"] = json!(true);
            save(&s, &sessions, id, Source::Phone, config);
        }
        s.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,'broken')",
                [format!("reminder.phone-ledger/{bad}")],
            )
            .unwrap();
        let mut engine = Engine::default();
        assert!(engine.tick(&s, clock(0), false, &mut || panic!()).is_ok());
        assert_eq!(&engine.notification().unwrap().source, good);
        assert!(engine.status(bad)["lastError"]
            .as_str()
            .unwrap()
            .contains("日槽"));
    }
    #[test]
    fn reminders_failed_publish_records_error_and_rolls_back_old_replacement_state() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let phone = add(&s, Source::Phone);
        enable(&s, &sessions, &island, Source::Island);
        let mut config = Source::Phone.default_config();
        config["enabled"] = json!(true);
        config["rules"][1]["enabled"] = json!(true);
        save(&s, &sessions, &phone, Source::Phone, config);
        let mut engine = Engine::default();
        engine.demo(&s, &island, Demo::Call, clock(1)).unwrap();
        let old = engine.notification().unwrap().id.clone();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_new_delivery BEFORE INSERT ON app_settings WHEN new.key LIKE 'reminder.delivery/%' AND json_extract(new.value,'$.state')='queued' BEGIN SELECT RAISE(FAIL,'queued denied'); END;").unwrap();
        engine.tick(&s, clock(0), false, &mut || panic!()).unwrap();
        assert!(
            engine.status(&phone)["lastError"].as_str().is_some(),
            "计划槽消费后保存失败仍须如实留错"
        );
        assert_eq!(engine.status(&phone)["delivery"], "delivery-failed");
        assert_eq!(engine.notification().unwrap().id, old);
        let old_state: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT json_extract(value,'$.state') FROM app_settings WHERE key=?1",
                    [format!("reminder.delivery/{island}")],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(
            old_state, "queued",
            "新事件排队失败不能先把旧事件标为已替换"
        );
        let mut restarted = Engine::default();
        restarted
            .tick(&s, clock(0), false, &mut || panic!())
            .unwrap();
        assert!(
            restarted.notification().is_none(),
            "失败来电也不能因重启再次消费同日槽"
        );
    }
    #[test]
    fn reminders_confirm_write_failure_cannot_open_fullscreen_or_discard_current_notice() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let sit = add(&s, Source::Sit);
        enable(&s, &sessions, &island, Source::Island);
        let mut cfg = Source::Sit.default_config();
        cfg["enabled"] = json!(true);
        cfg["sitInterval"] = json!(1);
        save(&s, &sessions, &sit, Source::Sit, cfg);
        let mut engine = Engine::default();
        for n in 0..=6 {
            engine
                .tick(&s, clock(n * 10000), false, &mut || Ok((n as i32, 0)))
                .unwrap();
        }
        let notice = engine.notification().unwrap().id.clone();
        let target = engine.target(Surface::Notice).unwrap();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_confirm BEFORE UPDATE ON app_settings WHEN new.key LIKE 'reminder.delivery/%' BEGIN SELECT RAISE(FAIL,'confirm denied'); END;").unwrap();
        assert!(engine
            .act(&s, &target, &notice, Action::Confirm, clock(60001))
            .is_err());
        assert!(engine.rest().is_none(), "递送状态未提交不得创建全屏休息");
        assert_eq!(engine.notification().unwrap().id, notice);
    }
    #[test]
    fn reminders_expired_countdown_conversion_never_becomes_a_forever_notice() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        enable(&s, &sessions, &island, Source::Island);
        let mut engine = Engine::default();
        engine.tick(&s, clock(0), false, &mut || panic!()).unwrap();
        engine
            .demo(&s, &island, Demo::AdvanceCountdown, clock(0))
            .unwrap();
        let notice = engine.notification().unwrap().id.clone();
        let target = engine.target(Surface::Notice).unwrap();
        let mut jumped = clock(1000);
        jumped.wall_ms += 100000;
        assert!(engine
            .act(&s, &target, &notice, Action::Countdown, jumped)
            .is_err());
        assert!(engine.notification().is_none());
    }
    fn fixture() -> (Store, widgets::Sessions) {
        (
            Store::open(
                std::env::temp_dir()
                    .join(format!("souldesk-reminder-engine-{}", uuid::Uuid::new_v4())),
            )
            .unwrap(),
            widgets::Sessions::default(),
        )
    }
    fn add(store: &Store, source: Source) -> String {
        let manifest = json!({"schemaVersion":1,"id":source.package(),"name":"提醒引擎检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":350,"height":180},"minSize":{"width":280,"height":140},"capabilities":[source.capability()],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT OR IGNORE INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'reminder-test','reminder-test')",rusqlite::params![source.package(),manifest.to_string()]).unwrap();
        widgets::create_instance(store, source.package()).unwrap()
    }
    fn save(store: &Store, sessions: &widgets::Sessions, id: &str, source: Source, config: Value) {
        let label = format!("float-{id}");
        let session = widgets::open_display_session(store, sessions, &label, id, None).unwrap();
        let token = session["token"].as_str().unwrap();
        let old =
            reminder_store::authorized(store, sessions, &label, token, source.get(), Value::Null)
                .unwrap();
        reminder_store::authorized(store,sessions,&label,token,match source{Source::Island=>"island.configure",Source::Sit=>"sit.configure",Source::Phone=>"phone.configure"},json!({"requestId":uuid::Uuid::new_v4().to_string(),"expectedRevision":old["revision"],"config":config})).unwrap();
    }
    fn enable(store: &Store, sessions: &widgets::Sessions, id: &str, source: Source) {
        let mut config = source.default_config();
        config["enabled"] = json!(true);
        save(store, sessions, id, source, config);
    }
    fn clock(at: u64) -> Clock {
        Clock {
            wall_ms: 1_800_000_000_000 + at,
            monotonic_ms: at,
            local: LocalMinute {
                day: 100,
                weekday: 1,
                minute: 900,
                second: ((at / 1000) % 60) as u8,
            },
        }
    }
    #[test]
    fn reminders_engine_never_reads_cursor_until_sit_explicitly_enabled() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let sit = add(&s, Source::Sit);
        let mut engine = Engine::default();
        engine
            .tick(&s, clock(0), false, &mut || panic!("默认关闭不得采集鼠标"))
            .unwrap();
        enable(&s, &sessions, &island, Source::Island);
        engine
            .tick(&s, clock(1000), false, &mut || {
                panic!("仅启用通知容器不得采集鼠标")
            })
            .unwrap();
        enable(&s, &sessions, &sit, Source::Sit);
        let mut count = 0;
        engine
            .tick(&s, clock(2000), false, &mut || {
                count += 1;
                Ok((10, 20))
            })
            .unwrap();
        engine
            .tick(&s, clock(3000), false, &mut || {
                panic!("不到十秒不得重复采样")
            })
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn reminders_sit_confirmation_opens_only_current_break_and_hide_all_cancels_surfaces() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let sit = add(&s, Source::Sit);
        enable(&s, &sessions, &island, Source::Island);
        let mut cfg = Source::Sit.default_config();
        cfg["enabled"] = json!(true);
        cfg["sitInterval"] = json!(1);
        save(&s, &sessions, &sit, Source::Sit, cfg);
        let mut engine = Engine::default();
        for n in 0..=6 {
            engine
                .tick(&s, clock(n * 10000), false, &mut || Ok((n as i32, 0)))
                .unwrap();
        }
        let notice = engine.notification().unwrap().clone();
        assert_eq!(notice.kind, NoticeKind::Reminder);
        assert!(engine.rest().is_none());
        let target = engine.target(Surface::Notice).unwrap();
        engine
            .act(&s, &target, &notice.id, Action::Confirm, clock(60001))
            .unwrap();
        let rest = engine.rest().unwrap().clone();
        assert_eq!(rest.source, sit);
        assert_eq!(rest.deadline, clock(60001).wall_ms + 300000);
        assert!(engine
            .act(&s, &target, &notice.id, Action::Confirm, clock(60002))
            .is_err());
        engine
            .tick(&s, clock(61000), true, &mut || panic!("未到采样时刻"))
            .unwrap();
        assert!(engine.target(Surface::Rest).is_none());
        assert!(engine.target(Surface::Notice).is_none());
    }
    #[test]
    fn reminders_phone_reserves_exact_slot_in_sqlite_before_display_and_never_replays_after_restart(
    ) {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let phone = add(&s, Source::Phone);
        enable(&s, &sessions, &island, Source::Island);
        let mut cfg = Source::Phone.default_config();
        cfg["enabled"] = json!(true);
        cfg["rules"][1]["enabled"] = json!(true);
        save(&s, &sessions, &phone, Source::Phone, cfg);
        let mut engine = Engine::default();
        engine
            .tick(&s, clock(0), false, &mut || panic!("来电不读鼠标"))
            .unwrap();
        let message = engine.notification().unwrap();
        assert_eq!(message.preset, Some(PhonePreset::Tea));
        assert_eq!(message.kind, NoticeKind::Call);
        let mut restarted = Engine::default();
        restarted
            .tick(&s, clock(0), false, &mut || panic!())
            .unwrap();
        assert!(restarted.notification().is_none());
    }
    #[test]
    fn reminders_failed_phone_reservation_cannot_reach_display() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let phone = add(&s, Source::Phone);
        enable(&s, &sessions, &island, Source::Island);
        let mut cfg = Source::Phone.default_config();
        cfg["enabled"] = json!(true);
        cfg["rules"][1]["enabled"] = json!(true);
        save(&s, &sessions, &phone, Source::Phone, cfg);
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_slot BEFORE INSERT ON app_settings WHEN new.key LIKE 'reminder.phone-ledger/%' BEGIN SELECT RAISE(FAIL,'slot denied'); END;").unwrap();
        let mut engine = Engine::default();
        assert!(engine.tick(&s, clock(0), false, &mut || panic!()).is_err());
        assert!(engine.notification().is_none());
    }
    #[test]
    fn reminders_stale_surface_and_action_cannot_operate_replacement_notice() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        enable(&s, &sessions, &island, Source::Island);
        let mut engine = Engine::default();
        engine.tick(&s, clock(0), false, &mut || panic!()).unwrap();
        engine.demo(&s, &island, Demo::Info, clock(0)).unwrap();
        let old = engine.notification().unwrap().id.clone();
        let target = engine.target(Surface::Notice).unwrap();
        engine.demo(&s, &island, Demo::Reminder, clock(1)).unwrap();
        let new = engine.notification().unwrap().id.clone();
        assert!(engine
            .act(&s, &target, &old, Action::Dismiss, clock(2))
            .is_err());
        let mut forged = target;
        forged.generation += 1;
        assert!(engine
            .act(&s, &forged, &new, Action::Dismiss, clock(2))
            .is_err());
        assert_eq!(engine.notification().unwrap().id, new);
    }
    #[test]
    fn reminders_provider_is_shared_retained_and_deleted_owner_does_not_resume_an_old_call() {
        let (s, sessions) = fixture();
        let a = add(&s, Source::Island);
        let b = add(&s, Source::Island);
        enable(&s, &sessions, &a, Source::Island);
        let mut engine = Engine::default();
        engine.tick(&s, clock(0), false, &mut || panic!()).unwrap();
        enable(&s, &sessions, &b, Source::Island);
        engine
            .tick(&s, clock(1000), false, &mut || panic!())
            .unwrap();
        engine.demo(&s, &a, Demo::Call, clock(1001)).unwrap();
        assert_eq!(engine.target(Surface::Notice).unwrap().owner, a);
        widgets::remove_instance(&s, &sessions, &a).unwrap();
        engine
            .tick(&s, clock(2000), false, &mut || panic!())
            .unwrap();
        assert!(engine.notification().is_none());
        engine.demo(&s, &b, Demo::Info, clock(2001)).unwrap();
        assert_eq!(engine.target(Surface::Notice).unwrap().owner, b);
    }
    #[test]
    fn reminders_cursor_failure_clears_usage_and_does_not_publish_false_reminder() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let sit = add(&s, Source::Sit);
        enable(&s, &sessions, &island, Source::Island);
        let mut cfg = Source::Sit.default_config();
        cfg["enabled"] = json!(true);
        cfg["sitInterval"] = json!(1);
        save(&s, &sessions, &sit, Source::Sit, cfg);
        let mut engine = Engine::default();
        engine
            .tick(&s, clock(0), false, &mut || Ok((0, 0)))
            .unwrap();
        engine
            .tick(&s, clock(10000), false, &mut || Ok((1, 0)))
            .unwrap();
        engine
            .tick(&s, clock(20000), false, &mut || Err("读取失败".into()))
            .unwrap();
        assert_eq!(engine.status(&sit)["usedMilliseconds"], 0);
        assert!(engine.status(&sit)["lastError"]
            .as_str()
            .unwrap()
            .contains("读取失败"));
        assert!(engine.notification().is_none());
    }
    #[test]
    fn reminders_explicit_phone_preview_needs_provider_but_does_not_enable_background_rules() {
        let (s, sessions) = fixture();
        let island = add(&s, Source::Island);
        let phone = add(&s, Source::Phone);
        let mut engine = Engine::default();
        assert!(engine
            .preview(&s, &phone, PhonePreset::Pill, clock(0))
            .is_err());
        enable(&s, &sessions, &island, Source::Island);
        engine.tick(&s, clock(0), false, &mut || panic!()).unwrap();
        engine
            .preview(&s, &phone, PhonePreset::Pill, clock(0))
            .unwrap();
        assert_eq!(
            engine.notification().unwrap().preset,
            Some(PhonePreset::Pill)
        );
        let doc = reminder_store::load(&s.db.lock().unwrap(), &phone, Source::Phone).unwrap();
        assert_eq!(doc.config["enabled"], false);
        engine
            .tick(&s, clock(1000), false, &mut || panic!())
            .unwrap();
        assert!(engine.notification().is_some());
        let _ = now_ms();
    }
}
