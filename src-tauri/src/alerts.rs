//! G2 到价规则纯状态转换；持久化成功后才允许递送事件。
use crate::store::AppResult;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

const FRESH_MS: u64 = 10_000;
const FUTURE_TOLERANCE_MS: u64 = 5_000;
const CLOCK_SHIFT_TOLERANCE_MS: i128 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price(Decimal);
pub fn parse_price(value: &str) -> AppResult<Price> {
    let parts = value.split('.').collect::<Vec<_>>();
    if value.len() > 64
        || parts.is_empty()
        || parts.len() > 2
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("INVALID_PRICE: 价格需要正的普通十进制字符串".into());
    }
    let value = Decimal::from_str_exact(value)
        .map_err(|_| "INVALID_PRICE: 价格精度或范围无法精确表示".to_owned())?;
    if value <= Decimal::ZERO {
        return Err("INVALID_PRICE: 价格必须大于0".into());
    }
    Ok(Price(value))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PriceKind {
    Last,
    Mark,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketKey {
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub price_kind: PriceKind,
}
impl MarketKey {
    pub fn binance(symbol: &str, price_kind: PriceKind) -> Self {
        Self {
            exchange: "binance".into(),
            market: "usd_m".into(),
            symbol: symbol.into(),
            price_kind,
        }
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.exchange != "binance"
            || self.market != "usd_m"
            || self.symbol.is_empty()
            || self.symbol.len() > 120
            || self.symbol.chars().count() > 40
            || !self
                .symbol
                .chars()
                .all(|c| (c.is_alphanumeric() && !c.is_lowercase()) || c == '_')
        {
            return Err("INVALID_ARGUMENT: 行情身份无效".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Once,
    Repeat,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub key: MarketKey,
    pub direction: Direction,
    pub threshold: String,
    pub mode: Mode,
    pub cooldown_ms: u64,
    pub enabled: bool,
    pub notification: bool,
    pub sound: bool,
}
impl Rule {
    pub fn validate(&self) -> AppResult<()> {
        self.key.validate()?;
        parse_price(&self.threshold)?;
        if uuid::Uuid::parse_str(&self.id).is_err()
            || self.revision == 0
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.cooldown_ms > 7 * 24 * 60 * 60 * 1000
        {
            return Err("INVALID_ARGUMENT: 规则标识、版本、名称或冷却范围无效".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub utc_ms: u64,
    pub mono_ms: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub clock: Clock,
    pub generation: u64,
    /// 由行情服务校正的交易所当前UTC，避免把本地钟跳变当成报价过期或冷却到期。
    pub source_now_utc_ms: u64,
}
#[derive(Clone, Debug)]
pub struct Observation {
    pub key: MarketKey,
    pub price: String,
    pub source_utc_ms: u64,
    pub received_mono_ms: u64,
    pub generation: u64,
    pub ordinal: u64,
    pub recovered: bool,
    pub valid: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleState {
    pub initialized: bool,
    pub waiting_side: bool,
    pub pending: bool,
    pub completed: bool,
    pub sequence: u64,
    pub cooldown_until_utc_ms: Option<u64>,
    #[serde(skip)]
    cooldown_until_mono_ms: Option<u64>,
    #[serde(skip)]
    last_input: Option<(u64, u64)>,
    #[serde(skip)]
    last_source: Option<(u64, Price)>,
    #[serde(skip)]
    last_clock: Option<Clock>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    InitialMet,
    Reached,
    CooldownDue,
    RecoveredMet,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub direction: Direction,
    pub revision: u64,
    pub sequence: u64,
    pub key: MarketKey,
    pub threshold: String,
    pub observed_price: String,
    pub source_utc_ms: u64,
    pub created_utc_ms: u64,
    pub reason: Reason,
    pub notification: bool,
    pub sound: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreReason {
    Disabled,
    Completed,
    WrongMarket,
    WrongGeneration,
    Unavailable,
    Stale,
    Duplicate,
    OutOfOrder,
    MonotonicClockRegression,
}
pub struct Transition {
    pub state: RuleState,
    pub event: Option<AlertEvent>,
    pub ignored: Option<IgnoreReason>,
    pub clock_shift: bool,
}
fn ignored(state: &RuleState, reason: IgnoreReason) -> Transition {
    Transition {
        state: state.clone(),
        event: None,
        ignored: Some(reason),
        clock_shift: false,
    }
}

pub fn evaluate(
    rule: &Rule,
    state: &RuleState,
    quote: &Observation,
    context: Context,
) -> AppResult<Transition> {
    rule.validate()?;
    if !rule.enabled {
        return Ok(ignored(state, IgnoreReason::Disabled));
    }
    if state.completed {
        return Ok(ignored(state, IgnoreReason::Completed));
    }
    if quote.key != rule.key {
        return Ok(ignored(state, IgnoreReason::WrongMarket));
    }
    if quote.generation != context.generation {
        return Ok(ignored(state, IgnoreReason::WrongGeneration));
    }
    if !quote.valid {
        return Ok(ignored(state, IgnoreReason::Unavailable));
    }
    if quote.received_mono_ms > context.clock.mono_ms
        || context.clock.mono_ms - quote.received_mono_ms > FRESH_MS
        || quote.source_utc_ms
            > context
                .source_now_utc_ms
                .saturating_add(FUTURE_TOLERANCE_MS)
        || context
            .source_now_utc_ms
            .saturating_sub(quote.source_utc_ms)
            > FRESH_MS
    {
        return Ok(ignored(state, IgnoreReason::Stale));
    }
    let price = parse_price(&quote.price)?;
    let threshold = parse_price(&rule.threshold)?;
    if state.last_input.is_some_and(|(generation, ordinal)| {
        generation == quote.generation && quote.ordinal <= ordinal
    }) {
        return Ok(ignored(state, IgnoreReason::Duplicate));
    }
    if state
        .last_source
        .is_some_and(|(time, old)| quote.source_utc_ms == time && price == old)
    {
        return Ok(ignored(state, IgnoreReason::Duplicate));
    }
    if state
        .last_source
        .is_some_and(|(time, _)| quote.source_utc_ms < time)
    {
        return Ok(ignored(state, IgnoreReason::OutOfOrder));
    }
    if state
        .last_clock
        .is_some_and(|clock| context.clock.mono_ms < clock.mono_ms)
    {
        return Ok(ignored(state, IgnoreReason::MonotonicClockRegression));
    }
    let clock_shift = state.last_clock.is_some_and(|old| {
        ((context.clock.utc_ms as i128 - old.utc_ms as i128)
            - (context.clock.mono_ms as i128 - old.mono_ms as i128))
            .abs()
            > CLOCK_SHIFT_TOLERANCE_MS
    });
    let mut next = state.clone();
    // 重启后只用UTC截止和新的校正时间还原剩余冷却，旧单调时间与报价缓存不复用。
    if next.cooldown_until_mono_ms.is_none() {
        if let Some(deadline) = next.cooldown_until_utc_ms {
            let remaining = deadline
                .saturating_sub(context.source_now_utc_ms)
                .min(rule.cooldown_ms);
            next.cooldown_until_mono_ms = Some(
                context
                    .clock
                    .mono_ms
                    .checked_add(remaining)
                    .ok_or("CLOCK_INVALID: 冷却截止溢出")?,
            );
        }
    }
    next.last_input = Some((quote.generation, quote.ordinal));
    next.last_source = Some((quote.source_utc_ms, price));
    next.last_clock = Some(context.clock);
    let met = match rule.direction {
        Direction::Up => price >= threshold,
        Direction::Down => price <= threshold,
    };
    let mut reason = None;
    if !next.initialized {
        next.initialized = true;
        next.waiting_side = !met;
        if met {
            reason = Some(Reason::InitialMet);
        }
    } else {
        let cooling = next
            .cooldown_until_mono_ms
            .is_some_and(|deadline| context.clock.mono_ms < deadline);
        let after_deadline = next
            .cooldown_until_mono_ms
            .is_none_or(|deadline| quote.received_mono_ms >= deadline)
            && next
                .cooldown_until_utc_ms
                .is_none_or(|deadline| quote.source_utc_ms >= deadline);
        if !met {
            next.waiting_side = true;
            if !cooling && after_deadline {
                next.pending = false;
            }
        } else if cooling || !after_deadline {
            if next.waiting_side {
                next.pending = true;
                next.waiting_side = false;
            }
        } else if next.pending {
            reason = Some(Reason::CooldownDue);
        } else if next.waiting_side {
            reason = Some(if quote.recovered {
                Reason::RecoveredMet
            } else {
                Reason::Reached
            });
        }
    }
    let event = if let Some(reason) = reason {
        next.sequence = next
            .sequence
            .checked_add(1)
            .ok_or("STATE_INVALID: 触发序号已达上限")?;
        next.pending = false;
        next.waiting_side = false;
        next.completed = rule.mode == Mode::Once;
        next.cooldown_until_utc_ms = Some(
            context
                .source_now_utc_ms
                .checked_add(rule.cooldown_ms)
                .ok_or("CLOCK_INVALID: UTC冷却截止溢出")?,
        );
        next.cooldown_until_mono_ms = Some(
            context
                .clock
                .mono_ms
                .checked_add(rule.cooldown_ms)
                .ok_or("CLOCK_INVALID: 单调冷却截止溢出")?,
        );
        Some(AlertEvent {
            id: format!("{}:{}:{}", rule.id, rule.revision, next.sequence),
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            direction: rule.direction,
            revision: rule.revision,
            sequence: next.sequence,
            key: rule.key.clone(),
            threshold: rule.threshold.clone(),
            observed_price: quote.price.clone(),
            source_utc_ms: quote.source_utc_ms,
            created_utc_ms: context.source_now_utc_ms,
            reason,
            notification: rule.notification,
            sound: rule.sound,
        })
    } else {
        None
    };
    Ok(Transition {
        state: next,
        event,
        ignored: None,
        clock_shift,
    })
}

pub fn revise(old: &Rule, new: &Rule, state: &RuleState) -> AppResult<RuleState> {
    old.validate()?;
    new.validate()?;
    if old.id != new.id || old.revision.checked_add(1) != Some(new.revision) {
        return Err("CONFLICT: 规则版本已变化，请刷新后修改".into());
    }
    let same = old.key == new.key
        && old.direction == new.direction
        && parse_price(&old.threshold)? == parse_price(&new.threshold)?
        && old.mode == new.mode
        && old.cooldown_ms == new.cooldown_ms
        && old.enabled == new.enabled;
    if same {
        Ok(state.clone())
    } else {
        Ok(RuleState {
            sequence: state.sequence,
            ..RuleState::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_symbols_support_chinese_names_but_reject_path_and_control_characters() {
        assert!(MarketKey::binance("币安人生USDT", PriceKind::Last)
            .validate()
            .is_ok());
        for symbol in ["btcusdt", "BTC/USDT", "BTC\nUSDT", "BTC%20USDT", ""] {
            assert!(MarketKey::binance(symbol, PriceKind::Last)
                .validate()
                .is_err());
        }
    }
    const UTC: u64 = 1_700_000_000_000;
    fn rule(direction: Direction) -> Rule {
        Rule {
            id: "00000000-0000-4000-8000-000000000001".into(),
            revision: 1,
            name: "测试规则".into(),
            key: MarketKey::binance("BTCUSDT", PriceKind::Last),
            direction,
            threshold: "100".into(),
            mode: Mode::Repeat,
            cooldown_ms: 10_000,
            enabled: true,
            notification: true,
            sound: false,
        }
    }
    fn input(rule: &Rule, price: &str, time: u64) -> Observation {
        Observation {
            key: rule.key.clone(),
            price: price.into(),
            source_utc_ms: UTC + time,
            received_mono_ms: time,
            generation: 1,
            ordinal: time + 1,
            recovered: false,
            valid: true,
        }
    }
    fn step(rule: &Rule, state: &RuleState, price: &str, time: u64) -> Transition {
        evaluate(
            rule,
            state,
            &input(rule, price, time),
            Context {
                clock: Clock {
                    utc_ms: UTC + time,
                    mono_ms: time,
                },
                generation: 1,
                source_now_utc_ms: UTC + time,
            },
        )
        .unwrap()
    }

    #[test]
    fn equality_and_continuous_met_are_symmetric_without_periodic_repeats() {
        for (direction, waiting, met) in
            [(Direction::Up, "99", "101"), (Direction::Down, "101", "99")]
        {
            let r = rule(direction);
            let first = step(&r, &RuleState::default(), waiting, 0);
            assert!(first.event.is_none());
            let hit = step(&r, &first.state, "100", 1);
            assert_eq!(hit.event.as_ref().unwrap().reason, Reason::Reached);
            assert_eq!(hit.state.sequence, 1);
            let held = step(&r, &hit.state, met, 20_000);
            assert!(held.event.is_none());
            assert_eq!(held.state.sequence, 1);
        }
    }
    #[test]
    fn cooldown_oscillations_merge_into_one_post_deadline_fresh_event() {
        for (direction, waiting, met) in
            [(Direction::Up, "99", "101"), (Direction::Down, "101", "99")]
        {
            let r = rule(direction);
            let mut s = step(&r, &RuleState::default(), "100", 0).state;
            for (price, t) in [(waiting, 1000), (met, 2000), (waiting, 3000), (met, 4000)] {
                let result = step(&r, &s, price, t);
                assert!(result.event.is_none());
                s = result.state;
            }
            assert!(s.pending);
            let due = step(&r, &s, "100", 10_000);
            assert_eq!(due.event.as_ref().unwrap().reason, Reason::CooldownDue);
            assert_eq!(due.state.sequence, 2);
            assert!(!due.state.pending);
            assert!(step(&r, &due.state, met, 20_000).event.is_none());
        }
    }
    #[test]
    fn pending_below_threshold_is_cancelled_then_new_reach_is_required() {
        let r = rule(Direction::Up);
        let mut s = step(&r, &RuleState::default(), "100", 0).state;
        for (p, t) in [("99", 1000), ("100", 2000)] {
            s = step(&r, &s, p, t).state;
        }
        let cancelled = step(&r, &s, "99", 10_000);
        assert!(cancelled.event.is_none());
        assert!(!cancelled.state.pending);
        let next = step(&r, &cancelled.state, "100", 10_001);
        assert_eq!(next.event.unwrap().reason, Reason::Reached);
    }
    #[test]
    fn stale_wrong_generation_identity_and_duplicate_inputs_do_not_change_state() {
        let r = rule(Direction::Up);
        let s = step(&r, &RuleState::default(), "99", 0).state;
        let good = input(&r, "100", 20_000);
        let context = Context {
            clock: Clock {
                utc_ms: UTC + 20_000,
                mono_ms: 20_000,
            },
            generation: 1,
            source_now_utc_ms: UTC + 20_000,
        };
        for bad in [
            Observation {
                generation: 0,
                ..good.clone()
            },
            Observation {
                key: MarketKey::binance("ETHUSDT", PriceKind::Last),
                ..good.clone()
            },
            Observation {
                received_mono_ms: 0,
                ..good.clone()
            },
            Observation {
                source_utc_ms: UTC,
                ..good.clone()
            },
            Observation {
                valid: false,
                ..good.clone()
            },
        ] {
            let result = evaluate(&r, &s, &bad, context).unwrap();
            assert!(result.event.is_none());
            assert_eq!(result.state, s);
            assert!(result.ignored.is_some());
        }
        let hit = evaluate(&r, &s, &good, context).unwrap();
        let duplicate = evaluate(&r, &hit.state, &good, context).unwrap();
        assert!(duplicate.event.is_none());
        assert_eq!(duplicate.state, hit.state);
    }
    #[test]
    fn pre_deadline_observation_cannot_release_pending_after_a_timer_fires() {
        let r = rule(Direction::Up);
        let mut s = step(&r, &RuleState::default(), "100", 0).state;
        s = step(&r, &s, "99", 1000).state;
        s = step(&r, &s, "100", 2000).state;
        let quote = input(&r, "100", 9999);
        let result = evaluate(
            &r,
            &s,
            &quote,
            Context {
                clock: Clock {
                    utc_ms: UTC + 10_000,
                    mono_ms: 10_000,
                },
                generation: 1,
                source_now_utc_ms: UTC + 10_000,
            },
        )
        .unwrap();
        assert!(result.event.is_none());
        assert!(result.state.pending);
        assert_eq!(
            step(&r, &result.state, "100", 10_001).event.unwrap().reason,
            Reason::CooldownDue
        );
    }
    #[test]
    fn recovery_fires_unhandled_conditions_but_does_not_replay_processed_side() {
        let r = rule(Direction::Up);
        let waiting = step(&r, &RuleState::default(), "99", 0).state;
        let quote = Observation {
            recovered: true,
            ..input(&r, "101", 20_000)
        };
        let context = Context {
            clock: Clock {
                utc_ms: UTC + 20_000,
                mono_ms: 20_000,
            },
            generation: 1,
            source_now_utc_ms: UTC + 20_000,
        };
        assert_eq!(
            evaluate(&r, &waiting, &quote, context)
                .unwrap()
                .event
                .unwrap()
                .reason,
            Reason::RecoveredMet
        );
        let handled = step(&r, &RuleState::default(), "100", 0).state;
        assert!(evaluate(&r, &handled, &quote, context)
            .unwrap()
            .event
            .is_none());
    }
    #[test]
    fn pending_recovery_generates_only_cooldown_due_and_once_stays_completed_after_restart() {
        let mut r = rule(Direction::Up);
        let mut s = step(&r, &RuleState::default(), "100", 0).state;
        s = step(&r, &s, "99", 1000).state;
        s = step(&r, &s, "100", 2000).state;
        let restored: RuleState =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let quote = Observation {
            received_mono_ms: 50,
            recovered: true,
            ..input(&r, "101", 20_000)
        };
        let due = evaluate(
            &r,
            &restored,
            &quote,
            Context {
                clock: Clock {
                    utc_ms: UTC + 20_000,
                    mono_ms: 50,
                },
                generation: 1,
                source_now_utc_ms: UTC + 20_000,
            },
        )
        .unwrap();
        assert_eq!(due.event.unwrap().reason, Reason::CooldownDue);
        assert_eq!(due.state.sequence, 2);
        r.mode = Mode::Once;
        let once = step(&r, &RuleState::default(), "100", 0).state;
        let once: RuleState = serde_json::from_str(&serde_json::to_string(&once).unwrap()).unwrap();
        assert!(evaluate(
            &r,
            &once,
            &quote,
            Context {
                clock: Clock {
                    utc_ms: UTC + 20_000,
                    mono_ms: 50
                },
                generation: 1,
                source_now_utc_ms: UTC + 20_000,
            }
        )
        .unwrap()
        .event
        .is_none());
        assert!(once.completed);
    }
    #[test]
    fn decimal_boundary_is_exact_and_invalid_prices_are_rejected() {
        let mut r = rule(Direction::Up);
        r.threshold = "0.1000000000000000000000000001".into();
        let waiting = step(
            &r,
            &RuleState::default(),
            "0.1000000000000000000000000000",
            0,
        );
        assert!(waiting.event.is_none());
        assert!(step(&r, &waiting.state, &r.threshold, 1).event.is_some());
        for invalid in [
            "0",
            "-1",
            "NaN",
            "1e2",
            "1_000",
            "0.00000000000000000000000000001",
            "79228162514264337593543950336",
        ] {
            assert!(parse_price(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn description_edit_keeps_state_but_business_edit_and_reenable_rearm_explicitly() {
        let old = rule(Direction::Up);
        let state = step(&old, &RuleState::default(), "100", 0).state;
        let mut renamed = old.clone();
        renamed.revision = 2;
        renamed.name = "改名".into();
        assert_eq!(revise(&old, &renamed, &state).unwrap(), state);
        let mut changed = renamed.clone();
        changed.revision = 3;
        changed.threshold = "200".into();
        let fresh = revise(&renamed, &changed, &state).unwrap();
        assert!(!fresh.initialized);
        assert_eq!(fresh.sequence, 1);
        assert_eq!(
            step(&changed, &fresh, "200", 1000).event.unwrap().reason,
            Reason::InitialMet
        );
        let mut paused = old.clone();
        paused.revision = 2;
        paused.enabled = false;
        let paused_state = revise(&old, &paused, &state).unwrap();
        assert!(step(&paused, &paused_state, "100", 1000).event.is_none());
        let mut enabled = paused.clone();
        enabled.revision = 3;
        enabled.enabled = true;
        let rearmed = revise(&paused, &enabled, &paused_state).unwrap();
        assert_eq!(
            step(&enabled, &rearmed, "100", 2000).event.unwrap().reason,
            Reason::InitialMet
        );
    }
    #[test]
    fn wall_clock_jump_keeps_monotonic_cooldown_and_reports_diagnostic() {
        let r = rule(Direction::Up);
        let mut state = step(&r, &RuleState::default(), "100", 0).state;
        state = step(&r, &state, "99", 1000).state;
        state = step(&r, &state, "100", 2000).state;
        let quote = input(&r, "100", 10_000);
        let result = evaluate(
            &r,
            &state,
            &quote,
            Context {
                clock: Clock {
                    utc_ms: UTC - 50_000,
                    mono_ms: 10_000,
                },
                generation: 1,
                source_now_utc_ms: UTC + 10_000,
            },
        )
        .unwrap();
        assert!(result.clock_shift);
        assert_eq!(result.event.unwrap().reason, Reason::CooldownDue);
        assert_eq!(result.state.sequence, 2);
    }
}
