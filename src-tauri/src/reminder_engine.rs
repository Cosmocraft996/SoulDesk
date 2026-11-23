//! 灵动通知、久坐与来电的核心协调；原生输入通过显式回调注入。
use crate::{
    alerts::Direction,
    notification_sdk::{self, Request},
    reminder_state::{
        IslandState, LocalMinute, PhoneLedger, PhonePreset, PhoneRule, SitConfig, SitUsage,
    },
    reminder_store::{self, Channel, InstanceConfig, IslandPreferences, Source},
    store::{err, AppResult, Store},
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

pub(crate) const HOST_NOTICE_OWNER: &str = "host";
const KIND_LIMIT: usize = 3;
pub(crate) const ISLAND_GAP: u32 = 4;
pub(crate) const ISLAND_MORE_HEIGHT: u32 = 36;

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
    MarketAlert,
    Reminder,
    AdvanceCountdown,
    Countdown,
    Call,
}
impl NoticeKind {
    fn priority(self) -> u8 {
        match self {
            Self::Call => 0,
            Self::Error => 1,
            Self::Warning => 2,
            Self::MarketAlert => 3,
            Self::Reminder => 4,
            Self::Countdown => 5,
            Self::AdvanceCountdown => 6,
            Self::Success => 7,
            Self::Info => 8,
        }
    }

