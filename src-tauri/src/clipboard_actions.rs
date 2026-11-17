//! 剪切板主动动作的持久请求回执；正文仅从本实例加密历史读取。
use crate::{
    clipboard_history,
    store::{err, now_ms, AppResult, Store},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Mutex};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub request_id: String,
    pub action_revision: u64,
    pub config_revision: u64,
    pub presentation_id: String,
    pub item_id: String,
    pub item_revision: u64,
    pub paste: bool,
}
pub(crate) struct Permit {
    pub config_revision: u64,
    pub presentation_id: String,
    pub paste_enabled: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Reserve,
    BeforeCopy,
    AfterCopy,
    PrepareInput,
    Input,
    Finish,
}
pub(crate) enum Write {
    Copied(u32),
    Uncertain,
}
pub(crate) enum Paste {
    Submitted,
    Uncertain,
}
pub(crate) trait Desktop {
    fn ready(&mut self) -> AppResult<()>;
    fn write(&mut self, text: &str) -> AppResult<Write>;
    fn prepare_paste(&mut self) -> AppResult<()>;
    fn target_current(&mut self) -> bool;
    fn send_paste(&mut self) -> AppResult<Paste>;
}
#[derive(Default)]
pub(crate) struct Engine {
    effects: Mutex<()>,
    active: Mutex<Option<(String, String)>>,
}
const MAX_SAFE: u64 = 9_007_199_254_740_991;
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum State {
    Pending,
    Completed,
    Partial,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum CopyState {
    NotStarted,
    Copied,
    Failed,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum PasteState {
    NotRequested,
    NotStarted,
    Submitted,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Failure {
    WriteFailed,
    ClipboardUncertain,
    AuthorizationChanged,
    UsageFailed,
    TargetChanged,
    InputFailed,
    InputUncertain,
    StateSaveFailed,
    Interrupted,
}
impl Failure {
    fn message(&self) -> &'static str {
        match self {
            Self::WriteFailed => "剪贴板写入失败，未发送粘贴",
            Self::ClipboardUncertain => "复制结果不确定，未发送或重复粘贴",
            Self::AuthorizationChanged => "实例、配置或窗口授权已变化，未继续输入",
            Self::UsageFailed => "文本已复制，但使用记录未保存，未发送粘贴",
            Self::TargetChanged => "原目标窗口或前台已变化，未发送粘贴",
            Self::InputFailed => "粘贴按键提交失败，未重复发送",
            Self::InputUncertain => "部分按键可能已提交，结果不确定，未重复发送",
            Self::StateSaveFailed => "动作结果未完整保存，请确认实际状态；未重复执行",
            Self::Interrupted => "上次动作已中断或结果不确定，未重复执行",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
    revision: u64,
    state: State,
    copy: CopyState,
    paste: PasteState,
    code: Option<Failure>,
    at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u32,
    revision: u64,
    entries: Vec<Receipt>,
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id)
}
fn key(instance: &str) -> AppResult<String> {
    if !valid_id(instance) {
        return Err("INVALID_ARGUMENT: 剪切板动作实例无效".into());
    }
    Ok(format!("clipboard.actions/{instance}"))
}
fn load(db: &Connection, instance: &str) -> AppResult<Ledger> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 动作回执读取失败")?;
    let Some(raw) = raw else {
        return Ok(Ledger {
            version: 1,
            revision: 0,
            entries: Vec::new(),
        });
    };
    if raw.len() > 65536 {
        return Err("DATA_UNAVAILABLE: 动作回执超限，未覆盖".into());
    }
    let ledger: Ledger =
        serde_json::from_str(&raw).map_err(|_| "DATA_UNAVAILABLE: 动作回执损坏，未重新执行")?;
    let mut ids = HashSet::new();
    let mut revision = 0;
    if ledger.version != 1
        || ledger.revision > MAX_SAFE
        || ledger.entries.len() > 64
        || ledger.entries.iter().any(|r| {
            let invalid = !valid_id(&r.id)
                || !ids.insert(&r.id)
                || r.fingerprint.len() != 64
                || !r.fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
                || r.revision <= revision
                || r.revision > ledger.revision
                || r.at > MAX_SAFE
                || (r.state == State::Completed
                    && (r.copy != CopyState::Copied
                        || !matches!(r.paste, PasteState::NotRequested | PasteState::Submitted)
                        || r.code.is_some()))
                || (r.state == State::Pending && r.code.is_some())
                || (!matches!(r.state, State::Completed | State::Pending) && r.code.is_none());
            revision = r.revision;
            invalid
        })
        || (ledger.entries.last().map(|r| r.revision).unwrap_or(0) != ledger.revision)
    {
        return Err("DATA_UNAVAILABLE: 动作回执无效，未重新执行".into());
    }
    Ok(ledger)
}
fn save(db: &Connection, instance: &str, ledger: &Ledger, insert: bool) -> AppResult<()> {
    let raw = serde_json::to_string(ledger).map_err(|_| "STORAGE_ERROR: 动作回执编码失败")?;
    if raw.len() > 65536 {
        return Err("CAPACITY_EXCEEDED: 动作回执超限".into());
    }
    let changed=if insert {db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key(instance)?,raw])}
        else{db.execute("UPDATE app_settings SET value=?2 WHERE key=?1",params![key(instance)?,raw])}.map_err(|_|"STORAGE_ERROR: 动作回执保存失败，未重新执行")?;
    if changed != 1 {
        return Err("STATE_EXPIRED: 动作回执已移除，未恢复旧数据".into());
    }
    Ok(())
}
fn output(receipt: &Receipt, revision: u64, replayed: bool, active: bool) -> Value {
    let mut r = receipt.clone();
    if r.state == State::Pending && !active {
        r.state = State::Unknown;
        r.code = Some(Failure::Interrupted);
        if r.copy == CopyState::NotStarted {
            r.copy = CopyState::Unknown;
        }
        if r.paste == PasteState::NotStarted {
            r.paste = PasteState::Cancelled;
        }
    }
    json!({"requestId":r.id,"actionRevision":revision,"state":r.state,"copy":r.copy,"paste":r.paste,"code":r.code,"error":r.code.as_ref().map(Failure::message),"replayed":replayed})
}
fn permit_valid(request: &Request, permit: &Permit) -> AppResult<()> {
    if request.config_revision != permit.config_revision {
        return Err("REVISION_CONFLICT: 剪切板动作配置已变化".into());
    }
    if request.presentation_id != permit.presentation_id {
        return Err("PRESENTATION_EXPIRED: 呼出窗口已变化".into());
    }
    if request.paste && !permit.paste_enabled {
        return Err("CAPABILITY_DENIED: 尚未显式启用跨应用粘贴".into());
    }
    Ok(())
}
fn guarded<A>(
    authorized: &mut A,
    stage: Stage,
    mut run: impl FnMut(Permit) -> AppResult<Value>,
) -> AppResult<Value>
where
    A: FnMut(Stage, &mut dyn FnMut(Permit) -> AppResult<Value>) -> AppResult<Value>,
{
    let mut calls = 0;
    let value = authorized(stage, &mut |permit| {
        calls += 1;
        if calls != 1 {
            return Err("AUTHORIZATION_ERROR: 同一阶段不能重复执行".into());
        }
        run(permit)
    })?;
    if calls != 1 {
        return Err("AUTHORIZATION_ERROR: 动作阶段没有获得授权".into());
    }
    Ok(value)
}
fn checkpoint(store: &Store, instance: &str, receipt: &Receipt) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let mut ledger = load(&tx, instance)?;
    let entry = ledger
        .entries
        .iter_mut()
        .find(|r| r.id == receipt.id && r.fingerprint == receipt.fingerprint)
        .ok_or("STATE_EXPIRED: 动作回执已移除，未恢复旧数据")?;
    *entry = receipt.clone();
    save(&tx, instance, &ledger, false)?;
    tx.commit()
        .map_err(|_| "STORAGE_ERROR: 动作回执提交失败".into())
}
pub(crate) fn revision(store: &Store, instance: &str) -> AppResult<u64> {
    let db = store.db.lock().map_err(err)?;
    Ok(load(&db, instance)?.revision)
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(instance)?])
        .map_err(|_| "STORAGE_ERROR: 动作回执清理失败")?;
    Ok(())
}
enum Begin {
    Replay(Value),
    Work {
        receipt: Receipt,
        text: String,
        revision: u64,
    },
}
struct Flight<'a>(&'a Mutex<Option<(String, String)>>);
impl Drop for Flight<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}
impl Engine {
    pub(crate) fn latest(&self, store: &Store, instance: &str) -> AppResult<(u64, Value)> {
        let db = store.db.lock().map_err(err)?;
        let ledger = load(&db, instance)?;
        drop(db);
        let active = self.active.lock().map_err(err)?;
        let value = ledger
            .entries
            .last()
            .map(|receipt| {
                let running = active
                    .as_ref()
                    .is_some_and(|(owner, id)| owner == instance && id == &receipt.id);
                output(receipt, ledger.revision, true, running)
            })
            .unwrap_or(Value::Null);
        Ok((ledger.revision, value))
    }
    pub(crate) fn execute(
        &self,
        store: &Store,
        instance: &str,
        request: Request,
        desktop: &mut impl Desktop,
        mut authorized: impl FnMut(
            Stage,
            &mut dyn FnMut(Permit) -> AppResult<Value>,
        ) -> AppResult<Value>,
    ) -> AppResult<Value> {
        if !valid_id(&request.request_id)
            || !valid_id(&request.presentation_id)
            || !valid_id(&request.item_id)
            || request.item_revision == 0
            || request.item_revision > MAX_SAFE
            || request.config_revision > MAX_SAFE
            || request.action_revision > MAX_SAFE
        {
            return Err("INVALID_ARGUMENT: 剪切板动作标识或版本无效".into());
        }
        key(instance)?;
        let _effects = self.effects.lock().map_err(err)?;
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&request).map_err(|_| "INVALID_ARGUMENT: 动作参数编码失败")?
            )
        );
        let mut begin = None;
        guarded(&mut authorized, Stage::Reserve, |permit| {
            let mut db = store.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            let mut ledger = load(&tx, instance)?;
            if let Some(receipt) = ledger.entries.iter().find(|r| r.id == request.request_id) {
                if receipt.fingerprint != fingerprint {
                    return Err("REQUEST_CONFLICT: 同一动作请求不能更换内容".into());
                }
                begin = Some(Begin::Replay(output(receipt, ledger.revision, true, false)));
                return Ok(Value::Null);
            }
            permit_valid(&request, &permit)?;
            if request.action_revision != ledger.revision {
                return Err("REVISION_CONFLICT: 动作版本已变化，旧请求未再次执行".into());
            }
            // 读取正文不能重入SQLite锁；先退出本次只读事务，再由加密历史模型读取。
            drop(tx);
            drop(db);
            let item = clipboard_history::get(store, instance, &request.item_id)?;
            if item["revision"] != request.item_revision {
                return Err("REVISION_CONFLICT: 所选记录已变化".into());
            }
            let text = item["content"]
                .as_str()
                .ok_or("DATA_UNAVAILABLE: 所选记录文本无效")?
                .to_owned();
            let mut db = store.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            ledger = load(&tx, instance)?;
            if ledger.revision != request.action_revision {
                return Err("REVISION_CONFLICT: 动作版本已变化".into());
            }
            for old in &mut ledger.entries {
                if old.state == State::Pending {
                    old.state = State::Unknown;
                    old.code = Some(Failure::Interrupted);
                    if old.copy == CopyState::NotStarted {
                        old.copy = CopyState::Unknown;
                    }
                    if old.paste == PasteState::NotStarted {
                        old.paste = PasteState::Cancelled;
                    }
                }
            }
            ledger.revision = ledger
                .revision
                .checked_add(1)
                .filter(|v| *v <= MAX_SAFE)
                .ok_or("CAPACITY_EXCEEDED: 动作版本已达上限")?;
            let receipt = Receipt {
                id: request.request_id.clone(),
                fingerprint: fingerprint.clone(),
                revision: ledger.revision,
                state: State::Pending,
                copy: CopyState::NotStarted,
                paste: if request.paste {
                    PasteState::NotStarted
                } else {
                    PasteState::NotRequested
                },
                code: None,
                at: now_ms(),
            };
            ledger.entries.push(receipt.clone());
            if ledger.entries.len() > 64 {
                ledger.entries.remove(0);
            }
            save(&tx, instance, &ledger, true)?;
            tx.commit()
                .map_err(|_| "STORAGE_ERROR: 动作预留提交失败，未执行")?;
            begin = Some(Begin::Work {
                receipt,
                text,
                revision: ledger.revision,
            });
            Ok(Value::Null)
        })?;
        let (mut receipt, text, revision) = match begin.ok_or("AUTHORIZATION_ERROR: 动作未获准")?
        {
            Begin::Replay(value) => return Ok(value),
            Begin::Work {
                receipt,
                text,
                revision,
            } => (receipt, text, revision),
        };
        *self.active.lock().map_err(err)? = Some((instance.into(), request.request_id.clone()));
        let _flight = Flight(&self.active);
        let mut written = None;
        let before = guarded(&mut authorized, Stage::BeforeCopy, |permit| {
            permit_valid(&request, &permit)?;
            let item = clipboard_history::get(store, instance, &request.item_id)?;
            if item["revision"] != request.item_revision {
                return Err("REVISION_CONFLICT: 写入前所选记录已变化".into());
            }
            desktop.ready()?;
            written = Some(desktop.write(&text));
            Ok(Value::Null)
        });
        if before.is_err() && written.is_none() {
            receipt.state = State::Cancelled;
            receipt.code = Some(Failure::AuthorizationChanged);
            if request.paste {
                receipt.paste = PasteState::Cancelled;
            }
        } else {
            match written {
                Some(Ok(Write::Copied(sequence))) if sequence != 0 => {
                    receipt.copy = CopyState::Copied
                }
                Some(Err(_)) => {
                    receipt.copy = CopyState::Failed;
                    receipt.state = State::Failed;
                    receipt.code = Some(Failure::WriteFailed);
                }
                _ => {
                    receipt.copy = CopyState::Unknown;
                    receipt.state = State::Unknown;
                    receipt.code = Some(Failure::ClipboardUncertain);
                }
            }
            if receipt.copy != CopyState::Copied {
                if request.paste {
                    receipt.paste = PasteState::Cancelled;
                }
            } else {
                let used = guarded(&mut authorized, Stage::AfterCopy, |permit| {
                    permit_valid(&request, &permit)?;
                    checkpoint(store, instance, &receipt).map_err(|_| "CHECKPOINT_FAILED")?;
                    clipboard_history::mutate(store,instance,json!({"requestId":request.request_id,"command":{"type":"used","id":request.item_id,"expectedItemRevision":request.item_revision}})).map_err(|_|"USAGE_FAILED")?;
                    Ok(Value::Null)
                });
                if let Err(error) = used {
                    receipt.state = if error == "CHECKPOINT_FAILED" {
                        State::Unknown
                    } else {
                        State::Partial
                    };
                    receipt.code = Some(if error == "USAGE_FAILED" {
                        Failure::UsageFailed
                    } else if error == "CHECKPOINT_FAILED" {
                        Failure::StateSaveFailed
                    } else {
                        Failure::AuthorizationChanged
                    });
                    if request.paste {
                        receipt.paste = PasteState::Cancelled;
                    }
                } else if !request.paste {
                    receipt.state = State::Completed;
                } else {
                    let prepared = guarded(&mut authorized, Stage::PrepareInput, |permit| {
                        permit_valid(&request, &permit)?;
                        receipt.paste = PasteState::Unknown;
                        checkpoint(store, instance, &receipt)?;
                        Ok(Value::Null)
                    });
                    if prepared.is_err() {
                        receipt.state = State::Partial;
                        receipt.paste = PasteState::Cancelled;
                        receipt.code = Some(Failure::AuthorizationChanged);
                    } else if desktop.prepare_paste().is_err() {
                        receipt.state = State::Partial;
                        receipt.paste = PasteState::Cancelled;
                        receipt.code = Some(Failure::TargetChanged);
                    } else {
                        let mut input = None;
                        let sent = guarded(&mut authorized, Stage::Input, |permit| {
                            permit_valid(&request, &permit)?;
                            let current =
                                clipboard_history::get(store, instance, &request.item_id)?;
                            if current["revision"] != request.item_revision + 1 {
                                return Err("REVISION_CONFLICT: 输入前所选记录再次变化".into());
                            }
                            if !desktop.target_current() {
                                return Err("TARGET_CHANGED".into());
                            }
                            input = Some(desktop.send_paste());
                            Ok(Value::Null)
                        });
                        match (sent, input) {
                            (Ok(_), Some(Ok(Paste::Submitted))) => {
                                receipt.state = State::Completed;
                                receipt.paste = PasteState::Submitted;
                            }
                            (_, Some(Ok(Paste::Uncertain))) => {
                                receipt.state = State::Unknown;
                                receipt.paste = PasteState::Unknown;
                                receipt.code = Some(Failure::InputUncertain);
                            }
                            (_, Some(Ok(Paste::Submitted))) => {
                                receipt.state = State::Unknown;
                                receipt.paste = PasteState::Submitted;
                                receipt.code = Some(Failure::AuthorizationChanged);
                            }
                            (_, Some(Err(_))) => {
                                receipt.state = State::Partial;
                                receipt.paste = PasteState::Failed;
                                receipt.code = Some(Failure::InputFailed);
                            }
                            _ => {
                                receipt.state = State::Partial;
                                receipt.paste = PasteState::Cancelled;
                                receipt.code = Some(Failure::TargetChanged);
                            }
                        }
                    }
                }
            }
        }
        let terminal = guarded(&mut authorized, Stage::Finish, |permit| {
            permit_valid(&request, &permit)?;
            checkpoint(store, instance, &receipt)?;
            Ok(Value::Null)
        });
        if terminal.is_err() {
            receipt.state = State::Unknown;
            receipt.code = Some(Failure::StateSaveFailed);
        }
        Ok(output(&receipt, revision, false, true))
    }
    pub(crate) fn status(&self, store: &Store, instance: &str, request: &str) -> AppResult<Value> {
        let db = store.db.lock().map_err(err)?;
        let ledger = load(&db, instance)?;
        drop(db);
        let receipt = ledger
            .entries
            .iter()
            .find(|r| r.id == request)
            .ok_or("NOT_FOUND: 动作回执不存在")?;
        let active = self
            .active
            .lock()
            .map_err(err)?
            .as_ref()
            .is_some_and(|(owner, id)| owner == instance && id == request);
        Ok(output(receipt, ledger.revision, true, active))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::clipboard_history;
    use serde_json::json;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn fixture() -> (Store, String, Request) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts")
            .join(format!("widgetjs-clipboard-actions-test-{}", uid()));
        let s = Store::open(root).unwrap();
        let instance = uid();
        clipboard_history::mutate(&s,&instance,json!({"requestId":uid(),"command":{"type":"configure","expectedRevision":0,"captureEnabled":true,"shortcutEnabled":false,"pasteEnabled":true,"shortcut":"Super+Alt+V","historyExpiredHours":12}})).unwrap();
        clipboard_history::capture(
            &s,
            &instance,
            1,
            "fixture-正文不入回执🙂",
            crate::store::now_ms(),
        )
        .unwrap();
        let item = clipboard_history::query(&s, &instance, json!({})).unwrap()["items"][0].clone();
        let request = Request {
            request_id: uid(),
            action_revision: 0,
            config_revision: 1,
            presentation_id: uid(),
            item_id: item["id"].as_str().unwrap().into(),
            item_revision: 1,
            paste: true,
        };
        (s, instance, request)
    }
    #[derive(Default)]
    struct Fake {
        writes: usize,
        prepares: usize,
        sends: usize,
        text: String,
        write_fail: bool,
        write_unknown: bool,
        prepare_fail: bool,
        target_changed: bool,
        input_unknown: bool,
        input_fail: bool,
        effect_guard: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }
    impl Desktop for Fake {
        fn ready(&mut self) -> AppResult<()> {
            if let Some(guard) = &self.effect_guard {
                assert!(
                    guard.load(std::sync::atomic::Ordering::Acquire),
                    "系统动作必须在授权锁内核对"
                );
            }
            Ok(())
        }
        fn write(&mut self, text: &str) -> AppResult<Write> {
            if let Some(guard) = &self.effect_guard {
                assert!(
                    guard.load(std::sync::atomic::Ordering::Acquire),
                    "剪贴板写入必须在授权锁内"
                );
            }
            self.writes += 1;
            self.text = text.into();
            if self.write_fail {
                Err("fixture write failed".into())
            } else if self.write_unknown {
                Ok(Write::Uncertain)
            } else {
                Ok(Write::Copied(77))
            }
        }
        fn prepare_paste(&mut self) -> AppResult<()> {
            self.prepares += 1;
            if self.prepare_fail {
                Err("fixture focus denied".into())
            } else {
                Ok(())
            }
        }
        fn target_current(&mut self) -> bool {
            !self.target_changed
        }
        fn send_paste(&mut self) -> AppResult<Paste> {
            if let Some(guard) = &self.effect_guard {
                assert!(
                    guard.load(std::sync::atomic::Ordering::Acquire),
                    "按键提交必须在授权锁内"
                );
            }
            self.sends += 1;
            if self.input_fail {
                Err("fixture input failed".into())
            } else if self.input_unknown {
                Ok(Paste::Uncertain)
            } else {
                Ok(Paste::Submitted)
            }
        }
    }
    fn run(
        engine: &Engine,
        s: &Store,
        instance: &str,
        request: Request,
        desktop: &mut Fake,
    ) -> AppResult<Value> {
        let presentation = request.presentation_id.clone();
        engine.execute(s, instance, request, desktop, |_, op| {
            op(Permit {
                config_revision: 1,
                presentation_id: presentation.clone(),
                paste_enabled: true,
            })
        })
    }
    #[test]
    fn clipboard_actions_success_records_copy_and_submitted_input_without_claiming_application_delivery(
    ) {
        let (s, id, request) = fixture();
        let engine = Engine::default();
        let mut io = Fake::default();
        let result = run(&engine, &s, &id, request.clone(), &mut io).unwrap();
        assert_eq!(result["state"], "completed");
        assert_eq!(result["copy"], "copied");
        assert_eq!(result["paste"], "submitted");
        assert_eq!(result["actionRevision"], 1);
        assert_eq!((io.writes, io.prepares, io.sends), (1, 1, 1));
        assert_eq!(io.text, "fixture-正文不入回执🙂");
        assert_eq!(
            clipboard_history::get(&s, &id, &request.item_id).unwrap()["useCount"],
            1
        );
        let raw: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [format!("clipboard.actions/{id}")],
                    |r| r.get(0),
                )
                .unwrap();
        assert!(!raw.contains("正文"));
        assert!(!raw.contains("fixture-"));
    }
    #[test]
    fn clipboard_actions_same_uuid_retries_and_restart_never_repeat_clipboard_or_input() {
        let (s, id, request) = fixture();
        let engine = Engine::default();
        let mut io = Fake::default();
        run(&engine, &s, &id, request.clone(), &mut io).unwrap();
        let replay = run(&Engine::default(), &s, &id, request.clone(), &mut io).unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!((io.writes, io.sends), (1, 1));
        let mut conflict = request;
        conflict.paste = false;
        assert!(run(&engine, &s, &id, conflict, &mut io)
            .unwrap_err()
            .contains("REQUEST_CONFLICT"));
        assert_eq!(io.sends, 1);
    }
    #[test]
    fn clipboard_actions_reservation_failure_cannot_touch_clipboard_or_count_usage() {
        let (s, id, request) = fixture();
        let engine = Engine::default();
        let mut io = Fake::default();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_action BEFORE INSERT ON app_settings WHEN new.key LIKE 'clipboard.actions/%' BEGIN SELECT RAISE(FAIL,'fixture denied'); END;").unwrap();
        assert!(run(&engine, &s, &id, request.clone(), &mut io)
            .unwrap_err()
            .contains("STORAGE_ERROR"));
        assert_eq!((io.writes, io.sends), (0, 0));
        assert_eq!(
            clipboard_history::get(&s, &id, &request.item_id).unwrap()["useCount"],
            0
        );
    }
    #[test]
    fn clipboard_actions_profile_or_presentation_change_blocks_every_effect() {
        let (s, id, request) = fixture();
        let engine = Engine::default();
        let mut io = Fake::default();
        let token = request.presentation_id.clone();
        assert!(engine
            .execute(&s, &id, request.clone(), &mut io, |_, op| op(Permit {
                config_revision: 2,
                presentation_id: token.clone(),
                paste_enabled: true
            }))
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
        assert!(engine
            .execute(&s, &id, request.clone(), &mut io, |_, op| op(Permit {
                config_revision: 1,
                presentation_id: uid(),
                paste_enabled: true
            }))
            .unwrap_err()
            .contains("PRESENTATION_EXPIRED"));
        assert!(engine
            .execute(&s, &id, request, &mut io, |_, op| op(Permit {
                config_revision: 1,
                presentation_id: token.clone(),
                paste_enabled: false
            }))
            .unwrap_err()
            .contains("CAPABILITY_DENIED"));
        assert_eq!((io.writes, io.sends), (0, 0));
    }
    #[test]
    fn clipboard_actions_copy_only_never_restores_foreground_or_sends_input() {
        let (s, id, mut request) = fixture();
        request.paste = false;
        let mut io = Fake::default();
        let token = request.presentation_id.clone();
        let result = Engine::default()
            .execute(&s, &id, request, &mut io, |_, op| {
                op(Permit {
                    config_revision: 1,
                    presentation_id: token.clone(),
                    paste_enabled: false,
                })
            })
            .unwrap();
        assert_eq!(result["state"], "completed");
        assert_eq!(result["paste"], "not-requested");
        assert_eq!((io.writes, io.prepares, io.sends), (1, 0, 0));
    }
    #[test]
    fn clipboard_actions_write_failure_or_uncertainty_never_sends_input_or_retries() {
        for unknown in [false, true] {
            let (s, id, request) = fixture();
            let engine = Engine::default();
            let mut io = Fake {
                write_fail: !unknown,
                write_unknown: unknown,
                ..Default::default()
            };
            let result = run(&engine, &s, &id, request.clone(), &mut io).unwrap();
            assert_ne!(result["state"], "completed");
            assert_eq!((io.prepares, io.sends), (0, 0));
            assert_eq!(
                clipboard_history::get(&s, &id, &request.item_id).unwrap()["useCount"],
                0
            );
            run(&engine, &s, &id, request, &mut io).unwrap();
            assert_eq!(io.writes, 1);
        }
    }
    #[test]
    fn clipboard_actions_focus_failure_or_changed_target_keeps_actual_copy_but_sends_no_input() {
        for changed in [false, true] {
            let (s, id, request) = fixture();
            let mut io = Fake {
                prepare_fail: !changed,
                target_changed: changed,
                ..Default::default()
            };
            let result = run(&Engine::default(), &s, &id, request.clone(), &mut io).unwrap();
            assert_eq!(result["state"], "partial");
            assert_eq!(result["copy"], "copied");
            assert_eq!(result["paste"], "cancelled");
            assert_eq!(io.sends, 0);
            assert_eq!(
                clipboard_history::get(&s, &id, &request.item_id).unwrap()["useCount"],
                1
            );
        }
    }
    #[test]
    fn clipboard_actions_partial_input_or_failed_terminal_save_never_replays_input() {
        let (s, id, request) = fixture();
        let engine = Engine::default();
        let mut io = Fake {
            input_unknown: true,
            ..Default::default()
        };
        let result = run(&engine, &s, &id, request.clone(), &mut io).unwrap();
        assert_eq!(result["state"], "unknown");
        assert_eq!(result["paste"], "unknown");
        run(&Engine::default(), &s, &id, request, &mut io).unwrap();
        assert_eq!(io.sends, 1);
        let (s, id, request) = fixture();
        let token = request.presentation_id.clone();
        let mut io = Fake::default();
        let result=engine.execute(&s,&id,request.clone(),&mut io,|stage,op|{if stage==Stage::Finish{s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_finish BEFORE UPDATE ON app_settings WHEN new.key LIKE 'clipboard.actions/%' BEGIN SELECT RAISE(FAIL,'fixture denied'); END;").unwrap();}op(Permit{config_revision:1,presentation_id:token.clone(),paste_enabled:true})}).unwrap();
        assert_eq!(result["state"], "unknown");
        assert_eq!(io.sends, 1);
        let replay = run(&Engine::default(), &s, &id, request, &mut io).unwrap();
        assert_eq!(replay["state"], "unknown");
        assert_eq!(io.sends, 1);
    }
    #[test]
    fn clipboard_actions_cancelled_after_copy_or_deleted_before_finish_cannot_paste_or_recreate_ledger(
    ) {
        let (s, id, request) = fixture();
        let engine = Engine::default();
        let token = request.presentation_id.clone();
        let mut io = Fake::default();
        let result = engine
            .execute(&s, &id, request.clone(), &mut io, |stage, op| {
                if stage == Stage::PrepareInput {
                    return Err("SESSION_EXPIRED: fixture stopped".into());
                }
                op(Permit {
                    config_revision: 1,
                    presentation_id: token.clone(),
                    paste_enabled: true,
                })
            })
            .unwrap();
        assert_ne!(result["state"], "completed");
        assert_eq!(io.sends, 0);
        let (s, id, request) = fixture();
        let token = request.presentation_id.clone();
        let mut io = Fake::default();
        engine
            .execute(&s, &id, request, &mut io, |stage, op| {
                if stage == Stage::Finish {
                    s.db.lock()
                        .unwrap()
                        .execute(
                            "DELETE FROM app_settings WHERE key=?1",
                            [format!("clipboard.actions/{id}")],
                        )
                        .unwrap();
                }
                op(Permit {
                    config_revision: 1,
                    presentation_id: token.clone(),
                    paste_enabled: true,
                })
            })
            .unwrap();
        let count: i64 =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM app_settings WHERE key=?1",
                    [format!("clipboard.actions/{id}")],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn clipboard_actions_new_capture_does_not_conflict_but_changed_item_version_does() {
        let (s, id, request) = fixture();
        clipboard_history::capture(&s, &id, 1, "后台的新复制", crate::store::now_ms()).unwrap();
        let mut io = Fake::default();
        run(&Engine::default(), &s, &id, request, &mut io).unwrap();
        assert_eq!(io.writes, 1);
        let (s, id, mut request) = fixture();
        request.item_revision = 2;
        let mut io = Fake::default();
        assert!(run(&Engine::default(), &s, &id, request, &mut io)
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
        assert_eq!(io.writes, 0);
    }

    #[test]
    fn clipboard_actions_authorization_guard_spans_actual_copy_and_input_not_just_preflight() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let (s, id, request) = fixture();
        let presentation = request.presentation_id.clone();
        let guard = Arc::new(AtomicBool::new(false));
        let mut io = Fake {
            effect_guard: Some(guard.clone()),
            ..Default::default()
        };
        let value = Engine::default()
            .execute(&s, &id, request, &mut io, |_, op| {
                guard.store(true, Ordering::Release);
                let result = op(Permit {
                    config_revision: 1,
                    presentation_id: presentation.clone(),
                    paste_enabled: true,
                });
                guard.store(false, Ordering::Release);
                result
            })
            .unwrap();
        assert_eq!(value["state"], "completed");
        assert_eq!((io.writes, io.sends), (1, 1));
    }

    #[test]
    fn clipboard_actions_restart_pending_receipt_reports_unknown_without_any_effect() {
        let (s, id, request) = fixture();
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&request).unwrap())
        );
        let receipt = Receipt {
            id: request.request_id.clone(),
            fingerprint,
            revision: 1,
            state: State::Pending,
            copy: CopyState::Copied,
            paste: PasteState::Unknown,
            code: None,
            at: now_ms(),
        };
        save(
            &s.db.lock().unwrap(),
            &id,
            &Ledger {
                version: 1,
                revision: 1,
                entries: vec![receipt],
            },
            true,
        )
        .unwrap();
        let engine = Engine::default();
        let status = engine.status(&s, &id, &request.request_id).unwrap();
        assert_eq!(status["state"], "unknown");
        assert_eq!(status["copy"], "copied");
        assert_eq!(status["paste"], "unknown");
        let mut io = Fake::default();
        let replay = run(&engine, &s, &id, request, &mut io).unwrap();
        assert_eq!(replay["state"], "unknown");
        assert_eq!((io.writes, io.sends), (0, 0));
    }

    #[test]
    fn clipboard_actions_uuid_rotation_keeps_old_action_version_from_repeating_copy() {
        let (s, id, mut request) = fixture();
        request.paste = false;
        let old = request.clone();
        let engine = Engine::default();
        let mut io = Fake::default();
        for n in 0..65 {
            request.request_id = if n == 0 {
                old.request_id.clone()
            } else {
                uid()
            };
            request.action_revision = n;
            request.item_revision = n + 1;
            assert_eq!(
                run(&engine, &s, &id, request.clone(), &mut io).unwrap()["state"],
                "completed"
            );
        }
        assert_eq!(revision(&s, &id).unwrap(), 65);
        assert_eq!(io.writes, 65);
        let ledger = load(&s.db.lock().unwrap(), &id).unwrap();
        assert_eq!(ledger.entries.len(), 64);
        assert!(run(&engine, &s, &id, old, &mut io)
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
        assert_eq!(io.writes, 65);
    }

    #[test]
    fn clipboard_actions_corrupt_or_unknown_ledger_never_overwrites_or_touches_clipboard() {
        let (s, id, request) = fixture();
        let mut io = Fake::default();
        let engine = Engine::default();
        for raw in ["broken-json", r#"{"version":2,"revision":0,"entries":[]}"#] {
            s.save_preference(&key(&id).unwrap(), raw).unwrap();
            assert!(run(&engine, &s, &id, request.clone(), &mut io)
                .unwrap_err()
                .contains("DATA_UNAVAILABLE"));
            let actual: String =
                s.db.lock()
                    .unwrap()
                    .query_row(
                        "SELECT value FROM app_settings WHERE key=?1",
                        [key(&id).unwrap()],
                        |r| r.get(0),
                    )
                    .unwrap();
            assert_eq!(actual, raw);
        }
        assert_eq!((io.writes, io.sends), (0, 0));
    }
    #[test]
    fn clipboard_actions_semantically_impossible_success_receipt_is_not_reported_as_completed() {
        let (s, id, request) = fixture();
        let receipt = Receipt {
            id: request.request_id.clone(),
            fingerprint: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&request).unwrap())
            ),
            revision: 1,
            state: State::Completed,
            copy: CopyState::NotStarted,
            paste: PasteState::Submitted,
            code: None,
            at: now_ms(),
        };
        save(
            &s.db.lock().unwrap(),
            &id,
            &Ledger {
                version: 1,
                revision: 1,
                entries: vec![receipt],
            },
            true,
        )
        .unwrap();
        assert!(Engine::default()
            .status(&s, &id, &request.request_id)
            .unwrap_err()
            .contains("DATA_UNAVAILABLE"));
        let mut io = Fake::default();
        assert!(run(&Engine::default(), &s, &id, request, &mut io).is_err());
        assert_eq!((io.writes, io.sends), (0, 0));
    }
}
