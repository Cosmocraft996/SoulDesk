//! 已持久化的行情事件由宿主递送和打开图表。
use crate::{
    alert_store,
    alerts::{AlertEvent, Direction, NotificationChannel, PriceKind, Reason},
    store::{err, AppResult},
    windows, Core,
};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

pub(crate) const MARKET_PACKAGE: &str = "local.souldesk.market";
pub(crate) const ISLAND_SUPPRESSED: &str = "NOTIFICATION_DISABLED: 灵动通知当前不显示";
const ACTIVATION_PREFIX: &str = "souldesk-market-alert://event/";
#[cfg(windows)]
pub(crate) const TOAST_GROUP: &str = "market-alert";

const TONE_DIRECTORY: &str = "market-alert-sounds";
const MAX_TONE_BYTES: u64 = 2 * 1024 * 1024;
const TONES: &[(&str, &[(&str, &str)])] = &[
    (
        "AiCoin",
        &[
            ("aicoin-rise.mp3", "上涨铃声"),
            ("aicoin-fall-1.mp3", "下跌铃声 1"),
            ("aicoin-fall-2.mp3", "下跌铃声 2"),
            ("aicoin-gentle.mp3", "平缓铃声"),
            ("aicoin-default.mp3", "通用提示"),
            ("aicoin-alert.mp3", "AiCoin 预警"),
        ],
    ),
    (
        "TradingView · 经典",
        &[
            ("tradingview/3_notes_reverb.mp3", "三音回响"),
            ("tradingview/alarm_clock.mp3", "闹钟"),
            ("tradingview/beep_beep.mp3", "双响哔声"),
            ("tradingview/calling.mp3", "呼叫"),
            ("tradingview/chirpy.mp3", "短促啁啾"),
            ("tradingview/fault.mp3", "故障提示"),
            ("tradingview/fired.mp3", "警报触发"),
            ("tradingview/hand_bell.mp3", "手铃"),
        ],
    ),
    (
        "TradingView · 柔和",
        &[
            ("tradingview/soft/banjo.mp3", "班卓琴"),
            ("tradingview/soft/droplet.mp3", "水滴声"),
            ("tradingview/soft/flickering.mp3", "闪烁声"),
            ("tradingview/soft/hoarse.mp3", "沙哑声"),
            ("tradingview/soft/knock-knock.mp3", "敲门"),
            ("tradingview/soft/promise.mp3", "约定"),
            ("tradingview/soft/trumpets.mp3", "号角"),
            ("tradingview/soft/you-win.mp3", "胜利音效"),
        ],
    ),
    (
        "TradingView · 趣味",
        &[
            ("tradingview/funny/applauses.mp3", "掌声"),
            ("tradingview/funny/breaking-some-glass.mp3", "玻璃碎裂"),
            ("tradingview/funny/cash-register.mp3", "收银机"),
            ("tradingview/funny/chimes-power-down.mp3", "关机铃声"),
            ("tradingview/funny/dramatic-one-note.mp3", "戏剧单音"),
            ("tradingview/funny/jackpot.mp3", "中奖"),
            ("tradingview/funny/man-laughing.mp3", "男子笑声"),
            ("tradingview/funny/martian-gun.mp3", "火星枪"),
        ],
    ),
    (
        "TradingView · 自然",
        &[
            ("tradingview/nature/bullfrog.mp3", "牛蛙"),
            ("tradingview/nature/cat.mp3", "猫叫"),
            ("tradingview/nature/dog.mp3", "狗叫"),
            ("tradingview/nature/raven.mp3", "渡鸦"),
            ("tradingview/nature/rooster.mp3", "公鸡打鸣"),
            ("tradingview/nature/thunder.mp3", "雷声"),
            ("tradingview/nature/water-drop.mp3", "水滴落下"),
            ("tradingview/nature/whale.mp3", "鲸鸣"),
        ],
    ),
    (
        "TradingView · 人声",
        &[
            ("tradingview/voices/attention.mp3", "请注意"),
            ("tradingview/voices/guess-what.mp3", "猜猜看"),
            ("tradingview/voices/hey-pssst.mp3", "嘿，嘘"),
            ("tradingview/voices/hey-take-a-look.mp3", "嘿，快来看"),
            ("tradingview/voices/hey.mp3", "嘿"),
            ("tradingview/voices/look-at-me.mp3", "看我"),
            ("tradingview/voices/mmm-sexy.mp3", "嗯，性感"),
            ("tradingview/voices/oh.mp3", "哦"),
        ],
    ),
];