    pub(crate) fn height(self) -> u32 {
        match self {
            Self::Reminder => 144,
            Self::Call | Self::AdvanceCountdown => 72,
            _ => 48,
        }
    }
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
    OpenSource,
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
    pub fn rest_label(&self) -> String {
        debug_assert_eq!(self.surface, Surface::Rest);
        format!("break-{}-{}", self.owner, self.generation)
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarketNotice {
    pub key: crate::alerts::MarketKey,
    pub direction: crate::alerts::Direction,
    pub price: String,
    pub threshold: String,
    triggered_at: u64,
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
    pub source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_alert: Option<MarketNotice>,
    #[serde(skip)]
    pub source_revision: u64,
    #[serde(skip)]
    preview: bool,
    #[serde(skip)]
    pub(crate) break_seconds: Option<u32>,
}

struct Receipt {
    source: String,
    request: Request,
    at: u64,
    result: AppResult<Value>,
}

struct ActiveNotice {
    notice: Notice,
    timer: IslandState,
    shown: bool,
}

fn sdk_island_allowed(store: &Store, notice: &Notice) -> AppResult<bool> {
    let db = store.db.lock().map_err(err)?;
    if notification_sdk::source(&db, &notice.source, notice.kind)?.is_none() {
        return Ok(false);
    }
    Ok(Channel::Island.enabled(&db)?
        && (notice.kind != NoticeKind::Call
            || reminder_store::preference(&db, reminder_store::WIDGET_CALLS)?.as_deref()
                == Some("true")))
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
    notices: Vec<ActiveNotice>,
    queued: VecDeque<Notice>,
    receipts: VecDeque<Receipt>,
    rest: Option<Rest>,
    usages: HashMap<String, (u64, SitUsage)>,
    last_cursor: Option<u64>,
    notice_generation: u64,
    rest_generation: u64,
    hidden: bool,
    preferences: IslandPreferences,
    errors: HashMap<String, String>,
    configuration_errors: HashMap<String, String>,
    delivery: HashMap<String, String>,
}
impl Engine {
    #[cfg(test)]
    pub fn notification(&self) -> Option<&Notice> {
        self.notices.first().map(|entry| &entry.notice)
    }
    pub(crate) fn notice(&self, id: &str) -> Option<&Notice> {
        self.notices
            .iter()
            .map(|entry| &entry.notice)
            .find(|notice| notice.id == id)
    }
    pub(crate) fn visible(&self) -> impl Iterator<Item = &Notice> + Clone {
        self.notices.iter().map(|entry| &entry.notice)
    }
    pub(crate) fn pending(&self) -> Vec<&Notice> {
        let mut queued = self.queued.iter().collect::<Vec<_>>();
        queued.sort_by_key(|notice| notice.kind.priority());
        queued
    }
    pub(crate) fn audible_call(&self) -> Option<&str> {
        self.visible()
            .find(|notice| notice.kind == NoticeKind::Call && !notice.mute)
            .map(|notice| notice.id.as_str())
    }
    pub(crate) fn island_preferences(&self) -> IslandPreferences {
        self.preferences
    }
    pub(crate) fn audible_market(&self) -> Option<&Notice> {
        if !self.preferences.market_sound || self.audible_call().is_some() {
            return None;
        }
        self.visible()
            .filter(|notice| notice.market_alert.is_some() && !notice.mute)
            .max_by_key(|notice| notice.market_alert.as_ref().map(|alert| alert.triggered_at))
    }
    pub(crate) fn market_tone(&self, direction: Direction) -> &'static str {
        match direction {
            Direction::Up => self.preferences.market_up_tone,
            Direction::Down => self.preferences.market_down_tone,
        }
    }
    pub(crate) fn set_market_tone(&mut self, direction: Direction, id: &'static str) {
        let tone = match direction {
            Direction::Up => &mut self.preferences.market_up_tone,
            Direction::Down => &mut self.preferences.market_down_tone,
        };
        if *tone != id {
            *tone = id;
            self.stack_changed();
        }
    }
    pub(crate) fn stack_target(&self) -> Option<Target> {
        if self.hidden || self.notices.is_empty() {
            return None;
        }
        let height = self
            .visible()
            .map(|notice| notice.kind.height())
            .sum::<u32>()
            + ISLAND_GAP * (self.notices.len() as u32 - 1)
            + if self.queued.is_empty() {
                0
            } else {
                ISLAND_GAP + ISLAND_MORE_HEIGHT
            };
        Some(Target {
            surface: Surface::Notice,
            owner: HOST_NOTICE_OWNER.into(),
            event: format!("stack-{}", self.notice_generation),
            generation: self.notice_generation,
            height,
        })
    }
    pub(crate) fn notice_target(&self, id: &str) -> Option<Target> {
        let notice = self.notice(id).filter(|_| !self.hidden)?;
        Some(Target {
            surface: Surface::Notice,
            owner: HOST_NOTICE_OWNER.into(),
            event: notice.id.clone(),
            generation: 0,
            height: notice.kind.height(),
        })
    }
    pub fn rest(&self) -> Option<&Rest> {
        self.rest.as_ref()
    }
    pub fn target(&self, surface: Surface) -> Option<Target> {
        if self.hidden {
            return None;
        }
        match surface {
            Surface::Notice => self.notice_target(&self.notices.first()?.notice.id),
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
        json!({"usedMilliseconds":self.usages.get(id).map(|(_,usage)|usage.used_ms()).unwrap_or(0),"providerAvailable":true,"lastError":self.configuration_errors.get(id).or_else(||self.errors.get(id)),"delivery":self.delivery.get(id),"breakActive":self.rest.as_ref().is_some_and(|rest|rest.source==id)})
    }
    pub fn clear(&mut self) {
        self.cancel_notifications();
        self.clear_rest();
        self.usages.clear();
        self.last_cursor = None;
    }
    pub(crate) fn cancel_notifications(&mut self) {
        if !self.notices.is_empty() || !self.queued.is_empty() {
            self.notices.clear();
            self.queued.clear();
            self.stack_changed();
        }
    }
    pub fn cancel(&mut self, surface: Surface) {
        if surface == Surface::Notice {
            self.cancel_notifications();
        } else {
            self.clear_rest();
        }
    }
    fn stack_changed(&mut self) {
        self.notice_generation = self.notice_generation.wrapping_add(1);
    }
    fn remove_notice(&mut self, id: &str) {
        self.notices.retain(|entry| entry.notice.id != id);
        self.stack_changed();
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
        let count = self.notices.len() + self.queued.len();
        self.queued.retain(|notice| notice.source != id);
        self.notices.retain(|entry| entry.notice.source != id);
        if count != self.notices.len() + self.queued.len() {
            self.stack_changed();
        }
        if self.rest.as_ref().is_some_and(|rest| rest.source == id) {
            self.clear_rest();
        }
    }
    pub(crate) fn sync(&mut self, store: &Store, clock: Clock) -> AppResult<()> {
        let (scan, island_enabled, preferences) = {
            let db = store.db.lock().map_err(err)?;
            (
                reminder_store::configurations(&db)?,
                Channel::Island.enabled(&db)?,
                IslandPreferences::read(&db)?,
            )
        };
        if !island_enabled {
            self.cancel_notifications();
        }
        if preferences != self.preferences {
            if preferences.hidden && !self.preferences.hidden {
                for entry in &mut self.notices {
                    entry
                        .timer
                        .hover(&entry.notice.id, clock.monotonic_ms, false);
                }
            }
            self.preferences = preferences;
            if self.notices.len() > preferences.max_visible {
                let overflow = self.notices.split_off(preferences.max_visible);
                for entry in overflow.into_iter().rev() {
                    self.queued.push_front(entry.notice);
                }
            }
            self.stack_changed();
        }
        self.configs = scan.valid;
        self.configuration_errors = scan.failures;
        let valid = |id: &str, revision: u64, preview: bool| {
            self.configs.iter().any(|c| {
                c.id == id && c.revision == revision && (preview || c.config["enabled"] == true)
            })
        };
        let mut invalid = Vec::new();
        for entry in &self.notices {
            let n = &entry.notice;
            let expired = n.target_at.is_some_and(|target| target <= clock.wall_ms);
            let unavailable = if n.source_name.is_some() {
                expired || !sdk_island_allowed(store, n)?
            } else {
                expired
                    || (n.source != HOST_NOTICE_OWNER
                        && !valid(&n.source, n.source_revision, n.preview))
            };
            if unavailable || entry.timer.expired(clock.monotonic_ms) {
                invalid.push(n.id.clone());
            }
        }
        let invalid_rest = self.rest.as_ref().is_some_and(|r| {
            !valid(&r.source, r.source_revision, false)
                || clock.wall_ms < r.started_at
                || clock.wall_ms >= r.deadline
        });
        for id in invalid {
            self.remove_notice(&id);
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
        Self::save_delivery(
            &*store.db.lock().map_err(err)?,
            id,
            state,
            error.as_deref(),
            now,
        )?;
        self.remember(id, state, error);
        Ok(())
    }
    fn save_delivery(
        db: &rusqlite::Connection,
        id: &str,
        state: &str,
        error: Option<&str>,
        now: u64,
    ) -> AppResult<()> {
        let value = json!({"state":state,"error":error,"at":now});
        db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("reminder.delivery/{id}"),value.to_string()]).map_err(err)?;
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
        let result = self.publish_saved(store, notice, clock, None);
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
    fn has_space(&self, kind: NoticeKind, replacing: Option<&str>) -> bool {
        let visible = self
            .visible()
            .filter(|notice| Some(notice.id.as_str()) != replacing);
        visible.clone().count() < self.preferences.max_visible
            && (kind == NoticeKind::MarketAlert
                || visible.filter(|notice| notice.kind == kind).count() < KIND_LIMIT)
    }

    fn publish_saved(
        &mut self,
        store: &Store,
        notice: Notice,
        clock: Clock,
        replacing: Option<&str>,
    ) -> AppResult<()> {
        let enabled = Channel::Island.enabled_in(store)?;
        if self.hidden || !enabled {
            let message = if self.hidden {
                "隐藏全部期间未显示专用提醒"
            } else {
                "灵动通知渠道已关闭"
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
        let has_space = self.has_space(notice.kind, replacing);
        if !has_space
            && notice.kind != NoticeKind::MarketAlert
            && self
                .queued
                .iter()
                .filter(|notice| notice.kind != NoticeKind::MarketAlert)
                .count()
                >= notification_sdk::QUEUE_LIMIT
        {
            return Err("LIMIT: 灵动通知队列已满".into());
        }
        self.mark(store, &notice.source, "queued", None, clock.wall_ms)?;
        if let Some(id) = replacing {
            self.remove_notice(id);
        }
        if has_space {
            self.activate(notice, clock);
        } else {
            self.queued.push_back(notice);
            self.stack_changed();
        }
        Ok(())
    }

    fn activate(&mut self, notice: Notice, clock: Clock) {
        self.notices.push(Self::active(notice, clock));
        self.notices
            .sort_by_key(|entry| entry.notice.kind.priority());
        self.stack_changed();
    }

    fn active(mut notice: Notice, clock: Clock) -> ActiveNotice {
        notice.created_at = clock.wall_ms;
        if notice.kind == NoticeKind::Countdown {
            notice.duration = notice
                .target_at
                .unwrap_or(clock.wall_ms)
                .saturating_sub(clock.wall_ms) as i64;
        }
        let mut timer = IslandState::default();
        timer.publish(
            notice.id.clone(),
            notice.source.clone(),
            clock.monotonic_ms,
            notice.duration,
        );
        ActiveNotice {
            notice,
            timer,
            shown: false,
        }
    }

    pub(crate) fn send(
        &mut self,
        store: &Store,
        source: &str,
        request: Request,
        clock: Clock,
        hidden: bool,
        windows: impl FnOnce(&str, &Notice) -> AppResult<()>,
    ) -> AppResult<Value> {
        let routing =
            notification_sdk::available(&*store.db.lock().map_err(err)?, source, &request)?;
        self.receipts
            .retain(|r| clock.monotonic_ms.saturating_sub(r.at) < 300_000);
        if let Some(prior) = self
            .receipts
            .iter()
            .find(|r| r.source == source && r.request.request_id == request.request_id)
        {
            return if prior.request == request {
                prior.result.clone()
            } else {
                Err("INVALID_ARGUMENT: 请求标识已用于另一条通知".into())
            };
        }
        if self
            .receipts
            .iter()
            .filter(|r| r.source == source && clock.monotonic_ms.saturating_sub(r.at) < 30_000)
            .count()
            >= 5
            || self.receipts.len() >= 512
        {
            return Err("LIMIT: 通知请求过于频繁".into());
        }
        request.validate_time(clock.wall_ms)?;
        self.hidden = hidden;
        let result = (|| {
            let Some((channel, name)) = routing.filter(|_| !hidden) else {
                return Ok(json!({"status":"suppressed"}));
            };
            let id = Uuid::new_v4().to_string();
            let mut notice = if let Some(preset) = request.preset {
                let mut notice = Self::phone_notice(source, 0, preset, clock, false, false);
                notice.duration = 45_000;
                notice
            } else {
                Notice {
                    id: id.clone(),
                    source: source.into(),
                    kind: request.kind,
                    title: request.title.clone().unwrap_or_default(),
                    message: request.body.clone().unwrap_or_default(),
                    preset: None,
                    target_at: request.target_at,
                    created_at: clock.wall_ms,
                    duration: if matches!(
                        request.kind,
                        NoticeKind::Reminder | NoticeKind::AdvanceCountdown
                    ) {
                        10_000
                    } else {
                        6_000
                    },
                    mute: true,
                    phase: String::new(),
                    source_name: None,
                    market_alert: None,
                    source_revision: 0,
                    preview: false,
                    break_seconds: None,
                }
            };
            notice.id = id.clone();
            notice.source_name = Some(name.clone());
            match channel {
                Channel::Windows => {
                    let delivery = windows(&name, &notice);
                    let state = if delivery.is_ok() {
                        "windows-accepted"
                    } else {
                        "delivery-failed"
                    };
                    let _ = store.log(
                        "delivery",
                        &format!("组件通知 {id} 来源 {source} 状态 {state}"),
                    );
                    delivery?;
                }
                Channel::Island => {
                    self.sync(store, clock)?;
                    self.show_queued(store, clock)?;
                    self.publish(store, notice, clock)?;
                }
            }
            Ok(json!({"status":"accepted","id":id,"channel":channel}))
        })();
        self.receipts.push_back(Receipt {
            source: source.into(),
            request,
            at: clock.monotonic_ms,
            result: result.clone(),
        });
        result
    }

    pub(crate) fn market_alert(
        &mut self,
        store: &Store,
        event: &crate::alerts::AlertEvent,
        clock: Clock,
        hidden: bool,
    ) -> AppResult<()> {
        self.hidden = hidden;
        if hidden || !Channel::Island.enabled_in(store)? {
            return Err(crate::alert_notice::ISLAND_SUPPRESSED.into());
        }
        self.sync(store, clock)?;
        self.show_queued(store, clock)?;
        if self.notice(&event.id).is_some()
            || self.queued.iter().any(|notice| notice.id == event.id)
        {
            return Ok(());
        }
        let (instrument, _) = crate::alert_notice::content(event);
        let movement = match event.direction {
            crate::alerts::Direction::Up => "上涨至",
            crate::alerts::Direction::Down => "下跌至",
        };
        let mut notice = Notice {
            id: event.id.clone(),
            source: HOST_NOTICE_OWNER.into(),
            kind: NoticeKind::MarketAlert,
            title: instrument,
            message: format!("{movement} {}", event.observed_price),
            preset: None,
            target_at: None,
            created_at: clock.wall_ms,
            duration: -1,
            mute: false,
            phase: String::new(),
            source_name: None,
            market_alert: Some(MarketNotice {
                key: event.key.clone(),
                direction: event.direction,
                price: event.observed_price.clone(),
                threshold: event.threshold.clone(),
                triggered_at: event.created_utc_ms,
            }),
            source_revision: 0,
            preview: false,
            break_seconds: None,
        };
        let same_instrument = |old: &Notice| {
            old.market_alert.as_ref().is_some_and(|alert| {
                alert.key.exchange == event.key.exchange
                    && alert.key.market == event.key.market
                    && alert.key.symbol == event.key.symbol
            })
        };
        if let Some(index) = self
            .notices
            .iter()
            .position(|entry| same_instrument(&entry.notice))
        {
            if self.notices[index]
                .notice
                .market_alert
                .as_ref()
                .is_some_and(|alert| alert.triggered_at > event.created_utc_ms)
            {
                return Ok(());
            }
            self.mark(store, HOST_NOTICE_OWNER, "queued", None, clock.wall_ms)?;
            notice.mute = self.notices[index].notice.mute;
            self.notices[index] = Self::active(notice, clock);
            self.stack_changed();
            return Ok(());
        }
        if let Some(index) = self.queued.iter().position(same_instrument) {
            if self.queued[index]
                .market_alert
                .as_ref()
                .is_some_and(|alert| alert.triggered_at > event.created_utc_ms)
            {
                return Ok(());
            }
            notice.mute = self.queued[index].mute;
            self.queued[index] = notice;
            self.stack_changed();
            return Ok(());
        }
        self.publish_saved(store, notice, clock, None)
    }

    pub(crate) fn mute_market_notice(
        &mut self,
        store: &Store,
        id: &str,
        muted: bool,
        clock: Clock,
    ) -> AppResult<()> {
        self.sync(store, clock)?;
        let notice = self
            .notices
            .iter_mut()
            .map(|entry| &mut entry.notice)
            .find(|notice| !self.hidden && notice.id == id && notice.market_alert.is_some())
            .ok_or("SESSION_EXPIRED: 此行情通知已关闭或替换")?;
        if notice.mute != muted {
            notice.mute = muted;
            self.stack_changed();
        }
        Ok(())
    }

    pub(crate) fn dismiss_opened_market(
        &mut self,
        store: &Store,
        id: &str,
        clock: Clock,
    ) -> AppResult<()> {
        let matches = |notice: &Notice| notice.id == id && notice.market_alert.is_some();
        if self.visible().chain(self.queued.iter()).any(matches) {
            self.mark(store, HOST_NOTICE_OWNER, "dismissed", None, clock.wall_ms)?;
            self.queued.retain(|notice| !matches(notice));
            self.remove_notice(id);
            self.show_queued(store, clock)?;
        }
        Ok(())
    }

    pub(crate) fn show_queued(&mut self, store: &Store, clock: Clock) -> AppResult<()> {
        let mut index = 0;
        while let Some(notice) = self.queued.get(index) {
            let expired = (notice.kind != NoticeKind::MarketAlert
                && (clock.wall_ms < notice.created_at
                    || clock.wall_ms.saturating_sub(notice.created_at)
                        >= notification_sdk::MAX_WAIT_MS))
                || notice
                    .target_at
                    .is_some_and(|target| target <= clock.wall_ms);
            let allowed = if notice.source_name.is_some() {
                sdk_island_allowed(store, notice)?
            } else {
                notice.source == HOST_NOTICE_OWNER
                    || self.configs.iter().any(|config| {
                        config.id == notice.source
                            && config.revision == notice.source_revision
                            && (notice.preview || config.config["enabled"] == true)
                    })
            };
            if expired || !allowed {
                let notice = self.queued.remove(index).expect("已读取排队通知");
                self.stack_changed();
                let state = if expired { "expired" } else { "not-shown" };
                let _ = store.log(
                    "delivery",
                    &format!("组件通知 {} 来源 {} 状态 {state}", notice.id, notice.source),
                );
            } else {
                index += 1;
            }
        }
        while let Some((index, _)) = self
            .queued
            .iter()
            .enumerate()
            .filter(|(_, notice)| self.has_space(notice.kind, None))
            .min_by_key(|(_, notice)| notice.kind.priority())
        {
            let notice = self.queued.remove(index).expect("已读取可显示通知");
            self.activate(notice, clock);
        }
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
            source_name: None,
            market_alert: None,
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
            self.cancel_notifications();
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
                                source_name: None,
                                market_alert: None,
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
        self.show_queued(store, clock)?;
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
        self.publish(store, Self::demo_notice(id, revision, demo, clock), clock)
    }

    pub(crate) fn host_demo(
        &mut self,
        store: &Store,
        demo: Demo,
        channel: Channel,
        clock: Clock,
        hidden: bool,
        windows: impl FnOnce(&Notice) -> AppResult<()>,
    ) -> AppResult<()> {
        self.hidden = hidden;
        if hidden {
            return Err("请先恢复组件显示".into());
        }
        if !channel.enabled_in(store)? {
            return Err("所选通知渠道已关闭".into());
        }
        let notice = Self::demo_notice(HOST_NOTICE_OWNER, 0, demo, clock);
        match channel {
            Channel::Island => {
                self.sync(store, clock)?;
                self.show_queued(store, clock)?;
                self.publish(store, notice, clock)
            }
            Channel::Windows => windows(&notice),
        }
    }

    fn demo_notice(id: &str, revision: u64, demo: Demo, clock: Clock) -> Notice {
        if matches!(demo, Demo::Call) {
            return Self::phone_notice(id, revision, PhonePreset::Squidward, clock, true, true);
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
            source_name: None,
            market_alert: None,
            preview: false,
            break_seconds: None,
        }
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
        let current = if target.surface == Surface::Notice {
            self.notice_target(id)
        } else {
            self.target(Surface::Rest)
        };
        if current.as_ref() != Some(target) || target.event != id {
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
        let notice = self
            .notice(id)
            .cloned()
            .ok_or("SESSION_EXPIRED: 通知已关闭")?;
        let rest_seconds = match action {
            Action::Answer if notice.kind == NoticeKind::Call => {
                if self.audible_call() != Some(id) {
                    return Err("请先结束正在播放的来电".into());
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
                    self.remove_notice(id);
                    saved?;
                    return Err("DATA_UNAVAILABLE: 倒计时已结束".into());
                }
                return self.publish_saved(store, notice, clock, Some(id));
            }
            Action::Confirm
                if notice.kind == NoticeKind::Reminder && notice.source_name.is_none() =>
            {
                notice.break_seconds
            }
            Action::OpenSource if notice.kind == NoticeKind::MarketAlert => return Ok(()),
            Action::OpenSource
                if notice.kind == NoticeKind::Reminder && notice.source_name.is_some() =>
            {
                None
            }
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
        self.remove_notice(id);
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
        if entered && self.preferences.hidden {
            return Ok(());
        }
        if target.surface != Surface::Notice
            || !self
                .notices
                .iter_mut()
                .find(|entry| entry.notice.id == id)
                .is_some_and(|entry| entry.timer.hover(id, clock.monotonic_ms, entered))
        {
            return Err("SESSION_EXPIRED: 通知已失效".into());
        }
        Ok(())
    }
    pub fn shown(&mut self, store: &Store, target: &Target, clock: Clock) -> AppResult<()> {
        let current = if target.surface == Surface::Notice {
            self.notice_target(&target.event)
        } else {
            self.target(Surface::Rest)
        };
        if current.as_ref() != Some(target) {
            return Err("SESSION_EXPIRED: 通知窗口已被替换".into());
        }
        if target.surface == Surface::Rest {
            return self.mark(store, &target.owner, "shown", None, clock.wall_ms);
        }
        self.record_shown(store, Some(&target.event), clock)
    }
    fn record_shown(&mut self, store: &Store, id: Option<&str>, clock: Clock) -> AppResult<()> {
        let pending = self
            .notices
            .iter_mut()
            .filter(|entry| !entry.shown && id.is_none_or(|id| entry.notice.id == id))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(());
        }
        {
            let mut db = store.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            for entry in &pending {
                reminder_store::remember_island_notice(&tx, &entry.notice, clock.wall_ms)?;
                Self::save_delivery(&tx, &entry.notice.source, "shown", None, clock.wall_ms)?;
            }
            tx.commit().map_err(err)?;
        }
        let sources = pending
            .into_iter()
            .map(|entry| {
                entry.shown = true;
                entry.notice.source.clone()
            })
            .collect::<Vec<_>>();
        for source in sources {
            self.remember(&source, "shown", None);
        }
        Ok(())
    }
    pub(crate) fn shown_stack(
        &mut self,
        store: &Store,
        target: &Target,
        clock: Clock,
    ) -> AppResult<()> {
        if self.stack_target().as_ref() != Some(target) {
            return Err("SESSION_EXPIRED: 通知窗口已被替换".into());
        }
        self.record_shown(store, None, clock)
    }
    pub fn failed(
        &mut self,
        store: &Store,
        target: &Target,
        error: String,
        clock: Clock,
    ) -> AppResult<()> {
        self.sync(store, clock)?;
        let current = if target.surface == Surface::Notice {
            self.stack_target()
        } else {
            self.target(Surface::Rest)
        };
        if current.as_ref() != Some(target) {
            return Ok(());
        }
        if target.surface == Surface::Notice {
            let sources = self
                .visible()
                .map(|notice| notice.source.clone())
                .collect::<Vec<_>>();
            for source in sources {
                self.mark(
                    store,
                    &source,
                    "display-failed",
                    Some(error.clone()),
                    clock.wall_ms,
                )?;
            }
            self.notices.clear();
            self.stack_changed();
            self.show_queued(store, clock)?;
        } else {
            self.mark(
                store,
                &target.owner,
                "display-failed",
                Some(error),
                clock.wall_ms,
            )?;
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
            .notice(id)
            .filter(|n| n.kind == NoticeKind::Call)
            .ok_or("INVALID_ARGUMENT: 当前不是语音来电")?;
        if self.audible_call() != Some(id) {
            return Err("SESSION_EXPIRED: 当前来电未获播放权限".into());
        }
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
                self.remove_notice(id);
                return Ok(());
            }
            _ => return Err("INVALID_ARGUMENT: 音频状态无效".into()),
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

    fn market_event(number: u64, market: &str, symbol: &str) -> crate::alerts::AlertEvent {
        use crate::alerts::{
            AlertEvent, Direction, MarketKey, NotificationChannel, PriceKind, Reason,
        };
        let key = if market == "spot" {
            MarketKey::spot(symbol)
        } else {
            MarketKey::binance(symbol, PriceKind::Last)
        };
        AlertEvent {
            id: format!("行情-{number}"),
            rule_id: "测试规则".into(),
            rule_name: "行情测试".into(),
            direction: Direction::Up,
            revision: 1,
            sequence: number,
            key,
            threshold: "100".into(),
            observed_price: "101".into(),
            source_utc_ms: number,
            created_utc_ms: number,
            reason: Reason::Crossed,
            notification: true,
            notification_channel: NotificationChannel::Island,
            sound: false,
        }
    }

    #[test]
    fn market_tone_preferences_restore_independently() {
        use crate::alerts::Direction;

        let (store, _) = fixture();
        let mut engine = Engine::default();
        engine.sync(&store, clock(1)).unwrap();
        assert_eq!(engine.market_tone(Direction::Up), "aicoin-rise.mp3");
        assert_eq!(engine.market_tone(Direction::Down), "aicoin-fall-2.mp3");

        let event = market_event(1, "spot", "BTCUSDT");
        engine
            .market_alert(&store, &event, clock(2), false)
            .unwrap();
        let previous = engine.stack_target();
        let tone = "tradingview/soft/trumpets.mp3";
        store
            .save_preference(reminder_store::market_tone_key(Direction::Up), tone)
            .unwrap();
        engine.set_market_tone(Direction::Up, tone);
        assert!(previous != engine.stack_target());
        assert_eq!(engine.audible_market().unwrap().id, event.id);
        assert_eq!(engine.market_tone(Direction::Up), tone);

        store
            .save_preference(
                reminder_store::market_tone_key(Direction::Down),
                "../bad.mp3",
            )
            .unwrap();
        let mut restarted = Engine::default();
        restarted.sync(&store, clock(3)).unwrap();
        assert_eq!(restarted.market_tone(Direction::Up), tone);
        assert_eq!(restarted.market_tone(Direction::Down), "aicoin-fall-2.mp3");
    }

    #[test]
    fn market_notices_retain_mute_on_replacement_and_resume_next_when_closed() {
        let (store, _) = fixture();
        let mut engine = Engine::default();
        let first = market_event(1, "spot", "BTCUSDT");
        let second = market_event(2, "usd_m", "BTCUSDT");
        let mut replacement = market_event(3, "spot", "BTCUSDT");
        replacement.direction = crate::alerts::Direction::Down;
        replacement.observed_price = "99.5".into();
        for event in [&first, &second] {
            engine.market_alert(&store, event, clock(1), false).unwrap();
        }
        engine
            .mute_market_notice(&store, &first.id, true, clock(2))
            .unwrap();
        for event in [&replacement, &first] {
            engine.market_alert(&store, event, clock(3), false).unwrap();
        }
        assert_eq!(
            engine
                .visible()
                .map(|notice| notice.id.as_str())
                .collect::<Vec<_>>(),
            [&replacement.id, &second.id].map(String::as_str)
        );
        assert_eq!(
            engine.notice(&replacement.id).unwrap().message,
            "下跌至 99.5"
        );
        assert!(engine.notice(&replacement.id).unwrap().mute);
        assert_eq!(engine.audible_market().unwrap().id, second.id);
        engine
            .dismiss_opened_market(&store, &first.id, clock(4))
            .unwrap();
        assert!(engine.notice(&replacement.id).is_some());
        engine
            .mute_market_notice(&store, &replacement.id, false, clock(5))
            .unwrap();
        assert_eq!(engine.audible_market().unwrap().id, replacement.id);
        store
            .save_preference(reminder_store::MARKET_SOUND, "false")
            .unwrap();
        engine.sync(&store, clock(90_000)).unwrap();
        assert!(engine.audible_market().is_none());
        assert_eq!(engine.visible().count(), 2);
        store
            .save_preference(reminder_store::MARKET_SOUND, "true")
            .unwrap();
        engine.sync(&store, clock(90_100)).unwrap();
        let target = engine.notice_target(&replacement.id).unwrap();
        engine
            .act(
                &store,
                &target,
                &replacement.id,
                Action::OpenSource,
                clock(90_100),
            )
            .unwrap();
        assert_eq!(engine.audible_market().unwrap().id, replacement.id);
        engine
            .dismiss_opened_market(&store, &replacement.id, clock(90_101))
            .unwrap();
        assert_eq!(engine.audible_market().unwrap().id, second.id);
    }

    #[test]
    fn market_stack_defaults_to_nine_and_adjusting_limit_preserves_pending() {
        let (store, _) = fixture();
        let mut engine = Engine::default();
        for number in 1..=10 {
            let event = market_event(number, "spot", &format!("TOKEN{number}USDT"));
            engine
                .market_alert(&store, &event, clock(number), false)
                .unwrap();
        }
        assert_eq!(engine.visible().count(), 9);
        assert_eq!(engine.pending().len(), 1);
        engine
            .mute_market_notice(&store, "行情-9", true, clock(11))
            .unwrap();
        store
            .save_preference(reminder_store::ISLAND_MAX_VISIBLE, "2")
            .unwrap();
        engine.sync(&store, clock(60_000)).unwrap();
        engine.show_queued(&store, clock(60_000)).unwrap();
        assert_eq!(engine.visible().count(), 2);
        assert_eq!(engine.pending().len(), 8);
        let replacement = market_event(20, "spot", "TOKEN9USDT");
        engine
            .market_alert(&store, &replacement, clock(60_001), false)
            .unwrap();
        assert_eq!(engine.pending().len(), 8);
        assert!(engine
            .pending()
            .iter()
            .any(|notice| notice.id == replacement.id && notice.mute));
        store
            .save_preference(reminder_store::ISLAND_MAX_VISIBLE, "12")
            .unwrap();
        engine.sync(&store, clock(60_002)).unwrap();
        engine.show_queued(&store, clock(60_002)).unwrap();
        assert_eq!(engine.visible().count(), 10);
        assert!(engine.pending().is_empty());
    }

    #[test]
    fn reminders_history_keeps_recent_distinct_notifications_after_restart() {
        let (store, _) = fixture();
        let mut engine = Engine::default();
        for index in 0..201 {
            engine
                .host_demo(
                    &store,
                    Demo::Info,
                    Channel::Island,
                    clock(index),
                    false,
                    |_| unreachable!(),
                )
                .unwrap();
            if index == 0 {
                assert_eq!(reminder_store::island_history(&store).unwrap(), json!([]));
            }
            let target = engine.target(Surface::Notice).unwrap();
            engine.shown(&store, &target, clock(index)).unwrap();
            engine.shown(&store, &target, clock(index + 1000)).unwrap();
            engine
                .act(
                    &store,
                    &target,
                    &target.event,
                    Action::Dismiss,
                    clock(index),
                )
                .unwrap();
        }
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        let history = reminder_store::island_history(&reopened).unwrap();
        assert_eq!(history.as_array().unwrap().len(), 200);
        assert_eq!(history[0]["at"], clock(200).wall_ms);
        assert_eq!(history[199]["at"], clock(1).wall_ms);
    }
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
        let sit = add(&s, Source::Sit);
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
        let phone = add(&s, Source::Phone);
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
    fn reminders_card_actions_keep_their_identity_across_stack_updates() {
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
        engine
            .act(&s, &target, &old, Action::Dismiss, clock(2))
            .unwrap();
        assert!(engine
            .act(&s, &target, &old, Action::Dismiss, clock(2))
            .is_err());
        assert!(engine
            .act(&s, &target, &new, Action::Dismiss, clock(2))
            .is_err());
        assert_eq!(engine.notification().unwrap().id, new);
    }
    #[test]
    fn reminders_stack_caps_same_kind_and_total_then_promotes_by_priority() {
        let (s, _) = fixture();
        s.save_preference(reminder_store::ISLAND_MAX_VISIBLE, "6")
            .unwrap();
        let mut engine = Engine::default();
        for (index, demo) in [
            Demo::Info,
            Demo::Info,
            Demo::Info,
            Demo::Info,
            Demo::Warning,
            Demo::Success,
            Demo::Success,
            Demo::Error,
            Demo::Call,
        ]
        .into_iter()
        .enumerate()
        {
            engine
                .host_demo(
                    &s,
                    demo,
                    Channel::Island,
                    clock(index as u64),
                    false,
                    |_| unreachable!(),
                )
                .unwrap();
        }
        let kinds = |engine: &Engine| {
            engine
                .visible()
                .map(|notice| notice.kind)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(&engine),
            [
                NoticeKind::Warning,
                NoticeKind::Success,
                NoticeKind::Success,
                NoticeKind::Info,
                NoticeKind::Info,
                NoticeKind::Info
            ]
        );
        assert_eq!(
            engine
                .pending()
                .iter()
                .map(|notice| notice.kind)
                .collect::<Vec<_>>(),
            [NoticeKind::Call, NoticeKind::Error, NoticeKind::Info]
        );
        assert_eq!(engine.stack_target().unwrap().height, 48 * 6 + 4 * 6 + 36);
        let priority = engine.target(Surface::Notice).unwrap();
        engine
            .act(&s, &priority, &priority.event, Action::Dismiss, clock(20))
            .unwrap();
        engine.show_queued(&s, clock(20)).unwrap();
        assert_eq!(engine.notification().unwrap().kind, NoticeKind::Call);
        assert_eq!(engine.pending().len(), 2);
    }
    #[test]
    fn reminders_host_owns_overlay_and_removed_legacy_demo_does_not_resume() {
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
        assert_eq!(
            engine.target(Surface::Notice).unwrap().owner,
            HOST_NOTICE_OWNER
        );
        widgets::remove_instance(&s, &sessions, &a).unwrap();
        engine
            .tick(&s, clock(2000), false, &mut || panic!())
            .unwrap();
        assert!(engine.notification().is_none());
        engine.demo(&s, &b, Demo::Info, clock(2001)).unwrap();
        assert_eq!(
            engine.target(Surface::Notice).unwrap().owner,
            HOST_NOTICE_OWNER
        );
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
    fn reminders_explicit_phone_preview_works_without_legacy_provider_or_background_rules() {
        let (s, _) = fixture();
        let phone = add(&s, Source::Phone);
        let mut engine = Engine::default();
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
