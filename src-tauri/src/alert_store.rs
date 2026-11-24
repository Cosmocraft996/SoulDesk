//! 规则状态与逻辑事件原子保存；系统递送只能在提交之后开始。
use crate::{
    alerts::{self, AlertEvent, Context, Observation, Rule, RuleState},
    store::{err, AppResult, Store},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

const HISTORY_DAYS_KEY: &str = "alerts.history.retention_days";
const DEFAULT_HISTORY_DAYS: u32 = 3;
const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

pub fn save_rule(store: &Store, rule: &Rule, expected_revision: Option<u64>) -> AppResult<()> {
    rule.validate()?;
    let mut db = store.db.lock().map_err(err)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let previous: Option<(u64, String, String)> = tx
        .query_row(
            "SELECT revision,definition,state FROM alert_rules WHERE id=?1",
            [&rule.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(err)?;
    let state = match (expected_revision, previous) {
        (None, None) if rule.revision == 1 => RuleState::default(),
        (Some(expected), Some((revision, definition, state))) if expected == revision => {
            alerts::revise(
                &serde_json::from_str(&definition).map_err(err)?,
                rule,
                &serde_json::from_str(&state).map_err(err)?,
            )?
        }
        _ => return Err("CONFLICT: 规则不存在、已存在或版本已变化，请刷新".into()),
    };
    tx.execute("INSERT INTO alert_rules(id,revision,definition,state,enabled,market_key) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,definition=excluded.definition,state=excluded.state,enabled=excluded.enabled,market_key=excluded.market_key",params![rule.id,rule.revision,serde_json::to_string(rule).map_err(err)?,serde_json::to_string(&state).map_err(err)?,rule.enabled,serde_json::to_string(&rule.key).map_err(err)?]).map_err(err)?;
    tx.commit().map_err(err)
}

pub fn list_rules(store: &Store) -> AppResult<Vec<(Rule, RuleState)>> {
    let db = store.db.lock().map_err(err)?;
    let mut query = db
        .prepare("SELECT definition,state FROM alert_rules ORDER BY rowid")
        .map_err(err)?;
    let values = query
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?
        .map(|row| {
            let (rule, state) = row.map_err(err)?;
            Ok((
                serde_json::from_str(&rule).map_err(err)?,
                serde_json::from_str(&state).map_err(err)?,
            ))
        })
        .collect();
    values
}

#[derive(Default)]
pub struct Engine {
    states: HashMap<String, (u64, RuleState)>,
}
impl Engine {
    pub fn process(
        &mut self,
        store: &Store,
        quote: &Observation,
        context: Context,
    ) -> AppResult<Vec<AlertEvent>> {
        let mut db = store.db.lock().map_err(err)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let rows = {
            let mut query = tx
                .prepare(
                    "SELECT definition,state FROM alert_rules WHERE market_key=?1 AND enabled=1",
                )
                .map_err(err)?;
            let rows = query
                .query_map([serde_json::to_string(&quote.key).map_err(err)?], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(err)?
        };
        let mut updates = Vec::new();
        let mut events = Vec::new();
        for (definition, persisted) in rows {
            let rule: Rule = serde_json::from_str(&definition).map_err(err)?;
            let loaded: RuleState = serde_json::from_str(&persisted).map_err(err)?;
            let state = self
                .states
                .get(&rule.id)
                // 保存时只有触发参数变更会把持久化状态重置为未初始化。
                // 渠道和名称变更可保留同一进程内的上一笔基准。
                .filter(|(revision, state)| {
                    loaded.initialized
                        && loaded.sequence == state.sequence
                        && (*revision == rule.revision
                            || rule.condition == alerts::Condition::Cross)
                })
                .map(|(_, state)| state)
                .unwrap_or(&loaded);
            let result = alerts::evaluate(&rule, state, quote, context)?;
            if result.clock_shift {
                store.log(
                    "clock_shift",
                    &format!(
                        "规则 {} 本地UTC={} 源校时UTC={} 单调时间={}",
                        rule.id,
                        context.clock.utc_ms,
                        context.source_now_utc_ms,
                        context.clock.mono_ms
                    ),
                )?;
            }
            let encoded = serde_json::to_string(&result.state).map_err(err)?;
            if encoded != persisted {
                tx.execute(
                    "UPDATE alert_rules SET state=?1 WHERE id=?2 AND revision=?3",
                    params![encoded, rule.id, rule.revision],
                )
                .map_err(err)?;
            }
            if let Some(event) = result.event {
                let sound = if event.notification
                    && event.notification_channel == alerts::NotificationChannel::Island
                {
                    "island"
                } else {
                    "suppressed"
                };
                tx.execute("INSERT INTO alert_events(id,rule_id,revision,sequence,event,created,notification,sound) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![event.id,event.rule_id,event.revision,event.sequence,serde_json::to_string(&event).map_err(err)?,event.created_utc_ms,if event.notification {"pending"}else{"suppressed"},sound]).map_err(err)?;
                events.push(event);
            }
            updates.push((rule.id, (rule.revision, result.state)));
        }
        tx.commit().map_err(err)?;
        // 只有事务提交成功才推进内存去重、单调时钟和运行状态。
        self.states.extend(updates);
        Ok(events)
    }
    pub fn retain_rules(&mut self, store: &Store) -> AppResult<()> {
        let ids = list_rules(store)?
            .into_iter()
            .map(|(r, _)| r.id)
            .collect::<std::collections::HashSet<_>>();
        self.states.retain(|id, _| ids.contains(id));
        Ok(())
    }
}

pub fn list_events(store: &Store, offset: u64, limit: u32) -> AppResult<Vec<Value>> {
    let db = store.db.lock().map_err(err)?;
    let mut query=db.prepare("SELECT event,notification,sound,notification_attempts,sound_attempts,notification_error,sound_error FROM alert_events WHERE coalesce(json_extract(event,'$.historyCleared'),0)=0 ORDER BY created DESC,rowid DESC LIMIT ?1 OFFSET ?2").map_err(err)?;
    let rows = query
        .query_map(params![limit.clamp(1, 100), offset], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(err)?;
    rows.map(|row| {
        let (event, n, s, na, sa, ne, se) = row.map_err(err)?;
        let mut value: Value = serde_json::from_str(&event).map_err(err)?;
        value["notification"] = json!(n);
        value["sound"] = json!(s);
        value["notificationAttempts"] = json!(na);
        value["soundAttempts"] = json!(sa);
        value["notificationError"] = json!(ne);
        value["soundError"] = json!(se);
        Ok(value)
    })
    .collect()
}

fn history_days(db: &Connection) -> AppResult<Option<u32>> {
    let value = crate::reminder_store::preference(db, HISTORY_DAYS_KEY)?;
    let days = value
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_HISTORY_DAYS);
    Ok((days > 0).then_some(days))
}

pub(crate) fn history_settings(store: &Store, days: Option<&Value>) -> AppResult<Value> {
    let db = store.db.lock().map_err(err)?;
    if let Some(value) = days {
        let days = if value.is_null() {
            0
        } else {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|&value| value > 0)
                .ok_or("INVALID_ARGUMENT: 保留天数必须为正整数")?
        };
        db.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![HISTORY_DAYS_KEY, days.to_string()],
        ).map_err(err)?;
    }
    Ok(json!({"days":history_days(&db)?}))
}

pub(crate) fn clear_history(store: &Store) -> AppResult<usize> {
    // 报警事件也是通知动作的依据。先从日志清除，后台再按保留期限删除数据。
    store.db.lock().map_err(err)?.execute(
        "UPDATE alert_events SET event=json_set(event,'$.historyCleared',1) WHERE coalesce(json_extract(event,'$.historyCleared'),0)=0",
        [],
    ).map_err(err)
}

pub(crate) fn prune_history(store: &Store, now: u64, active: &[&str]) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let cutoff = |days: u32| now.saturating_sub(u64::from(days) * DAY_MS);
    let visible_cutoff = history_days(&tx)?.map(cutoff).unwrap_or(0);
    let cleared_cutoff = cutoff(DEFAULT_HISTORY_DAYS).max(visible_cutoff);
    tx.execute(
        "DELETE FROM alert_events
         WHERE (created < ?1 OR (created < ?2 AND json_extract(event,'$.historyCleared')=1))
           AND notification NOT IN ('pending','sending')
           AND id NOT IN (SELECT value FROM json_each(?3))",
        params![
            visible_cutoff,
            cleared_cutoff,
            serde_json::to_string(active).map_err(err)?
        ],
    )
    .map_err(err)?;
    tx.execute(
        "UPDATE alert_events SET event=json_set(event,'$.historyCleared',1)
         WHERE created < ?1 AND coalesce(json_extract(event,'$.historyCleared'),0)=0",
        [visible_cutoff],
    )
    .map_err(err)?;
    tx.commit().map_err(err)
}

pub(crate) fn notification_event(store: &Store, id: &str) -> AppResult<AlertEvent> {
    let db = store.db.lock().map_err(err)?;
    let raw: String = db
        .query_row("SELECT event FROM alert_events WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(err)?
        .ok_or("该警报已经找不到")?;
    let event: AlertEvent = serde_json::from_str(&raw).map_err(err)?;
    if event.id != id || !event.notification {
        return Err("该警报没有通知入口".into());
    }
    event.key.validate()?;
    Ok(event)
}

#[derive(Clone, Copy, Debug)]
pub enum Channel {
    Notification,
    #[cfg(test)]
    Sound,
}
impl Channel {
    fn column(self) -> &'static str {
        match self {
            Self::Notification => "notification",
            #[cfg(test)]
            Self::Sound => "sound",
        }
    }
}
pub fn claim_delivery(store: &Store, id: &str, channel: Channel) -> AppResult<bool> {
    let column = channel.column();
    let sql=format!("UPDATE alert_events SET {column}='sending',{column}_attempts={column}_attempts+1 WHERE id=?1 AND {column}='pending' AND {column}_attempts<3");
    Ok(store
        .db
        .lock()
        .map_err(err)?
        .execute(&sql, [id])
        .map_err(err)?
        == 1)
}

pub fn finish_delivery(
    store: &Store,
    id: &str,
    channel: Channel,
    outcome: AppResult<()>,
) -> AppResult<()> {
    let column = channel.column();
    let (status, error) = match outcome {
        Ok(()) => ("sent", None),
        Err(message)
            if matches!(
                message.as_str(),
                crate::reminder_store::WINDOWS_DISABLED | crate::alert_notice::ISLAND_SUPPRESSED
            ) =>
        {
            ("suppressed", None)
        }
        Err(message) => (
            "failed",
            Some(message.chars().take(500).collect::<String>()),
        ),
    };
    let sql = format!(
        "UPDATE alert_events SET {column}=?1,{column}_error=?2 WHERE id=?3 AND {column}='sending'"
    );
    if store
        .db
        .lock()
        .map_err(err)?
        .execute(&sql, params![status, error, id])
        .map_err(err)?
        != 1
    {
        return Err("CONFLICT: 递送状态已变化".into());
    }
    Ok(())
}
pub fn finish_uncertain(store: &Store, id: &str, channel: Channel, message: &str) -> AppResult<()> {
    let column = channel.column();
    let sql=format!("UPDATE alert_events SET {column}='unknown',{column}_error=?1 WHERE id=?2 AND {column}='sending'");
    if store
        .db
        .lock()
        .map_err(err)?
        .execute(
            &sql,
            params![message.chars().take(500).collect::<String>(), id],
        )
        .map_err(err)?
        != 1
    {
        return Err("CONFLICT: 递送状态已变化".into());
    }
    Ok(())
}
pub fn retry_delivery(store: &Store, id: &str, channel: Channel) -> AppResult<()> {
    // 用户主动重试失败、不确定或过期的同一事件；发送中、已发送及关闭的渠道不重播。
    let column = channel.column();
    let sql=format!("UPDATE alert_events SET {column}='pending',{column}_error='用户主动重试' WHERE id=?1 AND {column} IN ('failed','unknown','expired') AND {column}_attempts<3 AND coalesce(json_extract(event,'$.historyCleared'),0)=0");
    if store
        .db
        .lock()
        .map_err(err)?
        .execute(&sql, [id])
        .map_err(err)?
        != 1
    {
        return Err("CONFLICT: 当前递送状态不能重试".into());
    }
    Ok(())
}
pub fn pending_deliveries(store: &Store, now: u64) -> AppResult<Vec<AlertEvent>> {
    let db = store.db.lock().map_err(err)?;
    // 已生成但尚未播出的旧版独立提示音停止排队，历史结果照常保留。
    db.execute(
        "UPDATE alert_events SET sound='suppressed',sound_error=NULL WHERE sound='pending'",
        [],
    )
    .map_err(err)?;
    db.execute("UPDATE alert_events SET notification='expired' WHERE notification='pending' AND created<?1 AND coalesce(notification_error,'')!='用户主动重试'",[now.saturating_sub(120_000)]).map_err(err)?;
    let mut query=db.prepare("SELECT event FROM alert_events WHERE notification='pending' AND notification_attempts<3 ORDER BY created,rowid LIMIT 50").map_err(err)?;
    let rows = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(err)?;
    rows.map(|s| serde_json::from_str(&s.map_err(err)?).map_err(err))
        .collect()
}
pub fn remove_rule(store: &Store, id: &str, revision: u64) -> AppResult<()> {
    if store
        .db
        .lock()
        .map_err(err)?
        .execute(
            "DELETE FROM alert_rules WHERE id=?1 AND revision=?2",
            params![id, revision],
        )
        .map_err(err)?
        != 1
    {
        return Err("CONFLICT: 规则已变化，请刷新".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_clock_jump_is_logged_without_creating_a_price_event() {
        let root =
            std::env::temp_dir().join(format!("souldesk-clock-log-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let mut engine = Engine::default();
        let (q, c) = quote(&r, "99", 0);
        engine.process(&store, &q, c).unwrap();
        let (q, mut c) = quote(&r, "99", 1000);
        c.clock.utc_ms += 300_000;
        assert!(engine.process(&store, &q, c).unwrap().is_empty());
        assert!(std::fs::read_to_string(root.join("logs/current.jsonl"))
            .unwrap()
            .contains("clock_shift"));
        assert!(list_events(&store, 0, 20).unwrap().is_empty());
    }
    #[test]
    fn an_interrupted_sender_is_unknown_without_a_replayable_failure() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-uncertain-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let (q, c) = quote(&r, "100", 0);
        let event = Engine::default().process(&store, &q, c).unwrap().remove(0);
        assert!(claim_delivery(&store, &event.id, Channel::Notification).unwrap());
        finish_uncertain(&store, &event.id, Channel::Notification, "发送任务中断").unwrap();
        assert_eq!(
            list_events(&store, 0, 20).unwrap()[0]["notification"],
            "unknown"
        );
        assert!(!claim_delivery(&store, &event.id, Channel::Notification).unwrap());
    }
    use crate::alerts::{Clock, Context, Direction, MarketKey, Mode, Observation, PriceKind};
    #[test]
    fn delivery_outcomes_retry_without_creating_a_new_price_event() {
        let root = std::env::temp_dir().join(format!("souldesk-outcome-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let (q, c) = quote(&r, "100", 0);
        let event = Engine::default().process(&store, &q, c).unwrap().remove(0);
        assert!(claim_delivery(&store, &event.id, Channel::Notification).unwrap());
        finish_delivery(
            &store,
            &event.id,
            Channel::Notification,
            Err("受控拒绝".into()),
        )
        .unwrap();
        assert!(retry_delivery(&store, &event.id, Channel::Notification).is_ok());
        assert!(claim_delivery(&store, &event.id, Channel::Notification).unwrap());
        finish_delivery(&store, &event.id, Channel::Notification, Ok(())).unwrap();
        assert!(retry_delivery(&store, &event.id, Channel::Notification).is_err());
        assert!(retry_delivery(&store, &event.id, Channel::Sound).is_err());
        let events = list_events(&store, 0, 20).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["notification"], "sent");
        assert_eq!(events[0]["notificationAttempts"], 2);
    }
    #[test]
    fn expired_queue_is_retained_and_rule_delete_checks_revision() {
        let root = std::env::temp_dir().join(format!("souldesk-expired-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let (q, c) = quote(&r, "100", 0);
        Engine::default().process(&store, &q, c).unwrap();
        assert!(pending_deliveries(&store, c.clock.utc_ms + 120_001)
            .unwrap()
            .is_empty());
        assert_eq!(
            list_events(&store, 0, 20).unwrap()[0]["notification"],
            "expired"
        );
        assert!(remove_rule(&store, &r.id, 2).is_err());
        remove_rule(&store, &r.id, 1).unwrap();
        assert!(list_rules(&store).unwrap().is_empty());
        assert_eq!(list_events(&store, 0, 20).unwrap().len(), 1);
    }
    fn rule() -> Rule {
        Rule {
            condition: Default::default(),
            notification_channel: Default::default(),
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: "持久化测试".into(),
            key: MarketKey::binance("BTCUSDT", PriceKind::Mark),
            direction: Direction::Up,
            threshold: "100".into(),
            mode: Mode::Once,
            cooldown_ms: 10_000,
            enabled: true,
            notification: true,
            sound: false,
        }
    }
    fn quote(r: &Rule, p: &str, t: u64) -> (Observation, Context) {
        (
            Observation {
                key: r.key.clone(),
                price: p.into(),
                source_utc_ms: 1_700_000_000_000 + t,
                received_mono_ms: t,
                generation: 1,
                ordinal: t + 1,
                recovered: false,
                valid: true,
            },
            Context {
                clock: Clock {
                    utc_ms: 1_700_000_000_000 + t,
                    mono_ms: t,
                },
                generation: 1,
                source_now_utc_ms: 1_700_000_000_000 + t,
            },
        )
    }
    #[test]
    fn old_json_and_notification_channel_edits_preserve_trigger_state() {
        let legacy = serde_json::to_value(rule()).unwrap();
        assert!(legacy.get("condition").is_none() && legacy.get("notificationChannel").is_none());
        let legacy: Rule = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.condition, alerts::Condition::Reach);
        assert_eq!(
            legacy.notification_channel,
            alerts::NotificationChannel::Windows
        );

        let root =
            std::env::temp_dir().join(format!("souldesk-alert-channel-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let mut crossing = Rule {
            condition: alerts::Condition::Cross,
            cooldown_ms: 0,
            ..legacy
        };
        save_rule(&store, &crossing, None).unwrap();
        let mut engine = Engine::default();
        let (observation, context) = quote(&crossing, "99", 0);
        assert!(engine
            .process(&store, &observation, context)
            .unwrap()
            .is_empty());
        crossing.revision += 1;
        crossing.notification_channel = alerts::NotificationChannel::Island;
        save_rule(&store, &crossing, Some(1)).unwrap();
        let (observation, context) = quote(&crossing, "100", 100);
        let events = engine.process(&store, &observation, context).unwrap();
        assert_eq!(events.len(), 1);
        let event = notification_event(&store, &events[0].id).unwrap();
        assert_eq!(event.reason, alerts::Reason::Crossed);
        assert_eq!(
            event.notification_channel,
            alerts::NotificationChannel::Island
        );
        assert_eq!(event.revision, 2);
    }

    #[test]
    fn state_and_event_commit_together_and_once_does_not_replay_on_restart() {
        let root =
            std::env::temp_dir().join(format!("souldesk-alert-store-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let mut engine = Engine::default();
        let (q, c) = quote(&r, "100", 0);
        assert_eq!(engine.process(&store, &q, c).unwrap().len(), 1);
        assert_eq!(list_events(&store, 0, 20).unwrap().len(), 1);
        drop(store);
        let store = Store::open(root).unwrap();
        let mut engine = Engine::default();
        let (q, c) = quote(&r, "101", 20_000);
        assert!(engine.process(&store, &q, c).unwrap().is_empty());
        assert_eq!(list_events(&store, 0, 20).unwrap().len(), 1);
    }
    #[test]
    fn failed_event_insert_rolls_back_state_and_does_not_advance_runtime_cache() {
        let root =
            std::env::temp_dir().join(format!("souldesk-alert-rollback-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let mut engine = Engine::default();
        let (q, c) = quote(&r, "99", 0);
        engine.process(&store, &q, c).unwrap();
        store.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_alert_event BEFORE INSERT ON alert_events BEGIN SELECT RAISE(ABORT,'受控写入失败'); END;").unwrap();
        let (q, c) = quote(&r, "100", 1);
        assert!(engine.process(&store, &q, c).is_err());
        assert_eq!(list_rules(&store).unwrap()[0].1.sequence, 0);
        assert!(list_events(&store, 0, 20).unwrap().is_empty());
        store
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_alert_event;")
            .unwrap();
        assert_eq!(engine.process(&store, &q, c).unwrap().len(), 1);
    }
    #[test]
    fn stale_editor_is_rejected_and_description_edit_preserves_completed_state() {
        let root =
            std::env::temp_dir().join(format!("souldesk-alert-revision-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let mut r = rule();
        save_rule(&store, &r, None).unwrap();
        let mut engine = Engine::default();
        let (q, c) = quote(&r, "100", 0);
        engine.process(&store, &q, c).unwrap();
        r.revision = 2;
        r.name = "新名称".into();
        save_rule(&store, &r, Some(1)).unwrap();
        assert!(list_rules(&store).unwrap()[0].1.completed);
        assert!(save_rule(&store, &r, Some(1)).is_err());
        assert_eq!(list_events(&store, 0, 20).unwrap().len(), 1);
    }
    #[test]
    fn delivery_claims_are_atomic_and_crash_recovery_keeps_uncertain_outcomes() {
        let root =
            std::env::temp_dir().join(format!("souldesk-alert-delivery-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let r = rule();
        save_rule(&store, &r, None).unwrap();
        let mut engine = Engine::default();
        let (q, c) = quote(&r, "100", 0);
        let event = engine.process(&store, &q, c).unwrap().remove(0);
        assert!(claim_delivery(&store, &event.id, Channel::Notification).unwrap());
        assert!(!claim_delivery(&store, &event.id, Channel::Notification).unwrap());
        assert!(!claim_delivery(&store, &event.id, Channel::Sound).unwrap());
        drop(store);
        let store = Store::open(root).unwrap();
        let events = list_events(&store, 0, 20).unwrap();
        assert_eq!(events[0]["notification"], "unknown");
        assert_eq!(events[0]["sound"], "suppressed");
        assert!(!claim_delivery(&store, &event.id, Channel::Notification).unwrap());
    }

    #[test]
    fn history_cleanup_protects_active_deliveries_and_keeps_rules_and_new_events() {
        let root =
            std::env::temp_dir().join(format!("souldesk-alert-history-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let r = Rule {
            mode: Mode::Repeat,
            ..rule()
        };
        save_rule(&store, &r, None).unwrap();
        assert_eq!(history_settings(&store, None).unwrap(), json!({"days":3}));
        assert!(history_settings(&store, Some(&json!(0))).is_err());

        let mut engine = Engine::default();
        let mut trigger = |t| {
            let (q, c) = quote(&r, "100", t);
            engine.process(&store, &q, c).unwrap().remove(0)
        };
        let old = trigger(0);
        let active = trigger(20_000);
        let pending = trigger(40_000);
        let recent = trigger(4 * DAY_MS);
        for event in [&old, &active, &recent] {
            assert!(claim_delivery(&store, &event.id, Channel::Notification).unwrap());
            finish_delivery(&store, &event.id, Channel::Notification, Ok(())).unwrap();
        }
        prune_history(&store, recent.created_utc_ms, &[&active.id]).unwrap();
        assert!(notification_event(&store, &old.id).is_err());
        assert!(notification_event(&store, &active.id).is_ok());
        assert_eq!(list_events(&store, 0, 20).unwrap().len(), 1);

        assert_eq!(clear_history(&store).unwrap(), 1);
        assert!(list_events(&store, 0, 20).unwrap().is_empty());
        assert!(notification_event(&store, &recent.id).is_ok());
        assert!(claim_delivery(&store, &pending.id, Channel::Notification).unwrap());
        finish_delivery(&store, &pending.id, Channel::Notification, Ok(())).unwrap();
        let after_clear = trigger(4 * DAY_MS + 20_000);
        assert_eq!(after_clear.sequence, 5);
        assert_eq!(list_events(&store, 0, 20).unwrap()[0]["id"], after_clear.id);
        assert_eq!(list_rules(&store).unwrap()[0].1.sequence, 5);
        assert_eq!(
            history_settings(&store, Some(&json!(7))).unwrap(),
            json!({"days":7})
        );
        assert_eq!(
            history_settings(&store, Some(&Value::Null)).unwrap(),
            json!({"days":null})
        );
        drop(store);

        let store = Store::open(root).unwrap();
        assert_eq!(
            history_settings(&store, None).unwrap(),
            json!({"days":null})
        );
        prune_history(&store, recent.created_utc_ms + 8 * DAY_MS, &[]).unwrap();
        assert!(notification_event(&store, &active.id).is_err());
        assert!(notification_event(&store, &pending.id).is_err());
        assert_eq!(list_events(&store, 0, 20).unwrap()[0]["id"], after_clear.id);
    }
}