pub(crate) fn default_tone(direction: Direction) -> &'static str {
    match direction {
        Direction::Up => "aicoin-rise.mp3",
        Direction::Down => "aicoin-fall-2.mp3",
    }
}

pub(crate) fn tone_id(value: &str) -> Option<&'static str> {
    TONES
        .iter()
        .flat_map(|(_, tones)| *tones)
        .find_map(|&(id, _)| (id == value).then_some(id))
}

pub(crate) fn tone_roots(app: &AppHandle, root: &Path) -> AppResult<[PathBuf; 3]> {
    Ok([
        root.join(TONE_DIRECTORY),
        app.path().resource_dir().map_err(err)?.join(TONE_DIRECTORY),
        // 未打包的开发与测试程序可直接使用工程内的发布资源。
        Path::new(env!("CARGO_MANIFEST_DIR")).join(TONE_DIRECTORY),
    ])
}

pub(crate) fn tone_groups(roots: &[PathBuf]) -> serde_json::Value {
    serde_json::Value::Array(
        TONES
            .iter()
            .map(|(name, tones)| {
                let sounds: Vec<_> = tones
                    .iter()
                    .map(|(id, label)| {
                        let available = roots.iter().any(|root| root.join(id).is_file());
                        serde_json::json!({ "id": id, "label": label, "available": available })
                    })
                    .collect();
                serde_json::json!({ "name": name, "sounds": sounds })
            })
            .collect(),
    )
}

pub(crate) fn read_tone(roots: &[PathBuf], id: &str) -> AppResult<Vec<u8>> {
    let id = tone_id(id).ok_or("INVALID_ARGUMENT: 未知的行情铃声")?;
    let mut file = None;
    for root in roots {
        match File::open(root.join(id)) {
            Ok(candidate) => {
                file = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("无法读取行情铃声 {id}：{error}")),
        }
    }
    let file = file.ok_or_else(|| format!("找不到行情铃声 {id}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_TONE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > MAX_TONE_BYTES
        || !(bytes.starts_with(b"ID3")
            || bytes
                .get(..2)
                .is_some_and(|header| header[0] == 0xff && header[1] & 0xe0 == 0xe0))
    {
        return Err(format!("行情铃声 {id} 须为 2 MiB 以内的 MP3"));
    }
    Ok(bytes)
}

pub(crate) fn encode_tone(bytes: Vec<u8>) -> serde_json::Value {
    use base64::{engine::general_purpose::STANDARD, Engine};
    serde_json::Value::String(STANDARD.encode(bytes))
}

pub(crate) fn market_sound(
    app: &AppHandle,
    root: &Path,
    direction: Direction,
    id: &str,
) -> AppResult<serde_json::Value> {
    let roots = tone_roots(app, root)?;
    let default = default_tone(direction);
    read_tone(&roots, id)
        .or_else(|error| {
            if id != default {
                read_tone(&roots, default)
            } else {
                Err(error)
            }
        })
        .map(encode_tone)
}

pub(crate) fn content(event: &AlertEvent) -> (String, String) {
    let market = if event.key.market == "spot" {
        "现货"
    } else {
        "USDT 永续"
    };
    let price = match event.key.price_kind {
        PriceKind::Last => "成交价",
        PriceKind::Mark => "标记价",
    };
    let reason = match (event.reason, event.direction) {
        (Reason::Crossed, Direction::Up) => "向上穿越",
        (Reason::Crossed, Direction::Down) => "向下穿越",
        (Reason::CooldownDue, _) => "冷却后补提醒",
        (Reason::RecoveredMet, _) => "恢复后到价",
        (Reason::InitialMet, _) => "启用时已到价",
        (Reason::Reached, Direction::Up) => "向上到价",
        (Reason::Reached, Direction::Down) => "向下到价",
    };
    let mut message = format!(
        "{reason} · 触发 {} · 目标 {}",
        event.observed_price, event.threshold
    );
    if event.replayed {
        let when = chrono::DateTime::from_timestamp_millis(event.source_utc_ms as i64)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S%.3f")
                    .to_string()
            })
            .unwrap_or_else(|| event.source_utc_ms.to_string());
        message = format!("重启补报 · {when} · {message}");
    }
    (
        format!("{} · {market} · {price}", event.key.symbol),
        message,
    )
}

pub(crate) fn open(app: &AppHandle, id: &str, channel: NotificationChannel) -> AppResult<()> {
    let event = alert_store::notification_event(&app.state::<Core>().store, id)?;
    if event.notification_channel != channel {
        return Err("该事件的通知渠道不匹配".into());
    }
    windows::open_market_alert_chart(app, &event.key)
}

