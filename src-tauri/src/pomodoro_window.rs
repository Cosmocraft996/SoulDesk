//! 番茄钟专用窗的私有许可与展示状态；计时、场景和共享库仍由pomodoro后台负责。
use crate::{
    desktop_geometry::WorkArea,
    store::{err, AppResult},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub(crate) const PACKAGE: &str = "local.souldesk.widgetjs.pomodoro";
pub(crate) const CAPABILITY: &str = "focus.pomodoro.window";
pub(crate) const SIZE: f64 = 150.;
pub(crate) const STRIP: f64 = 6.;
const HIDE_DELAY: u64 = 2_000;
const ANIMATION_TIME: u64 = 240;
const MAX_SAFE: u64 = 9_007_199_254_740_991;
const DATA_ERROR: &str = "DATA_UNAVAILABLE: 番茄专用窗设置损坏，已保留原值";
const EXPIRED: &str = "PRESENTATION_EXPIRED: 番茄专用窗已关闭或被替换";
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|v| v.to_string() == value)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn key(instance: &str) -> AppResult<String> {
    if !uuid(instance) {
        return Err("INVALID_ARGUMENT: 番茄实例标识无效".into());
    }
    Ok(format!("pomodoro.window/{instance}"))
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Preferences {
    pub enabled: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    version: u8,
    pub revision: u64,
    pub config: Preferences,
    receipts: Vec<Receipt>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            config: Preferences::default(),
            receipts: Vec::new(),
        }
    }
}
impl Document {
    pub(crate) fn snapshot(&self) -> Value {
        json!({"revision":self.revision,"config":self.config})
    }
}
pub(crate) fn load(db: &Connection, instance: &str) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 番茄专用窗设置读取失败")?;
    let Some(raw) = raw else {
        return Ok(Document::default());
    };
    if raw.len() > 16 * 1024 {
        return Err(DATA_ERROR.into());
    }
    let value: Document = serde_json::from_str(&raw).map_err(|_| DATA_ERROR)?;
    let mut ids = std::collections::HashSet::new();
    if value.version != 1
        || value.revision > MAX_SAFE
        || value.receipts.len() > 32
        || value.receipts.len() as u64 > value.revision
        || value.revision == 0 && value.config.enabled
        || value
            .receipts
            .iter()
            .any(|r| !uuid(&r.id) || !hash(&r.fingerprint) || !ids.insert(&r.id))
    {
        return Err(DATA_ERROR.into());
    }
    Ok(value)
}
pub(crate) fn configure(
    db: &mut Connection,
    instance: &str,
    request: &str,
    expected: u64,
    config: Preferences,
) -> AppResult<(Document, bool)> {
    if !uuid(request) || expected > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 专用窗请求或版本无效".into());
    }
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(expected, &config)).map_err(err)?)
    );
    let tx = db
        .transaction()
        .map_err(|_| "STORAGE_ERROR: 专用窗设置事务失败")?;
    let mut d = load(&tx, instance)?;
    if let Some(r) = d.receipts.iter().find(|r| r.id == request) {
        if r.fingerprint != fingerprint {
            return Err("REQUEST_REUSED: 专用窗请求标识已用于其他设置".into());
        }
        return Ok((d, true));
    }
    if d.revision != expected {
        return Err("REVISION_CONFLICT: 专用窗设置已改变，请重新读取".into());
    }
    d.revision = d
        .revision
        .checked_add(1)
        .filter(|v| *v <= MAX_SAFE)
        .ok_or("LIMIT: 专用窗版本达到上限")?;
    d.config = config;
    d.receipts.push(Receipt {
        id: request.into(),
        fingerprint,
    });
    if d.receipts.len() > 32 {
        d.receipts.remove(0);
    }
    tx.execute("INSERT INTO app_settings(key,value)VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key(instance)?,serde_json::to_string(&d).map_err(err)?]).map_err(|_|"STORAGE_ERROR: 专用窗设置未保存")?;
    tx.commit()
        .map_err(|_| "STORAGE_ERROR: 专用窗设置提交失败")?;
    Ok((d, false))
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(instance)?])
        .map_err(|_| "STORAGE_ERROR: 专用窗设置清理失败")?;
    Ok(())
}

