//! AI直聘的本地任务与岗位状态。这里只操作调用方连接，不执行网页或模型请求。
use crate::{
    job_protocol::{self, Job, Rating, Search},
    store::{err, AppResult},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

const MAX_SAFE: u64 = 9_007_199_254_740_991;
const MAX_JOBS: usize = 5000;
const DATA_ERROR: &str = "DATA_UNAVAILABLE: AI直聘本地数据损坏或版本无效，原数据未覆盖";
const STALE: &str = "PRESENTATION_EXPIRED: AI直聘任务或条件已改变，旧结果未写入";
const STORAGE: &str = "STORAGE_ERROR: AI直聘状态保存失败，本次事务未提交";
fn uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}
fn digest<T: Serialize>(value: &T) -> AppResult<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(err)?)
    ))
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn prefix(instance: &str) -> AppResult<String> {
    if !uuid(instance) {
        return Err("INVALID_ARGUMENT: AI直聘实例标识无效".into());
    }
    Ok(format!("job.row/{instance}/"))
}
fn control_key(instance: &str) -> AppResult<String> {
    prefix(instance)?;
    Ok(format!("job.control/{instance}"))
}
fn row_key(instance: &str, id: &str) -> AppResult<String> {
    job_protocol::job_id(id)?;
    Ok(format!("{}{id}", prefix(instance)?))
}
fn read<T: DeserializeOwned>(db: &Connection, key: &str, max: usize) -> AppResult<Option<T>> {
    let raw: Option<String> = db
        .query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|_| DATA_ERROR)?;
    raw.map(|s| {
        if s.len() > max {
            Err(DATA_ERROR.into())
        } else {
            serde_json::from_str(&s).map_err(|_| DATA_ERROR.into())
        }
    })
    .transpose()
}
fn write<T: Serialize>(db: &Connection, key: &str, value: &T) -> AppResult<()> {
    let raw = serde_json::to_string(value).map_err(|_| STORAGE)?;
    if raw.len() > 128 * 1024 {
        return Err("CAPACITY_EXCEEDED: AI直聘单项数据超过容量，未覆盖".into());
    }
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,raw]).map_err(|_|STORAGE)?;
    Ok(())
}
fn next(value: u64) -> AppResult<u64> {
    value
        .checked_add(1)
        .filter(|n| *n <= MAX_SAFE)
        .ok_or_else(|| "LIMIT: AI直聘版本达到上限".into())
}
fn context(search: &Search, credential_revision: u64) -> AppResult<String> {
    digest(&(search, credential_revision))
}
fn input_hash(job: &Job) -> AppResult<String> {
    digest(&job.ai_value())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Config {
    pub search: Search,
    pub background_enabled: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            search: Search::default(),
            background_enabled: false,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    WaitingPage,
    ReadyToRate,
    WaitingModel,
    Completed,
    Stopped,
    Interrupted,
    Failed,
}
impl Phase {
    pub(crate) fn active(&self) -> bool {
        matches!(
            self,
            Self::WaitingPage | Self::ReadyToRate | Self::WaitingModel
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    id: String,
    input_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Batch {
    id: String,
    jobs: Vec<Identity>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Run {
    pub id: String,
    pub target: Option<String>,
    pub generation: u64,
    pub config_revision: u64,
    pub credential_revision: u64,
    pub phase: Phase,
    pub page: u32,
    pub has_next: bool,
    pub total: u64,
    pub seen: u64,
    pub rated: u64,
    pub updated_at: u64,
    context: String,
    queue: Vec<Identity>,
    pending: Option<Batch>,
}
impl Run {
    pub(crate) fn uncertain_request(&self) -> bool {
        self.pending.is_some()
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Control {
    version: u8,
    pub revision: u64,
    pub generation: u64,
    pub config: Config,
    pub run: Option<Run>,
    receipts: Vec<Receipt>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            generation: 0,
            config: Config::default(),
            run: None,
            receipts: Vec::new(),
        }
    }
}

pub(crate) fn load(db: &Connection, instance: &str) -> AppResult<Control> {
    let c: Control = read(db, &control_key(instance)?, 128 * 1024)?.unwrap_or_default();
    let mut receipts = HashSet::new();
    if c.version != 1
        || c.revision > MAX_SAFE
        || c.generation > MAX_SAFE
        || c.config.search.validate(false).is_err()
        || c.revision == 0 && c.config != Config::default()
        || c.receipts.len() > 64
        || c.receipts.len() as u64 > c.revision
        || c.receipts
            .iter()
            .any(|r| !uuid(&r.id) || !hash(&r.fingerprint) || !receipts.insert(&r.id))
    {
        return Err(DATA_ERROR.into());
    }
    if let Some(r) = &c.run {
        let mut ids = HashSet::new();
        if !uuid(&r.id)
            || r.target.as_ref().is_some_and(|id| {
                job_protocol::job_id(id).is_err() || r.page != 1 || r.has_next || r.total != 1
            })
            || r.generation == 0
            || r.generation > c.generation
            || r.config_revision > c.revision
            || r.credential_revision == 0
            || r.credential_revision > MAX_SAFE
            || !(1..=10000).contains(&r.page)
            || r.total > MAX_SAFE
            || r.seen > MAX_SAFE
            || r.rated > r.seen
            || !hash(&r.context)
            || r.queue.len() > 100
            || r.queue.iter().any(|j| {
                job_protocol::job_id(&j.id).is_err() || !hash(&j.input_hash) || !ids.insert(&j.id)
            })
            || r.phase.active()
                && (r.generation != c.generation
                    || r.config_revision != c.revision
                    || r.context != context(&c.config.search, r.credential_revision)?
                    || c.config.search.validate(true).is_err())
            || r.phase == Phase::WaitingModel && r.pending.is_none()
            || matches!(r.phase, Phase::WaitingPage | Phase::Completed)
                && (!r.queue.is_empty() || r.pending.is_some())
            || r.phase == Phase::ReadyToRate && (r.queue.is_empty() || r.pending.is_some())
            || r.pending.as_ref().is_some_and(|b| {
                !uuid(&b.id)
                    || b.jobs.is_empty()
                    || b.jobs.len() > r.queue.len()
                    || r.queue[..b.jobs.len()] != b.jobs
            })
        {
            return Err(DATA_ERROR.into());
        }
    }
    Ok(c)
}
fn save(db: &Connection, instance: &str, c: &Control) -> AppResult<()> {
    write(db, &control_key(instance)?, c)
}

/// 返回最新快照和是否重放；重放不再次应用旧设置，也不恢复旧任务。
pub(crate) fn configure(
    db: &mut Connection,
    instance: &str,
    id: &str,
    expected: u64,
    config: Config,
    now: u64,
) -> AppResult<(Control, bool)> {
    if !uuid(id) || expected > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 配置请求标识或版本无效".into());
    }
    config.search.validate(false)?;
    let fingerprint = digest(&(expected, &config))?;
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    if let Some(r) = c.receipts.iter().find(|r| r.id == id) {
        if r.fingerprint != fingerprint {
            return Err("REQUEST_CONFLICT: 同一请求不能保存不同求职条件".into());
        }
        return Ok((c, true));
    }
    if c.revision != expected {
        return Err("REVISION_CONFLICT: 求职条件已有新版本，请保留草稿后重新读取".into());
    }
    c.revision = next(c.revision)?;
    c.config = config;
    if let Some(r) = c.run.as_mut().filter(|r| r.phase.active()) {
        r.phase = Phase::Stopped;
        r.updated_at = now;
    }
    c.receipts.push(Receipt {
        id: id.into(),
        fingerprint,
    });
    if c.receipts.len() > 64 {
        c.receipts.remove(0);
    }
    save(&tx, instance, &c)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok((c, false))
}
/// 只有fresh为true的调用才能启动页面；丢失响应后的同请求重试只读已有预留。
pub(crate) fn start(
    db: &mut Connection,
    instance: &str,
    id: &str,
    expected_revision: u64,
    expected_generation: u64,
    credential_revision: u64,
    now: u64,
) -> AppResult<(Run, bool)> {
    begin(
        db,
        instance,
        id,
        expected_revision,
        expected_generation,
        credential_revision,
        None,
        now,
    )
}
pub(crate) fn rerate(
    db: &mut Connection,
    instance: &str,
    id: &str,
    expected_revision: u64,
    expected_generation: u64,
    credential_revision: u64,
    job_id: &str,
    now: u64,
) -> AppResult<(Run, bool)> {
    job_protocol::job_id(job_id)?;
    begin(
        db,
        instance,
        id,
        expected_revision,
        expected_generation,
        credential_revision,
        Some(job_id),
        now,
    )
}
fn begin(
    db: &mut Connection,
    instance: &str,
    id: &str,
    expected_revision: u64,
    expected_generation: u64,
    credential_revision: u64,
    target: Option<&str>,
    now: u64,
) -> AppResult<(Run, bool)> {
    if !uuid(id) || credential_revision == 0 || credential_revision > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 任务标识或模型配置版本无效".into());
    }
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    if let Some(r) = c.run.as_ref().filter(|r| r.id == id) {
        if r.config_revision != expected_revision
            || r.target.as_deref() != target
            || r.generation.checked_sub(1) != Some(expected_generation)
            || r.credential_revision != credential_revision
        {
            return Err("REQUEST_CONFLICT: 同一搜索请求不能改变条件或模型版本".into());
        }
        return Ok((r.clone(), false));
    }
    if c.revision != expected_revision || c.generation != expected_generation {
        return Err("REVISION_CONFLICT: 搜索条件或任务已有新版本".into());
    }
    if c.run.as_ref().is_some_and(|r| r.phase.active()) {
        return Err("BUSY: 请先停止当前搜索".into());
    }
    c.config.search.validate(true)?;
    let queue = match target {
        Some(id) => {
            let item = row(&tx, instance, id)?.ok_or("NOT_FOUND: 职位已不存在，请重新读取列表")?;
            vec![Identity {
                id: id.into(),
                input_hash: input_hash(&item.job)?,
            }]
        }
        None => Vec::new(),
    };
    c.generation = next(c.generation)?;
    let run = Run {
        id: id.into(),
        target: target.map(str::to_owned),
        generation: c.generation,
        config_revision: c.revision,
        credential_revision,
        phase: if target.is_some() {
            Phase::ReadyToRate
        } else {
            Phase::WaitingPage
        },
        page: 1,
        has_next: false,
        total: u64::from(target.is_some()),
        seen: u64::from(target.is_some()),
        rated: 0,
        updated_at: now,
        context: context(&c.config.search, credential_revision)?,
        queue,
        pending: None,
    };
    c.run = Some(run.clone());
    save(&tx, instance, &c)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok((run, true))
}
fn current(c: &Control, id: &str, credential_revision: u64) -> AppResult<Run> {
    c.run
        .as_ref()
        .filter(|r| {
            r.id == id
                && r.phase.active()
                && r.config_revision == c.revision
                && r.credential_revision == credential_revision
        })
        .cloned()
        .ok_or_else(|| STALE.into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Scored {
    context: String,
    input_hash: String,
    pub rating: Rating,
    pub at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Row {
    version: u8,
    pub job: Job,
    pub search_keyword: String,
    pub archived_at: u64,
    pub updated_at: u64,
    pub scored: Option<Scored>,
}
impl Row {
    pub(crate) fn current_rating(
        &self,
        search: &Search,
        credential_revision: u64,
    ) -> AppResult<Option<&Rating>> {
        let ctx = context(search, credential_revision)?;
        let input = input_hash(&self.job)?;
        Ok(self
            .scored
            .as_ref()
            .filter(|s| s.context == ctx && s.input_hash == input)
            .map(|s| &s.rating))
    }
}
pub(crate) fn row(db: &Connection, instance: &str, id: &str) -> AppResult<Option<Row>> {
    let r: Option<Row> = read(db, &row_key(instance, id)?, 128 * 1024)?;
    if let Some(r) = &r {
        if r.version != 1
            || r.job.encrypt_job_id != id
            || r.search_keyword.chars().count() > 120
            || Job::from_page(serde_json::to_value(&r.job).map_err(err)?).is_err()
            || r.scored.as_ref().is_some_and(|s| {
                !hash(&s.context)
                    || !hash(&s.input_hash)
                    || job_protocol::ratings(&json!([s.rating]).to_string(), &[id.into()]).is_err()
            })
        {
            return Err(DATA_ERROR.into());
        }
    }
    Ok(r)
}
fn row_limits(db: &Connection, instance: &str) -> AppResult<()> {
    let pattern = format!("{}%", prefix(instance)?);
    let (count,bytes):(u64,u64)=db.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(value AS BLOB))),0) FROM app_settings WHERE key LIKE ?1",[pattern],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|DATA_ERROR)?;
    if count > MAX_JOBS as u64 || bytes > 32 * 1024 * 1024 {
        return Err("CAPACITY_EXCEEDED: 职位库已达5000项或32MiB上限，请先整理已有职位".into());
    }
    Ok(())
}
fn advance(r: &mut Run) -> AppResult<()> {
    if !r.queue.is_empty() {
        r.phase = Phase::ReadyToRate;
    } else if r.has_next {
        r.page = r
            .page
            .checked_add(1)
            .filter(|p| *p <= 10000)
            .ok_or("LIMIT: 搜索页数已达上限")?;
        r.phase = Phase::WaitingPage;
    } else {
        r.phase = Phase::Completed;
    }
    Ok(())
}
/// 单页岗位与游标同一事务落库。采集回调不能跨任务、条件或页码写入。
pub(crate) fn capture(
    db: &mut Connection,
    instance: &str,
    id: &str,
    credential_revision: u64,
    value: Value,
    now: u64,
) -> AppResult<Run> {
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    let mut run = current(&c, id, credential_revision)?;
    if run.phase != Phase::WaitingPage {
        return Err(STALE.into());
    }
    let page = job_protocol::parse_page(value, run.page)?;
    run.total = page.total;
    run.has_next = page.has_next;
    run.seen = run
        .seen
        .checked_add(page.jobs.len() as u64)
        .filter(|v| *v <= MAX_SAFE)
        .ok_or("LIMIT: 岗位计数达到上限")?;
    for job in page.jobs {
        let previous = row(&tx, instance, &job.encrypt_job_id)?;
        let value = Row {
            version: 1,
            archived_at: previous.as_ref().map_or(now, |r| r.archived_at),
            updated_at: now,
            search_keyword: c.config.search.search_keyword.clone(),
            scored: previous.and_then(|r| r.scored),
            job,
        };
        if value
            .current_rating(&c.config.search, credential_revision)?
            .is_none()
        {
            run.queue.push(Identity {
                id: value.job.encrypt_job_id.clone(),
                input_hash: input_hash(&value.job)?,
            });
        }
        write(&tx, &row_key(instance, &value.job.encrypt_job_id)?, &value)?;
    }
    row_limits(&tx, instance)?;
    run.updated_at = now;
    advance(&mut run)?;
    c.run = Some(run.clone());
    save(&tx, instance, &c)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(run)
}

pub(crate) struct Prepared {
    pub token: String,
    pub body: Value,
    pub jobs: Vec<String>,
}
/// 先提交预留才返回请求正文。等待模型时不会再次返回正文，重启亦不会自动补发。
pub(crate) fn reserve(
    db: &mut Connection,
    instance: &str,
    id: &str,
    credential_revision: u64,
    model: &str,
    now: u64,
) -> AppResult<Prepared> {
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    let mut r = current(&c, id, credential_revision)?;
    if r.phase != Phase::ReadyToRate {
        return Err("BUSY: 当前批次未就绪或已经预留模型请求".into());
    }
    let mut jobs = Vec::new();
    let mut body = None;
    for identity in &r.queue {
        let item = row(&tx, instance, &identity.id)?.ok_or(DATA_ERROR)?;
        if input_hash(&item.job)? != identity.input_hash {
            return Err(STALE.into());
        }
        jobs.push(item.job);
        match job_protocol::rating_body(model, &c.config.search, &jobs) {
            Ok(value) => body = Some(value),
            Err(e) if e.starts_with("CAPACITY_EXCEEDED:") && jobs.len() > 1 => {
                jobs.pop();
                break;
            }
            Err(e) => return Err(e),
        }
    }
    let body = body.ok_or("DATA_UNAVAILABLE: 当前没有待评分职位")?;
    let token = uuid::Uuid::new_v4().to_string();
    let ids = jobs.iter().map(|j| j.encrypt_job_id.clone()).collect();
    r.pending = Some(Batch {
        id: token.clone(),
        jobs: r.queue[..jobs.len()].to_vec(),
    });
    r.phase = Phase::WaitingModel;
    r.updated_at = now;
    c.run = Some(r);
    save(&tx, instance, &c)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(Prepared {
        token,
        body,
        jobs: ids,
    })
}
/// 完整校验后整批提交；任一异常或保存失败都不留下部分评分。
pub(crate) fn complete(
    db: &mut Connection,
    instance: &str,
    id: &str,
    credential_revision: u64,
    token: &str,
    output: &str,
    now: u64,
) -> AppResult<Run> {
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    let mut r = current(&c, id, credential_revision)?;
    let batch = r
        .pending
        .clone()
        .filter(|b| r.phase == Phase::WaitingModel && b.id == token)
        .ok_or(STALE)?;
    let ids = batch.jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
    let ratings = job_protocol::ratings(output, &ids)?;
    for rating in ratings {
        let identity = batch
            .jobs
            .iter()
            .find(|j| j.id == rating.encrypt_job_id)
            .ok_or(STALE)?;
        let mut item = row(&tx, instance, &identity.id)?.ok_or(DATA_ERROR)?;
        if input_hash(&item.job)? != identity.input_hash {
            return Err(STALE.into());
        }
        item.scored = Some(Scored {
            context: r.context.clone(),
            input_hash: identity.input_hash.clone(),
            rating,
            at: now,
        });
        write(&tx, &row_key(instance, &identity.id)?, &item)?;
    }
    row_limits(&tx, instance)?;
    r.queue.drain(..batch.jobs.len());
    r.rated = r
        .rated
        .checked_add(batch.jobs.len() as u64)
        .filter(|n| *n <= r.seen)
        .ok_or(DATA_ERROR)?;
    r.pending = None;
    r.updated_at = now;
    advance(&mut r)?;
    c.run = Some(r.clone());
    save(&tx, instance, &c)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(r)
}
pub(crate) fn stop(
    db: &mut Connection,
    instance: &str,
    id: &str,
    phase: Phase,
    now: u64,
) -> AppResult<Control> {
    if !matches!(phase, Phase::Stopped | Phase::Interrupted | Phase::Failed) {
        return Err("INVALID_ARGUMENT: 任务结束状态无效".into());
    }
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    let r = c.run.as_mut().filter(|r| r.id == id).ok_or(STALE)?;
    if r.phase.active() {
        r.phase = phase;
        r.updated_at = now;
        save(&tx, instance, &c)?;
        tx.commit().map_err(|_| STORAGE)?;
    }
    Ok(c)
}
pub(crate) fn recover(db: &mut Connection, instance: &str, now: u64) -> AppResult<Control> {
    let c = load(db, instance)?;
    if let Some(r) = c.run.as_ref().filter(|r| r.phase.active()) {
        return stop(db, instance, &r.id, Phase::Interrupted, now);
    }
    Ok(c)
}
pub(crate) fn clear_jobs(
    db: &mut Connection,
    instance: &str,
    expected_generation: u64,
) -> AppResult<Control> {
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut c = load(&tx, instance)?;
    if c.generation != expected_generation {
        return Err("REVISION_CONFLICT: 搜索任务已有新版本".into());
    }
    if c.run.as_ref().is_some_and(|r| r.phase.active()) {
        return Err("BUSY: 请先停止搜索再清空职位".into());
    }
    // 仅删除当前实例固定前缀的数据库记录，不操作文件和其他实例。
    tx.execute(
        "DELETE FROM app_settings WHERE key LIKE ?1",
        [format!("{}%", prefix(instance)?)],
    )
    .map_err(|_| STORAGE)?;
    c.generation = next(c.generation)?;
    c.run = None;
    save(&tx, instance, &c)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(c)
}

pub(crate) fn snapshot(c: &Control) -> Value {
    json!({"revision":c.revision,"generation":c.generation,"config":c.config,"run":c.run.as_ref().map(|r|json!({
        "id":r.id,"target":r.target,"phase":r.phase,"page":r.page,"total":r.total,"seen":r.seen,"rated":r.rated,"updatedAt":r.updated_at,"uncertainRequest":r.uncertain_request()
    }))})
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute(
        "DELETE FROM app_settings WHERE key=?1 OR key LIKE ?2",
        params![control_key(instance)?, format!("{}%", prefix(instance)?)],
    )
    .map_err(|_| "STORAGE_ERROR: AI直聘本地数据清理失败")?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Query {
    pub search_keyword: String,
    pub score: Option<u8>,
    pub page: u32,
    pub page_size: u32,
}
fn frequency(values: BTreeMap<String, usize>) -> Value {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    json!(values
        .into_iter()
        .map(|(name, count)| json!({"name":name,"count":count}))
        .collect::<Vec<_>>())
}
pub(crate) fn query(
    db: &Connection,
    instance: &str,
    credential_revision: u64,
    query: Query,
) -> AppResult<Value> {
    if credential_revision > MAX_SAFE
        || query.search_keyword.chars().count() > 120
        || query.page == 0
        || query.page > 10000
        || !(1..=100).contains(&query.page_size)
        || query.score.is_some_and(|s| s > 5)
    {
        return Err("INVALID_ARGUMENT: 职位筛选、页码或评分无效".into());
    }
    let c = load(db, instance)?;
    row_limits(db, instance)?;
    let keys = {
        let mut stmt = db
            .prepare("SELECT key FROM app_settings WHERE key LIKE ?1 ORDER BY key")
            .map_err(|_| DATA_ERROR)?;
        let result = stmt
            .query_map([format!("{}%", prefix(instance)?)], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| DATA_ERROR)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| DATA_ERROR)?;
        result
    };
    let mut rows = Vec::new();
    let mut keywords = BTreeMap::<String, usize>::new();
    let mut skills = BTreeMap::<String, usize>::new();
    let mut industries = BTreeMap::<String, usize>::new();
    let mut unscored = 0;
    let p = prefix(instance)?;
    for key in keys {
        let id = key.strip_prefix(&p).ok_or(DATA_ERROR)?;
        let item = row(db, instance, id)?.ok_or(DATA_ERROR)?;
        *keywords.entry(item.search_keyword.clone()).or_default() += 1;
        if !query.search_keyword.is_empty() && query.search_keyword != item.search_keyword {
            continue;
        }
        let analysis = item.current_rating(&c.config.search, credential_revision)?;
        if analysis.is_none() {
            unscored += 1;
        }
        let score = analysis.map_or(0, |r| r.score);
        if !query.score.is_none_or(|s| s == score) {
            continue;
        }
        let tags = item
            .job
            .skills
            .iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<HashSet<_>>();
        for tag in tags {
            *skills.entry(tag.clone()).or_default() += 1;
        }
        if !item.job.brand_industry.trim().is_empty() {
            *industries
                .entry(item.job.brand_industry.clone())
                .or_default() += 1;
        }
        rows.push(item);
    }
    rows.sort_by(|a, b| {
        b.archived_at
            .cmp(&a.archived_at)
            .then(a.job.encrypt_job_id.cmp(&b.job.encrypt_job_id))
    });
    let total = rows.len();
    let offset = (query.page as usize - 1) * query.page_size as usize;
    let items = rows
        .into_iter()
        .skip(offset)
        .take(query.page_size as usize)
        .map(|item| -> AppResult<Value> {
            let current = item.current_rating(&c.config.search, credential_revision)?;
            let mut v = serde_json::to_value(&item.job).map_err(err)?;
            v["searchKeyword"] = json!(item.search_keyword);
            v["archiveTime"] = json!(item.archived_at);
            v["updatedAt"] = json!(item.updated_at);
            v["analysis"] = json!(current);
            v["analysisStale"] = json!(current.is_none() && item.scored.is_some());
            v["previousAnalysis"] = json!(item
                .scored
                .as_ref()
                .filter(|_| current.is_none())
                .map(|s| &s.rating));
            v["analysisAt"] = json!(item.scored.as_ref().map(|s| s.at));
            Ok(v)
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut v = snapshot(&c);
    v["jobs"] = json!(items);
    v["total"] = json!(total);
    v["page"] = json!(query.page);
    v["hasNext"] = json!(offset.saturating_add(query.page_size as usize) < total);
    v["unscored"] = json!(unscored);
    v["keywords"] = frequency(keywords);
    v["skills"] = frequency(skills);
    v["industries"] = frequency(industries);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn setup() -> (Connection, String) {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);")
            .unwrap();
        let instance = id();
        let config = Config {
            search: Search {
                search_keyword: "前端".into(),
                skills: "Vue".into(),
                ..Search::default()
            },
            ..Config::default()
        };
        configure(&mut db, &instance, &id(), 0, config, 1).unwrap();
        (db, instance)
    }
    fn begin(db: &mut Connection, i: &str, crev: u64) -> Run {
        let c = load(db, i).unwrap();
        start(db, i, &id(), c.revision, c.generation, crev, 2)
            .unwrap()
            .0
    }
    fn page(ids: &[&str]) -> Value {
        json!({"origin":"https://www.zhipin.com","path":"/web/geek/job","loggedIn":true,"page":1,"pageSize":30,"total":ids.len(),"jobs":ids.iter().map(|id|json!({"encryptJobId":id,"jobName":"开发工程师","skills":["Vue"],"salaryDesc":"15-25K"})).collect::<Vec<_>>()})
    }
    fn output(ids: &[String]) -> String {
        Value::Array(ids.iter().map(|id|json!({"encryptJobId":id,"salaryMatch":true,"degreeMatch":true,"comment":"技能和工作内容相符","score":4})).collect()).to_string()
    }
    fn captured(db: &mut Connection, i: &str, ids: &[&str]) -> Run {
        let r = begin(db, i, 1);
        capture(db, i, &r.id, 1, page(ids), 3).unwrap()
    }
    fn control_bytes(db: &Connection, i: &str) -> String {
        db.query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [control_key(i).unwrap()],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn defaults_disable_background_and_configure_uses_conflict_and_replay_protection() {
        let (mut db, i) = setup();
        let c = load(&db, &i).unwrap();
        assert!(!c.config.background_enabled);
        assert_eq!(c.revision, 1);
        let request = id();
        let mut config = c.config;
        config.search.search_keyword = "Rust".into();
        let (saved, replayed) = configure(&mut db, &i, &request, 1, config.clone(), 2).unwrap();
        assert!(!replayed);
        assert_eq!(saved.revision, 2);
        let r = begin(&mut db, &i, 1);
        let (latest, replayed) = configure(&mut db, &i, &request, 1, config.clone(), 3).unwrap();
        assert!(replayed);
        assert_eq!(latest.run.unwrap().id, r.id);
        config.search.skills = "其他技能".into();
        assert!(configure(&mut db, &i, &request, 1, config.clone(), 4).is_err());
        assert!(configure(&mut db, &i, &id(), 1, config, 4).is_err());
        assert_eq!(load(&db, &i).unwrap().revision, 2);
        assert!(prefix("../x").is_err());
    }
    #[test]
    fn start_reservation_is_idempotent_and_never_reopens_from_an_old_retry() {
        let (mut db, i) = setup();
        let request = id();
        let (r, fresh) = start(&mut db, &i, &request, 1, 0, 1, 2).unwrap();
        assert!(fresh);
        let (same, fresh) = start(&mut db, &i, &request, 1, 0, 1, 3).unwrap();
        assert!(!fresh);
        assert_eq!(same.id, r.id);
        assert!(start(&mut db, &i, &request, 1, 0, 2, 3).is_err());
        assert!(start(&mut db, &i, &id(), 1, 1, 1, 3).is_err());
        stop(&mut db, &i, &request, Phase::Stopped, 4).unwrap();
        let (same, fresh) = start(&mut db, &i, &request, 1, 0, 1, 5).unwrap();
        assert!(!fresh);
        assert_eq!(same.phase, Phase::Stopped);
        let newer = begin(&mut db, &i, 1);
        assert_ne!(newer.id, request);
        assert!(start(&mut db, &i, &request, 1, 0, 1, 6).is_err());
    }
    #[test]
    fn captured_pages_and_model_results_require_the_current_task_and_credential_version() {
        let (mut db, i) = setup();
        let r = begin(&mut db, &i, 1);
        assert!(capture(&mut db, &i, &r.id, 2, page(&["id1"]), 3).is_err());
        assert!(capture(&mut db, &i, &id(), 1, page(&["id1"]), 3).is_err());
        assert!(row(&db, &i, "id1").unwrap().is_none());
        let r = capture(&mut db, &i, &r.id, 1, page(&["id1"]), 3).unwrap();
        assert_eq!(r.phase, Phase::ReadyToRate);
        assert!(capture(&mut db, &i, &r.id, 1, page(&["id1"]), 4).is_err());
        let p = reserve(&mut db, &i, &r.id, 1, "my-model", 4).unwrap();
        assert!(complete(&mut db, &i, &r.id, 2, &p.token, &output(&p.jobs), 5).is_err());
        assert!(complete(&mut db, &i, &r.id, 1, &id(), &output(&p.jobs), 5).is_err());
        let r = complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).unwrap();
        assert_eq!(r.phase, Phase::Completed);
        assert_eq!((r.seen, r.rated), (1, 1));
        assert!(!r.uncertain_request());
        assert!(complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 6).is_err());
    }
    #[test]
    fn paid_request_is_returned_only_after_a_successful_durable_reservation() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1"]);
        let before = control_bytes(&db, &i);
        db.execute_batch("CREATE TEMP TRIGGER fail_control BEFORE UPDATE ON app_settings WHEN NEW.key LIKE 'job.control/%' BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        assert!(reserve(&mut db, &i, &r.id, 1, "my-model", 4).is_err());
        assert_eq!(control_bytes(&db, &i), before);
        db.execute_batch("DROP TRIGGER fail_control;").unwrap();
        let p = reserve(&mut db, &i, &r.id, 1, "my-model", 5).unwrap();
        assert_eq!(p.body["model"], "my-model");
        assert!(reserve(&mut db, &i, &r.id, 1, "my-model", 6).is_err());
        assert_eq!(
            load(&db, &i).unwrap().run.unwrap().phase,
            Phase::WaitingModel
        );
    }
    #[test]
    fn failed_capture_commit_rolls_back_all_new_jobs_and_cursor() {
        let (mut db, i) = setup();
        let r = begin(&mut db, &i, 1);
        let before = control_bytes(&db, &i);
        db.execute_batch("CREATE TEMP TRIGGER fail_control BEFORE UPDATE ON app_settings WHEN NEW.key LIKE 'job.control/%' BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        assert!(capture(&mut db, &i, &r.id, 1, page(&["id1", "id2"]), 3).is_err());
        assert!(row(&db, &i, "id1").unwrap().is_none());
        assert!(row(&db, &i, "id2").unwrap().is_none());
        assert_eq!(control_bytes(&db, &i), before);
    }
    #[test]
    fn incomplete_or_failed_score_write_never_leaves_a_partial_success() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1", "id2"]);
        let p = reserve(&mut db, &i, &r.id, 1, "my-model", 4).unwrap();
        assert!(complete(&mut db, &i, &r.id, 1, &p.token, &output(&["id1".into()]), 5).is_err());
        assert!(row(&db, &i, "id1").unwrap().unwrap().scored.is_none());
        db.execute_batch("CREATE TEMP TRIGGER fail_second BEFORE UPDATE ON app_settings WHEN NEW.key LIKE 'job.row/%/id2' BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        assert!(complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).is_err());
        for j in ["id1", "id2"] {
            assert!(row(&db, &i, j).unwrap().unwrap().scored.is_none());
        }
        assert_eq!(
            load(&db, &i).unwrap().run.unwrap().phase,
            Phase::WaitingModel
        );
        db.execute_batch("DROP TRIGGER fail_second;").unwrap();
        assert_eq!(
            complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 6)
                .unwrap()
                .phase,
            Phase::Completed
        );
    }
    #[test]
    fn stopping_or_changing_conditions_rejects_late_callbacks_and_preserves_uncertainty() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1"]);
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        let c = stop(&mut db, &i, &r.id, Phase::Stopped, 5).unwrap();
        assert!(c.run.unwrap().uncertain_request());
        assert!(complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 6).is_err());
        let next = begin(&mut db, &i, 1);
        let c = load(&db, &i).unwrap();
        let mut config = c.config;
        config.search.skills = "Rust".into();
        configure(&mut db, &i, &id(), c.revision, config, 7).unwrap();
        assert_eq!(load(&db, &i).unwrap().run.unwrap().phase, Phase::Stopped);
        assert!(capture(&mut db, &i, &next.id, 1, page(&["id2"]), 8).is_err());
        assert!(row(&db, &i, "id2").unwrap().is_none());
    }
    #[test]
    fn restart_interrupts_pending_work_without_returning_a_second_request() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1"]);
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        let recovered = recover(&mut db, &i, 5).unwrap();
        let old = recovered.run.unwrap();
        assert_eq!(old.phase, Phase::Interrupted);
        assert!(old.uncertain_request());
        assert!(reserve(&mut db, &i, &r.id, 1, "m", 6).is_err());
        assert!(complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 6).is_err());
        assert_eq!(recover(&mut db, &i, 7).unwrap().run.unwrap().updated_at, 5);
    }
    #[test]
    fn cache_is_bound_to_conditions_model_revision_and_complete_job_input() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1"]);
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).unwrap();
        let c = load(&db, &i).unwrap();
        let saved = row(&db, &i, "id1").unwrap().unwrap();
        assert!(saved.current_rating(&c.config.search, 1).unwrap().is_some());
        assert!(saved.current_rating(&c.config.search, 2).unwrap().is_none());
        let mut changed = c.config.search.clone();
        changed.skills = "Rust".into();
        assert!(saved.current_rating(&changed, 1).unwrap().is_none());
        let r = begin(&mut db, &i, 1);
        let same = capture(&mut db, &i, &r.id, 1, page(&["id1"]), 6).unwrap();
        assert_eq!(same.phase, Phase::Completed);
        assert_eq!(same.rated, 0);
        let r = begin(&mut db, &i, 1);
        let mut v = page(&["id1"]);
        v["jobs"][0]["salaryDesc"] = json!("10-15K");
        let fresh = capture(&mut db, &i, &r.id, 1, v, 7).unwrap();
        assert_eq!(fresh.phase, Phase::ReadyToRate);
        let updated = row(&db, &i, "id1").unwrap().unwrap();
        assert!(updated.scored.is_some());
        assert!(updated
            .current_rating(&c.config.search, 1)
            .unwrap()
            .is_none());
        assert_eq!(updated.archived_at, 3);
    }
    #[test]
    fn input_limit_splits_only_between_whole_jobs_and_scores_every_job_once() {
        let (mut db, i) = setup();
        let r = begin(&mut db, &i, 1);
        let mut value = page(&["id1", "id2", "id3", "id4", "id5", "id6"]);
        for j in value["jobs"].as_array_mut().unwrap() {
            j["skills"] = json!(vec!["字".repeat(300); 20]);
        }
        let mut r = capture(&mut db, &i, &r.id, 1, value, 3).unwrap();
        let mut seen = HashSet::new();
        let mut batches = 0;
        while r.phase == Phase::ReadyToRate {
            let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
            assert!(serde_json::to_vec(&p.body).unwrap().len() <= 64 * 1024);
            assert!(p.jobs.len() < 6);
            for j in &p.jobs {
                assert!(seen.insert(j.clone()));
            }
            batches += 1;
            r = complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).unwrap();
        }
        assert!(batches > 1);
        assert_eq!(seen.len(), 6);
        assert_eq!(r.rated, 6);
        assert_eq!(r.phase, Phase::Completed);
    }
    #[test]
    fn page_cursor_advances_only_after_all_scores_are_committed() {
        let (mut db, i) = setup();
        let r = begin(&mut db, &i, 1);
        let mut value = page(&["id1"]);
        value["total"] = json!(2);
        value["pageSize"] = json!(1);
        let r = capture(&mut db, &i, &r.id, 1, value, 3).unwrap();
        assert_eq!(r.page, 1);
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        let r = complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).unwrap();
        assert_eq!((r.page, r.phase), (2, Phase::WaitingPage));
        assert!(capture(&mut db, &i, &r.id, 1, page(&["id2"]), 6).is_err());
        let mut value = page(&["id2"]);
        value["page"] = json!(2);
        value["total"] = json!(2);
        value["pageSize"] = json!(1);
        assert_eq!(capture(&mut db, &i, &r.id, 1, value, 6).unwrap().page, 2);
    }
    #[test]
    fn corrupt_state_is_preserved_and_never_silently_reset() {
        let (mut db, i) = setup();
        let key = control_key(&i).unwrap();
        let broken = "{\"version\":999,\"secret\":\"不能输出或覆盖\"}";
        db.execute(
            "UPDATE app_settings SET value=?2 WHERE key=?1",
            params![key, broken],
        )
        .unwrap();
        assert!(load(&db, &i).is_err());
        assert!(configure(&mut db, &i, &id(), 0, Config::default(), 2).is_err());
        assert_eq!(control_bytes(&db, &i), broken);
    }
    #[test]
    fn clearing_jobs_is_instance_scoped_and_refuses_an_active_run() {
        let (mut db, i) = setup();
        let other = id();
        let config = load(&db, &i).unwrap().config;
        configure(&mut db, &other, &id(), 0, config, 1).unwrap();
        let r = captured(&mut db, &i, &["a_b"]);
        let second = captured(&mut db, &other, &["a_b"]);
        assert!(clear_jobs(&mut db, &i, r.generation).is_err());
        stop(&mut db, &i, &r.id, Phase::Stopped, 4).unwrap();
        let c = clear_jobs(&mut db, &i, r.generation).unwrap();
        assert!(row(&db, &i, "a_b").unwrap().is_none());
        assert!(row(&db, &other, "a_b").unwrap().is_some());
        assert_eq!(load(&db, &other).unwrap().run.unwrap().id, second.id);
        assert_eq!(c.config.search.search_keyword, "前端");
        assert!(clear_jobs(&mut db, &i, r.generation).is_err());
    }
    #[test]
    fn explicit_rerating_reuses_saved_job_and_cannot_replay_as_a_search() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1"]);
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).unwrap();
        let c = load(&db, &i).unwrap();
        let request = id();
        let (r, fresh) =
            rerate(&mut db, &i, &request, c.revision, c.generation, 1, "id1", 6).unwrap();
        assert!(fresh);
        assert_eq!(r.target.as_deref(), Some("id1"));
        assert_eq!(r.phase, Phase::ReadyToRate);
        assert!(
            !rerate(&mut db, &i, &request, c.revision, c.generation, 1, "id1", 7)
                .unwrap()
                .1
        );
        assert!(start(&mut db, &i, &request, c.revision, c.generation, 1, 7).is_err());
        assert!(rerate(&mut db, &i, &request, c.revision, c.generation, 1, "id2", 7).is_err());
        assert!(capture(&mut db, &i, &request, 1, page(&["id2"]), 7).is_err());
        let p = reserve(&mut db, &i, &request, 1, "m", 7).unwrap();
        assert_eq!(p.jobs, ["id1"]);
        let r = complete(&mut db, &i, &request, 1, &p.token, &output(&p.jobs), 8).unwrap();
        assert_eq!(r.phase, Phase::Completed);
        assert_eq!(r.rated, 1);
    }
    fn q(score: Option<u8>, page: u32) -> Query {
        Query {
            search_keyword: String::new(),
            score,
            page,
            page_size: 1,
        }
    }
    #[test]
    fn query_filters_pages_and_exposes_old_scores_without_treating_them_as_current() {
        let (mut db, i) = setup();
        let r = begin(&mut db, &i, 1);
        let mut value = page(&["id1", "id2"]);
        for j in value["jobs"].as_array_mut().unwrap() {
            j["skills"] = json!(["Vue", "Vue", "Rust"]);
            j["brandIndustry"] = json!("互联网");
        }
        capture(&mut db, &i, &r.id, 1, value, 3).unwrap();
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).unwrap();
        let first = query(&db, &i, 1, q(Some(4), 1)).unwrap();
        assert_eq!(first["total"], 2);
        assert_eq!(first["jobs"][0]["encryptJobId"], "id1");
        assert_eq!(first["hasNext"], true);
        assert_eq!(first["unscored"], 0);
        assert_eq!(
            first["skills"],
            json!([{"name":"Rust","count":2},{"name":"Vue","count":2}])
        );
        assert_eq!(first["industries"], json!([{"name":"互联网","count":2}]));
        let empty = query(&db, &i, 1, q(Some(3), 1)).unwrap();
        assert_eq!(empty["skills"], json!([]));
        assert_eq!(empty["industries"], json!([]));
        let second = query(&db, &i, 1, q(None, 2)).unwrap();
        assert_eq!(second["jobs"][0]["encryptJobId"], "id2");
        assert_eq!(second["hasNext"], false);
        let stale = query(&db, &i, 2, q(Some(0), 1)).unwrap();
        assert_eq!(stale["total"], 2);
        assert_eq!(stale["unscored"], 2);
        assert!(stale["jobs"][0]["analysis"].is_null());
        assert_eq!(stale["jobs"][0]["analysisStale"], true);
        assert_eq!(stale["jobs"][0]["previousAnalysis"]["score"], 4);
        let encoded = stale.to_string();
        for internal in ["receipts", "fingerprint", "pending", "inputHash", "context"] {
            assert!(!encoded.contains(internal));
        }
        let mut filter = q(None, 1);
        filter.search_keyword = "不存在的关键字".into();
        let empty = query(&db, &i, 1, filter).unwrap();
        assert_eq!(empty["total"], 0);
        assert_eq!(empty["keywords"], json!([{"name":"前端","count":2}]));
        assert!(query(&db, &i, 1, q(Some(6), 1)).is_err());
        assert!(query(&db, &i, 1, q(None, 0)).is_err());
    }
    #[test]
    fn a_changed_saved_job_cannot_receive_a_score_for_the_old_input() {
        let (mut db, i) = setup();
        let r = captured(&mut db, &i, &["id1", "id2"]);
        let p = reserve(&mut db, &i, &r.id, 1, "m", 4).unwrap();
        let mut item = row(&db, &i, "id2").unwrap().unwrap();
        item.job.salary_desc = "5-10K".into();
        write(&db, &row_key(&i, "id2").unwrap(), &item).unwrap();
        assert!(complete(&mut db, &i, &r.id, 1, &p.token, &output(&p.jobs), 5).is_err());
        assert!(row(&db, &i, "id1").unwrap().unwrap().scored.is_none());
        assert!(row(&db, &i, "id2").unwrap().unwrap().scored.is_none());
    }
    #[test]
    fn job_capacity_failure_rolls_back_the_page_and_keeps_existing_rows() {
        let (mut db, i) = setup();
        let r = begin(&mut db, &i, 1);
        let tx = db.transaction().unwrap();
        for n in 0..MAX_JOBS {
            tx.execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,'{}')",
                [row_key(&i, &format!("saved{n}")).unwrap()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        assert!(capture(&mut db, &i, &r.id, 1, page(&["new1"]), 3)
            .unwrap_err()
            .starts_with("CAPACITY_EXCEEDED:"));
        assert!(row(&db, &i, "new1").unwrap().is_none());
        assert_eq!(
            load(&db, &i).unwrap().run.unwrap().phase,
            Phase::WaitingPage
        );
        let count: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key LIKE ?1",
                [format!("{}%", prefix(&i).unwrap())],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, MAX_JOBS as u64);
    }
}