pub(crate) fn activation_event(argument: &str) -> Option<&str> {
    let id = argument.strip_prefix(ACTIVATION_PREFIX)?;
    let mut fields = id.split(':');
    let rule = fields.next()?;
    if rule.len() != 36 || uuid::Uuid::parse_str(rule).is_err() {
        return None;
    }
    for _ in 0..2 {
        let number = fields.next()?;
        if !number.bytes().all(|byte| byte.is_ascii_digit()) || number.parse::<u64>().ok()? == 0 {
            return None;
        }
    }
    fields.next().is_none().then_some(id)
}

pub(crate) fn activate_argument(app: &AppHandle, mut args: impl Iterator<Item = String>) {
    let core = app.state::<Core>();
    if core.integration.is_some() || core.verification.is_some() {
        return;
    }
    if let Some(id) = args.find_map(|arg| activation_event(&arg).map(str::to_owned)) {
        windows::dispatch(app, move |app| open(app, &id, NotificationChannel::Windows));
    }
}

#[cfg(windows)]
pub(crate) fn toast_tag(event_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(event_id.as_bytes()))[..16].into()
}

#[cfg(windows)]
pub(crate) fn toast_xml(event: &AlertEvent) -> AppResult<::windows::Data::Xml::Dom::XmlDocument> {
    use crate::store::err;
    use ::windows::{
        core::{h, HSTRING},
        Data::Xml::Dom::XmlDocument,
    };
    let xml = XmlDocument::new().map_err(err)?;
    xml.LoadXml(h!("<toast activationType='protocol'><visual><binding template='ToastGeneric'><text/><text/><text/></binding></visual><actions><action content='查看 K 线' activationType='protocol'/></actions><audio silent='true'/></toast>")).map_err(err)?;
    let uri = HSTRING::from(format!("{ACTIVATION_PREFIX}{}", event.id));
    xml.DocumentElement()
        .map_err(err)?
        .SetAttribute(h!("launch"), &uri)
        .map_err(err)?;
    let actions = xml.GetElementsByTagName(h!("action")).map_err(err)?;
    use ::windows::{core::Interface, Data::Xml::Dom::XmlElement};
    let action: XmlElement = actions.Item(0).map_err(err)?.cast().map_err(err)?;
    action.SetAttribute(h!("arguments"), &uri).map_err(err)?;
    let (instrument, message) = content(event);
    let title = format!("SoulDesk · {}", event.rule_name);
    let lines = xml.GetElementsByTagName(h!("text")).map_err(err)?;
    for (index, value) in [title, format!("币安 · {instrument}"), message]
        .iter()
        .enumerate()
    {
        let text = xml.CreateTextNode(&HSTRING::from(value)).map_err(err)?;
        lines
            .Item(index as u32)
            .map_err(err)?
            .AppendChild(&text)
            .map_err(err)?;
    }
    Ok(xml)
}

#[cfg(windows)]
pub(crate) fn windows_notification(
    store: &crate::store::Store,
    identifier: &str,
    suppress_popup: bool,
    event: &AlertEvent,
) -> AppResult<()> {
    use crate::store::err;
    use ::windows::{
        core::HSTRING,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
    };
    crate::reminder_store::ensure_windows_enabled(store)?;
    let toast = ToastNotification::CreateToastNotification(&toast_xml(event)?).map_err(err)?;
    toast
        .SetTag(&HSTRING::from(toast_tag(&event.id)))
        .map_err(err)?;
    toast.SetGroup(&HSTRING::from(TOAST_GROUP)).map_err(err)?;
    if suppress_popup {
        // 隔离检查读取操作中心记录，不弹出会打断用户工作的横幅。
        toast.SetSuppressPopup(true).map_err(err)?;
    }
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identifier))
        .and_then(|notifier| notifier.Show(&toast))
        .map_err(err)
}

#[cfg(not(windows))]
pub(crate) fn windows_notification(
    _: &crate::store::Store,
    _: &str,
    _: bool,
    _: &AlertEvent,
) -> AppResult<()> {
    Err("当前平台未验证实际警报通知适配器".into())
}

pub(crate) fn deliver(app: &AppHandle, event: &AlertEvent) -> AppResult<()> {
    let core = app.state::<Core>();
    match event.notification_channel {
        NotificationChannel::Windows => windows_notification(
            &core.store,
            &app.config().identifier,
            core.integration
                .as_ref()
                .is_some_and(|config| config.market_check()),
            event,
        ),
        NotificationChannel::Island => core.reminders.market_alert(app, event),
    }
}