/// 统一使用物理像素；原150/6逻辑像素按所在显示器缩放，支持负坐标。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}
fn dimensions(area: WorkArea) -> AppResult<(u32, i64)> {
    if area.width == 0
        || area.height == 0
        || !area.scale_factor.is_finite()
        || !(0.25..=8.).contains(&area.scale_factor)
    {
        return Err("INVALID_GEOMETRY: 番茄工作区或缩放无效".into());
    }
    if i64::from(area.left) + i64::from(area.width) > i64::from(i32::MAX)
        || i64::from(area.top) + i64::from(area.height) > i64::from(i32::MAX)
    {
        return Err("INVALID_GEOMETRY: 工作区超出原生坐标范围".into());
    }
    Ok((
        (SIZE * area.scale_factor).round() as u32,
        (STRIP * area.scale_factor).round().max(1.) as i64,
    ))
}
pub(crate) fn project(
    at: Bounds,
    area: WorkArea,
    edge: Edge,
    expanded: bool,
    center: bool,
) -> AppResult<Bounds> {
    let (size, strip) = dimensions(area)?;
    let left = i64::from(area.left);
    let top = i64::from(area.top);
    let right = left + i64::from(area.width);
    let bottom = top + i64::from(area.height);
    let size64 = i64::from(size);
    let max_x = left + (i64::from(area.width) - size64).max(0);
    let max_y = top + (i64::from(area.height) - size64).max(0);
    let mut x = i64::from(at.x).clamp(left, max_x);
    let mut y = i64::from(at.y).clamp(top, max_y);
    if center {
        x = left + (max_x - left + 1) / 2;
    }
    match (edge, expanded) {
        (Edge::Left, true) => x = left,
        (Edge::Right, true) => x = max_x,
        (Edge::Top, true) => y = top,
        (Edge::Bottom, true) => y = max_y,
        (Edge::Left, false) => x = left - size64 + strip,
        (Edge::Right, false) => x = right - strip,
        (Edge::Top, false) => y = top - size64 + strip,
        (Edge::Bottom, false) => y = bottom - strip,
    }
    Ok(Bounds {
        x: i32::try_from(x).map_err(|_| "INVALID_GEOMETRY: 番茄横坐标溢出")?,
        y: i32::try_from(y).map_err(|_| "INVALID_GEOMETRY: 番茄纵坐标溢出")?,
        width: size,
        height: size,
    })
}
pub(crate) fn nearest(at: Bounds, area: WorkArea) -> AppResult<Edge> {
    dimensions(area)?;
    // 保留原同距离时底、顶、左、右的顺序。
    let right = i64::from(area.left) + i64::from(area.width);
    let bottom = i64::from(area.top) + i64::from(area.height);
    let values = [
        (
            Edge::Bottom,
            (i64::from(at.y) + i64::from(at.height) - bottom).abs(),
        ),
        (Edge::Top, (i64::from(at.y) - i64::from(area.top)).abs()),
        (Edge::Left, (i64::from(at.x) - i64::from(area.left)).abs()),
        (
            Edge::Right,
            (i64::from(at.x) + i64::from(at.width) - right).abs(),
        ),
    ];
    Ok(values.into_iter().min_by_key(|v| v.1).unwrap().0)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub instance: String,
    pub presentation: String,
    pub digest: String,
}
impl Owner {
    pub(crate) fn label(&self) -> String {
        format!("pomodoro-window-{}-{}", self.instance, self.presentation)
    }
    fn validate(&self) -> AppResult<()> {
        if !uuid(&self.instance) || !uuid(&self.presentation) || !hash(&self.digest) {
            return Err("INVALID_ARGUMENT: 专用窗身份无效".into());
        }
        Ok(())
    }
}
pub(crate) fn role(label: &str, instance: &str) -> bool {
    label
        .strip_prefix(&format!("pomodoro-window-{instance}-"))
        .is_some_and(uuid)
        && uuid(instance)
}
#[derive(Clone)]
struct Motion {
    from: Bounds,
    to: Bounds,
    at: u64,
}
impl Motion {
    fn frame(&self, now: u64) -> Bounds {
        let ratio = (now.saturating_sub(self.at) as f64 / ANIMATION_TIME as f64).clamp(0., 1.);
        let ratio = 1. - (1. - ratio).powi(3);
        Bounds {
            // 原Math.round的负数半像素向正方向取整，不能换成Rust的远离零取整。
            x: (f64::from(self.from.x)
                + (f64::from(self.to.x) - f64::from(self.from.x)) * ratio
                + 0.5)
                .floor() as i32,
            y: (f64::from(self.from.y)
                + (f64::from(self.to.y) - f64::from(self.from.y)) * ratio
                + 0.5)
                .floor() as i32,
            width: self.to.width,
            height: self.to.height,
        }
    }
}
#[derive(Clone)]
pub(crate) struct Surface {
    pub owner: Owner,
    pub area: WorkArea,
    pub expanded: bool,
    pub edge: Edge,
    pub manual_hidden: bool,
    pub suspended: bool,
    pub ready: bool,
    pub focus: bool,
    pub focus_request: u64,
    pub serial: u64,
    pub applied: u64,
    pub fault: Option<String>,
    confirmed_frame: Option<(Bounds, bool)>,
    full: Bounds,
    position: Bounds,
    motion: Option<Motion>,
    hide_at: Option<u64>,
    window_hover: bool,
    tray_hover: bool,
    pub dragging: bool,
    phase: String,
    token: Option<String>,
    pub data_source: Option<String>,
}
impl Surface {
    pub(crate) fn anchor(&self) -> Bounds {
        self.full
    }
    pub(crate) fn animating(&self) -> bool {
        self.motion.is_some()
    }
    pub(crate) fn frame(&self, now: u64) -> Bounds {
        self.motion.as_ref().map_or(self.position, |m| m.frame(now))
    }
    pub(crate) fn target_visible(&self) -> bool {
        !self.manual_hidden && !self.suspended && self.ready && self.fault.is_none()
    }
    pub(crate) fn snapshot(&self) -> Value {
        let settled = self.serial == self.applied
            && self.motion.is_none()
            && !self.focus
            && self.confirmed_frame == Some((self.position, self.target_visible()));
        json!({"presentationId":self.owner.presentation,"edge":self.edge,"expanded":self.expanded,"hidden":self.manual_hidden,"suspended":self.suspended,"ready":self.ready,"nativeSettled":settled,"fault":self.fault})
    }
    fn delay(&mut self, now: u64) {
        self.hide_at = if !self.ready
            || self.phase == "waiting"
            || self.window_hover
            || self.tray_hover
            || self.dragging
            || self.manual_hidden
            || self.suspended
        {
            None
        } else {
            Some(now.saturating_add(HIDE_DELAY))
        };
    }
    fn expand(&mut self, value: bool, now: u64) -> AppResult<()> {
        let target = project(self.full, self.area, self.edge, value, false)?;
        let from = self.frame(now);
        self.position = from;
        self.motion = if from == target {
            None
        } else {
            Some(Motion {
                from,
                to: target,
                at: now,
            })
        };
        self.expanded = value;
        Ok(())
    }
}
#[derive(Default)]
pub(crate) struct Book {
    entries: HashMap<String, Surface>,
    pub retired: Vec<Owner>,
    sequence: u64,
}
impl Book {
    fn next(&mut self) -> AppResult<u64> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .filter(|v| *v <= MAX_SAFE)
            .ok_or("LIMIT: 专用窗代次达到上限")?;
        Ok(self.sequence)
    }
    pub(crate) fn open(
        &mut self,
        instance: &str,
        digest: &str,
        area: WorkArea,
        focus: bool,
        now: u64,
    ) -> AppResult<Surface> {
        let owner = Owner {
            instance: instance.into(),
            presentation: uuid::Uuid::new_v4().to_string(),
            digest: digest.into(),
        };
        owner.validate()?;
        if self
            .entries
            .get(instance)
            .is_some_and(|s| s.owner.digest != digest)
        {
            self.retire(instance);
        }
        if self.retired.iter().any(|o| o.instance == instance) {
            return Err("BUSY: 旧番茄专用窗释放尚未确认".into());
        }
        let serial = self.next()?;
        if let Some(s) = self.entries.get_mut(instance) {
            s.serial = serial;
            s.manual_hidden = false;
            s.focus |= focus;
            if focus {
                s.focus_request = serial;
            }
            s.fault = None;
            s.expand(true, now)?;
            s.delay(now);
            return Ok(s.clone());
        }
        if self.entries.len() >= 32 {
            return Err("LIMIT: 番茄专用窗最多32个".into());
        }
        let full = project(
            Bounds {
                x: area.left,
                y: area.top,
                width: 1,
                height: 1,
            },
            area,
            Edge::Bottom,
            true,
            true,
        )?;
        let mut s = Surface {
            owner,
            area,
            expanded: true,
            edge: Edge::Bottom,
            manual_hidden: false,
            suspended: false,
            ready: false,
            focus,
            focus_request: if focus { serial } else { 0 },
            serial,
            applied: 0,
            fault: None,
            confirmed_frame: None,
            full,
            position: full,
            motion: None,
            hide_at: None,
            window_hover: false,
            tray_hover: false,
            dragging: false,
            phase: "stop".into(),
            token: None,
            data_source: None,
        };
        s.delay(now);
        self.entries.insert(instance.into(), s.clone());
        Ok(s)
    }
    pub(crate) fn current(&self, owner: &Owner) -> bool {
        self.entries
            .get(&owner.instance)
            .is_some_and(|s| s.owner == *owner)
    }
    pub(crate) fn get(&self, instance: &str) -> Option<Surface> {
        self.entries.get(instance).cloned()
    }
    pub(crate) fn by_label(&self, label: &str) -> Option<Surface> {
        self.entries
            .values()
            .find(|s| s.owner.label() == label)
            .cloned()
    }
    pub(crate) fn list(&self) -> Vec<Surface> {
        self.entries.values().cloned().collect()
    }
    pub(crate) fn retire(&mut self, instance: &str) {
        if let Some(s) = self.entries.remove(instance) {
            self.retired.push(s.owner);
        }
    }
    pub(crate) fn retired_confirmed(&mut self, owner: &Owner) {
        self.retired.retain(|o| o != owner);
    }
    pub(crate) fn bind(&mut self, label: &str, token: &str) -> AppResult<()> {
        if !uuid(token) {
            return Err("INVALID_ARGUMENT: 专用窗会话无效".into());
        }
        let s = self.by_label(label).ok_or(EXPIRED)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&s.owner.instance).unwrap();
        s.token = Some(token.into());
        s.data_source = None;
        s.ready = false;
        s.hide_at = None;
        s.dragging = false;
        s.window_hover = false;
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn authorized(&self, label: &str, token: &str) -> AppResult<Surface> {
        self.by_label(label)
            .filter(|s| s.token.as_deref() == Some(token))
            .ok_or_else(|| EXPIRED.into())
    }
    pub(crate) fn bind_source(
        &mut self,
        label: &str,
        token: &str,
        source: String,
    ) -> AppResult<()> {
        let s = self.authorized(label, token)?;
        self.entries.get_mut(&s.owner.instance).unwrap().data_source = Some(source);
        Ok(())
    }
    pub(crate) fn close_session(&mut self, label: &str, token: &str) -> AppResult<()> {
        let s = self.authorized(label, token)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&s.owner.instance).unwrap();
        s.token = None;
        s.ready = false;
        s.hide_at = None;
        s.dragging = false;
        s.window_hover = false;
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn ready(&mut self, label: &str, token: &str, now: u64) -> AppResult<()> {
        let s = self.authorized(label, token)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&s.owner.instance).unwrap();
        s.ready = true;
        s.serial = serial;
        s.delay(now);
        Ok(())
    }
    pub(crate) fn hover(
        &mut self,
        owner: &Owner,
        tray: bool,
        entered: bool,
        now: u64,
    ) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        if s.suspended {
            return Err(EXPIRED.into());
        }
        if tray {
            s.tray_hover = entered;
        } else {
            s.window_hover = entered;
        }
        s.serial = serial;
        if entered && !s.manual_hidden {
            s.expand(true, now)?;
        }
        s.delay(now);
        Ok(())
    }
    pub(crate) fn hide(&mut self, owner: &Owner) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        s.manual_hidden = true;
        s.focus = false;
        s.hide_at = None;
        s.motion = None;
        s.position = s.full;
        s.window_hover = false;
        s.tray_hover = false;
        s.dragging = false;
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn suspend(&mut self, instance: &str, value: bool, now: u64) -> AppResult<()> {
        let Some(s) = self.entries.get(instance) else {
            return Ok(());
        };
        if s.suspended == value {
            return Ok(());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(instance).unwrap();
        s.suspended = value;
        s.focus = false;
        s.window_hover = false;
        s.tray_hover = false;
        s.dragging = false;
        s.serial = serial;
        if value {
            s.hide_at = None;
            s.motion = None;
            s.position = project(s.full, s.area, s.edge, s.expanded, false)?;
        } else {
            s.delay(now);
        }
        Ok(())
    }
    pub(crate) fn phase(&mut self, instance: &str, value: &str, now: u64) -> AppResult<()> {
        if ![
            "stop",
            "running",
            "pause",
            "waiting",
            "resting",
            "rest-pause",
        ]
        .contains(&value)
        {
            return Err("DATA_UNAVAILABLE: 番茄阶段无效".into());
        }
        let Some(old) = self.entries.get(instance).map(|s| s.phase.clone()) else {
            return Ok(());
        };
        if old == value {
            return Ok(());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(instance).unwrap();
        s.phase = value.into();
        s.serial = serial;
        if value == "waiting" || old == "resting" && (value == "running" || value == "stop") {
            s.manual_hidden = false;
            s.expand(true, now)?;
        }
        s.delay(now);
        Ok(())
    }
    pub(crate) fn drag(&mut self, owner: &Owner, now: u64) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        if !s.target_visible() {
            return Err(EXPIRED.into());
        }
        s.dragging = true;
        s.hide_at = None;
        s.position = s.frame(now);
        s.motion = None;
        s.serial = serial;
        Ok(())
    }
    /// 坐标只由原生窗口事件提供，前端没有setBounds或任意坐标接口。
    pub(crate) fn settle_drag(
        &mut self,
        owner: &Owner,
        at: Bounds,
        area: WorkArea,
        now: u64,
    ) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let edge = nearest(at, area)?;
        let full = project(at, area, edge, true, false)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        if s.suspended || s.manual_hidden {
            return Err(EXPIRED.into());
        }
        s.area = area;
        s.edge = edge;
        s.full = full;
        s.position = at;
        s.motion = None;
        s.dragging = false;
        s.expanded = true;
        s.serial = serial;
        s.expand(true, now)?;
        s.delay(now);
        Ok(())
    }
    pub(crate) fn reposition(&mut self, owner: &Owner, area: WorkArea, now: u64) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let full = project(
            Bounds {
                x: area.left,
                y: area.top,
                width: 1,
                height: 1,
            },
            area,
            Edge::Bottom,
            true,
            true,
        )?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        if s.suspended {
            return Err(EXPIRED.into());
        }
        s.area = area;
        s.edge = Edge::Bottom;
        s.full = full;
        s.position = full;
        s.motion = None;
        s.expanded = true;
        s.manual_hidden = false;
        s.dragging = false;
        s.serial = serial;
        s.delay(now);
        Ok(())
    }
    /// 工作区或缩放改变时保留贴边和手动显隐，不重新开始计时。
    pub(crate) fn reflow(
        &mut self,
        owner: &Owner,
        actual: Bounds,
        area: WorkArea,
        now: u64,
    ) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let old = self.entries.get(&owner.instance).unwrap();
        let full = project(old.full, area, old.edge, true, false)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        s.area = area;
        s.full = full;
        s.position = actual;
        s.motion = None;
        s.serial = serial;
        s.expand(s.expanded, now)?;
        Ok(())
    }
    pub(crate) fn tick(&mut self, now: u64) -> AppResult<()> {
        let ids = self.entries.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let due = self.entries[&id].hide_at.is_some_and(|due| now >= due);
            if due {
                let serial = self.next()?;
                let s = self.entries.get_mut(&id).unwrap();
                s.hide_at = None;
                if !s.window_hover
                    && !s.tray_hover
                    && !s.dragging
                    && !s.suspended
                    && !s.manual_hidden
                    && s.phase != "waiting"
                {
                    s.expand(false, now)?;
                    s.serial = serial;
                }
            }
            let s = self.entries.get_mut(&id).unwrap();
            if let Some(m) = s
                .motion
                .as_ref()
                .filter(|m| now.saturating_sub(m.at) >= ANIMATION_TIME)
            {
                s.position = m.to;
                s.motion = None;
            }
        }
        Ok(())
    }
    pub(crate) fn confirmed(&mut self, owner: &Owner, serial: u64, bounds: Bounds, visible: bool) {
        if let Some(s) = self
            .entries
            .get_mut(&owner.instance)
            .filter(|s| s.owner == *owner && s.serial == serial)
        {
            s.applied = serial;
            s.confirmed_frame = Some((bounds, visible));
        }
    }
    pub(crate) fn focused(&mut self, owner: &Owner, request: u64) {
        if let Some(s) = self
            .entries
            .get_mut(&owner.instance)
            .filter(|s| s.owner == *owner && s.focus_request == request)
        {
            s.focus = false;
        }
    }
    pub(crate) fn fail(&mut self, owner: &Owner) -> AppResult<()> {
        if !self.current(owner) {
            return Err(EXPIRED.into());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(&owner.instance).unwrap();
        s.fault = Some("专用窗显示失败，请重新打开".into());
        s.ready = false;
        s.focus = false;
        s.hide_at = None;
        s.serial = serial;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .unwrap();
        db
    }
    fn area() -> WorkArea {
        WorkArea {
            left: -1920,
            top: -100,
            width: 1920,
            height: 1040,
            scale_factor: 1.,
        }
    }
    fn open() -> (Book, Surface) {
        let mut b = Book::default();
        let s = b.open(&id(), &"a".repeat(64), area(), false, 0).unwrap();
        (b, s)
    }
    fn ready(b: &mut Book, s: &Surface) -> String {
        let token = id();
        b.bind(&s.owner.label(), &token).unwrap();
        b.ready(&s.owner.label(), &token, 0).unwrap();
        token
    }
    #[test]
    fn defaults_never_enable_a_window_and_saving_is_cas_and_idempotent() {
        let mut db = db();
        let instance = id();
        assert!(!load(&db, &instance).unwrap().config.enabled);
        let request = id();
        let (d, replayed) = configure(
            &mut db,
            &instance,
            &request,
            0,
            Preferences { enabled: true },
        )
        .unwrap();
        assert!(!replayed);
        assert_eq!(d.revision, 1);
        assert!(
            configure(
                &mut db,
                &instance,
                &request,
                0,
                Preferences { enabled: true }
            )
            .unwrap()
            .1
        );
        assert!(configure(
            &mut db,
            &instance,
            &request,
            0,
            Preferences { enabled: false }
        )
        .is_err());
        assert!(configure(&mut db, &instance, &id(), 0, Preferences { enabled: false }).is_err());
        assert!(load(&db, &instance).unwrap().config.enabled);
    }
    #[test]
    fn failed_save_and_corrupt_data_are_preserved_without_enabling_anything() {
        let mut db = db();
        let instance = id();
        db.execute_batch("CREATE TRIGGER refuse_window BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL,'denied');END;").unwrap();
        assert!(configure(&mut db, &instance, &id(), 0, Preferences { enabled: true }).is_err());
        assert!(!load(&db, &instance).unwrap().config.enabled);
        db.execute_batch("DROP TRIGGER refuse_window;").unwrap();
        db.execute(
            "INSERT INTO app_settings VALUES(?1,'broken')",
            [key(&instance).unwrap()],
        )
        .unwrap();
        assert!(configure(&mut db, &instance, &id(), 0, Preferences { enabled: true }).is_err());
        assert_eq!(
            db.query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [key(&instance).unwrap()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "broken"
        );
    }
    #[test]
    fn window_preferences_are_private_and_delete_only_the_chosen_instance() {
        let mut db = db();
        let a = id();
        let b = id();
        configure(&mut db, &a, &id(), 0, Preferences { enabled: true }).unwrap();
        configure(&mut db, &b, &id(), 0, Preferences { enabled: true }).unwrap();
        remove_data(&db, &a).unwrap();
        assert!(!load(&db, &a).unwrap().config.enabled);
        assert!(load(&db, &b).unwrap().config.enabled);
        assert!(load(&db, "shared:foreign").is_err());
    }
    #[test]
    fn all_edges_keep_a_six_pixel_strip_and_preserve_fixed_size_in_negative_coordinates() {
        let a = area();
        let at = Bounds {
            x: -1000,
            y: 500,
            width: 150,
            height: 150,
        };
        for (edge, x, y) in [
            (Edge::Left, -2064, 500),
            (Edge::Right, -6, 500),
            (Edge::Top, -1000, -244),
            (Edge::Bottom, -1000, 934),
        ] {
            assert_eq!(
                project(at, a, edge, false, false).unwrap(),
                Bounds {
                    x,
                    y,
                    width: 150,
                    height: 150
                }
            );
        }
        let full = project(at, a, Edge::Bottom, true, true).unwrap();
        assert_eq!((full.x, full.y), (-1035, 790));
    }
    #[test]
    fn dpi_and_work_area_limits_do_not_wrap_or_accept_nan() {
        let mut a = area();
        a.scale_factor = 1.5;
        let at = Bounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let s = project(at, a, Edge::Right, false, false).unwrap();
        assert_eq!(s.width, 225);
        assert_eq!(s.x, -9);
        a.scale_factor = f64::NAN;
        assert!(project(at, a, Edge::Top, true, false).is_err());
        a.scale_factor = 1.;
        a.left = i32::MAX - 10;
        assert!(project(at, a, Edge::Left, true, false).is_err());
    }
    #[test]
    fn nearest_edge_uses_the_original_bottom_top_left_right_tie_order() {
        let a = WorkArea {
            left: 0,
            top: 0,
            width: 500,
            height: 500,
            scale_factor: 1.,
        };
        assert_eq!(
            nearest(
                Bounds {
                    x: 175,
                    y: 175,
                    width: 150,
                    height: 150
                },
                a
            )
            .unwrap(),
            Edge::Bottom
        );
        assert_eq!(
            nearest(
                Bounds {
                    x: 2,
                    y: 200,
                    width: 150,
                    height: 150
                },
                a
            )
            .unwrap(),
            Edge::Left
        );
    }
    #[test]
    fn odd_work_area_center_and_negative_motion_follow_javascript_rounding() {
        let mut a = area();
        a.width = 1919;
        let at = Bounds {
            x: -1000,
            y: 0,
            width: 150,
            height: 150,
        };
        assert_eq!(project(at, a, Edge::Bottom, true, true).unwrap().x, -1035);
        let from = Bounds { x: -6, ..at };
        let to = Bounds { x: -2, ..at };
        assert_eq!(Motion { from, to, at: 0 }.frame(120).x, -2);
    }
    #[test]
    fn an_open_window_waits_for_its_own_ready_and_does_not_replace_primary_or_another_owner() {
        let (mut b, s) = open();
        assert!(!s.target_visible());
        let token = ready(&mut b, &s);
        assert!(b.get(&s.owner.instance).unwrap().target_visible());
        let second = b.open(&id(), &"b".repeat(64), area(), false, 0).unwrap();
        assert!(b.authorized(&s.owner.label(), &token).is_ok());
        assert!(!role(
            &format!("pomodoro-window-{}-../x", s.owner.instance),
            &s.owner.instance
        ));
        assert!(!b.current(&Owner {
            digest: "c".repeat(64),
            ..s.owner.clone()
        }));
        assert!(b.current(&second.owner));
    }
    #[test]
    fn reload_changes_only_the_same_window_token_and_old_close_cannot_clear_the_new_one() {
        let (mut b, s) = open();
        let old = ready(&mut b, &s);
        let fresh = ready(&mut b, &s);
        assert!(b.close_session(&s.owner.label(), &old).is_err());
        assert!(b.authorized(&s.owner.label(), &fresh).unwrap().ready);
    }
    #[test]
    fn two_second_auto_hide_uses_cubic_motion_and_hover_reverses_without_jumping() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        b.tick(1999).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
        b.tick(2000).unwrap();
        let mid = b.get(&s.owner.instance).unwrap().frame(2120);
        assert!(mid.y > s.frame(0).y && mid.y < 934);
        b.hover(&s.owner, false, true, 2120).unwrap();
        assert_eq!(b.get(&s.owner.instance).unwrap().frame(2120), mid);
        b.tick(2360).unwrap();
        assert_eq!(b.get(&s.owner.instance).unwrap().frame(2360).y, 790);
        b.tick(9000).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
    }
    #[test]
    fn crossing_between_tray_and_window_does_not_hide_while_either_is_hovered() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        b.hover(&s.owner, true, true, 1000).unwrap();
        b.hover(&s.owner, false, true, 1100).unwrap();
        b.hover(&s.owner, true, false, 1200).unwrap();
        b.tick(5000).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
        b.hover(&s.owner, false, false, 5001).unwrap();
        b.tick(7000).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
        b.tick(7001).unwrap();
        assert!(!b.get(&s.owner.instance).unwrap().expanded);
    }
    #[test]
    fn waiting_and_completion_restore_the_clock_without_stealing_focus_or_hiding_again() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        b.hide(&s.owner).unwrap();
        b.phase(&s.owner.instance, "running", 10).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().manual_hidden);
        b.phase(&s.owner.instance, "waiting", 20).unwrap();
        b.tick(9000).unwrap();
        let s = b.get(&s.owner.instance).unwrap();
        assert!(!s.manual_hidden && s.expanded && !s.focus);
    }
    #[test]
    fn suspension_cancels_old_hover_animation_and_keeps_manual_hide_after_restore() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        b.tick(2000).unwrap();
        b.suspend(&s.owner.instance, true, 2050).unwrap();
        assert!(b.hover(&s.owner, true, true, 2100).is_err());
        b.phase(&s.owner.instance, "waiting", 2100).unwrap();
        assert!(!b.get(&s.owner.instance).unwrap().target_visible());
        b.suspend(&s.owner.instance, false, 2200).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().target_visible());
        b.hide(&s.owner).unwrap();
        b.suspend(&s.owner.instance, true, 2300).unwrap();
        b.suspend(&s.owner.instance, false, 2400).unwrap();
        assert!(!b.get(&s.owner.instance).unwrap().target_visible());
    }
    #[test]
    fn dragging_cancels_old_deadlines_snaps_nearest_and_reset_returns_bottom_center() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        b.drag(&s.owner, 0).unwrap();
        b.tick(5000).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
        let at = Bounds {
            x: -1900,
            y: 200,
            width: 150,
            height: 150,
        };
        b.settle_drag(&s.owner, at, area(), 5100).unwrap();
        assert_eq!(b.get(&s.owner.instance).unwrap().frame(5100), at);
        b.tick(5340).unwrap();
        let moved = b.get(&s.owner.instance).unwrap();
        assert_eq!(moved.edge, Edge::Left);
        assert_eq!(moved.frame(5340).x, -1920);
        b.reposition(&s.owner, area(), 5400).unwrap();
        let moved = b.get(&s.owner.instance).unwrap();
        assert_eq!((moved.frame(5400).x, moved.frame(5400).y), (-1035, 790));
    }
    #[test]
    fn replacement_rejects_old_callbacks_and_cleanup_must_be_confirmed_before_reopening() {
        let (mut b, s) = open();
        let token = ready(&mut b, &s);
        assert!(b
            .open(&s.owner.instance, &"b".repeat(64), area(), false, 1)
            .is_err());
        assert!(!b.current(&s.owner));
        assert!(b.authorized(&s.owner.label(), &token).is_err());
        b.confirmed(&s.owner, s.serial, s.frame(0), false);
        b.retired_confirmed(&s.owner);
        let fresh = b
            .open(&s.owner.instance, &"b".repeat(64), area(), false, 2)
            .unwrap();
        assert_ne!(fresh.owner.presentation, s.owner.presentation);
        assert!(b.hover(&s.owner, false, true, 3).is_err());
        assert!(b.current(&fresh.owner));
    }
    #[test]
    fn a_late_native_ack_or_focus_receipt_cannot_confirm_newer_intent() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        let earlier = b.get(&s.owner.instance).unwrap();
        b.open(&s.owner.instance, &s.owner.digest, area(), true, 20)
            .unwrap();
        let current = b.get(&s.owner.instance).unwrap();
        b.confirmed(&s.owner, earlier.serial, earlier.frame(0), true);
        b.focused(&s.owner, earlier.serial);
        let value = b.get(&s.owner.instance).unwrap();
        assert!(value.focus);
        assert_ne!(value.applied, current.serial);
        b.confirmed(&s.owner, current.serial, current.frame(0), true);
        b.focused(&s.owner, current.serial);
        assert!(!b.get(&s.owner.instance).unwrap().focus);
    }
    #[test]
    fn loading_does_not_consume_the_two_second_reveal_before_the_window_is_ready() {
        let (mut b, s) = open();
        b.tick(5000).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
        let token = id();
        b.bind(&s.owner.label(), &token).unwrap();
        b.ready(&s.owner.label(), &token, 5000).unwrap();
        b.tick(6999).unwrap();
        assert!(b.get(&s.owner.instance).unwrap().expanded);
        b.tick(7000).unwrap();
        assert!(!b.get(&s.owner.instance).unwrap().expanded);
    }
    #[test]
    fn an_intermediate_animation_ack_cannot_report_the_final_native_position_as_confirmed() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        b.tick(2000).unwrap();
        let moving = b.get(&s.owner.instance).unwrap();
        b.confirmed(&s.owner, moving.serial, moving.frame(2120), true);
        b.tick(2240).unwrap();
        let ended = b.get(&s.owner.instance).unwrap();
        assert_eq!(ended.snapshot()["nativeSettled"], false);
        b.confirmed(&s.owner, ended.serial, ended.frame(2240), true);
        assert_eq!(
            b.get(&s.owner.instance).unwrap().snapshot()["nativeSettled"],
            true
        );
    }
    #[test]
    fn layout_changes_do_not_repeat_focus_and_new_explicit_focus_keeps_its_own_receipt() {
        let (mut b, s) = open();
        ready(&mut b, &s);
        let requested = b
            .open(&s.owner.instance, &s.owner.digest, area(), true, 10)
            .unwrap();
        b.hover(&s.owner, false, true, 20).unwrap();
        b.focused(&s.owner, requested.focus_request);
        assert!(!b.get(&s.owner.instance).unwrap().focus);
        b.open(&s.owner.instance, &s.owner.digest, area(), true, 30)
            .unwrap();
        b.focused(&s.owner, requested.focus_request);
        assert!(b.get(&s.owner.instance).unwrap().focus);
    }
}
