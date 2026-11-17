//! 已启用消费者的单次文本采样；与系统读取分开，关闭时不访问剪贴板。
use crate::store::AppResult;
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct Consumer {
    pub instance: String,
    pub config_revision: u64,
}
pub(crate) struct Delivery {
    pub consumer: Consumer,
    pub text: String,
}
pub(crate) struct Failure {
    pub consumer: Consumer,
    pub error: String,
}
pub(crate) enum Read {
    Text(String),
    Ignore,
    Retry,
}
pub(crate) trait Reader {
    fn sequence(&mut self) -> AppResult<u32>;
    fn read(&mut self, expected_sequence: u32) -> AppResult<Read>;
}
#[derive(Default)]
pub(crate) struct Tick {
    pub deliveries: Vec<Delivery>,
    pub failures: Vec<Failure>,
}
struct Watch {
    revision: u64,
    sequence: u32,
}
#[derive(Default)]
pub(crate) struct Gate {
    watches: HashMap<String, Watch>,
}
impl Gate {
    pub(crate) fn tick(
        &mut self,
        owners: &[Consumer],
        reader: &mut impl Reader,
    ) -> AppResult<Tick> {
        self.watches
            .retain(|id, _| owners.iter().any(|owner| owner.instance == *id));
        if owners.is_empty() {
            return Ok(Tick::default());
        }
        let sequence = reader.sequence()?;
        if sequence == 0 {
            return Err("CLIPBOARD_ERROR: 当前会话无法观察剪贴板序号".into());
        }
        let mut eligible = Vec::new();
        for owner in owners {
            match self.watches.get(&owner.instance) {
                Some(watch) if watch.revision == owner.config_revision => {
                    if watch.sequence != sequence {
                        eligible.push(owner.clone());
                    }
                }
                _ => {
                    self.watches.insert(
                        owner.instance.clone(),
                        Watch {
                            revision: owner.config_revision,
                            sequence,
                        },
                    );
                }
            }
        }
        if eligible.is_empty() {
            return Ok(Tick::default());
        }
        let read = reader.read(sequence);
        if matches!(read, Ok(Read::Retry)) {
            return Ok(Tick::default());
        }
        // 一个序号只处理一次；锁暂不可用/读取中被替换才保留重试资格。
        for owner in &eligible {
            if let Some(watch) = self.watches.get_mut(&owner.instance) {
                watch.sequence = sequence;
            }
        }
        let mut result = Tick::default();
        match read {
            Ok(Read::Text(text)) if !text.is_empty() => {
                result.deliveries = eligible
                    .into_iter()
                    .map(|consumer| Delivery {
                        consumer,
                        text: text.clone(),
                    })
                    .collect();
            }
            Err(error) => {
                result.failures = eligible
                    .into_iter()
                    .map(|consumer| Failure {
                        consumer,
                        error: error.clone(),
                    })
                    .collect();
            }
            _ => {}
        }
        Ok(result)
    }
    pub(crate) fn suppress(&mut self, sequence: u32) {
        for watch in self.watches.values_mut() {
            watch.sequence = sequence;
        }
    }
}
fn decode_text(units: &[u16]) -> AppResult<String> {
    let end = units
        .iter()
        .take(100001)
        .position(|&unit| unit == 0)
        .ok_or("DATA_UNAVAILABLE: 剪贴板文本未终止或超过50000字符，未截断保存")?;
    let text = String::from_utf16(&units[..end])
        .map_err(|_| "DATA_UNAVAILABLE: 剪贴板Unicode文本无效，未替换为猜测内容")?;
    if text.chars().take(50001).count() > 50000 {
        return Err("CAPACITY_EXCEEDED: 剪贴板文本超过50000字符，未截断保存".into());
    }
    Ok(text)
}
fn excluded(all: bool, include_history: Option<u32>) -> AppResult<bool> {
    if all {
        return Ok(true);
    }
    match include_history {
        None | Some(1) => Ok(false),
        Some(0) => Ok(true),
        _ => Err("DATA_UNAVAILABLE: 剪贴板历史授权格式无效，未记录".into()),
    }
}
fn own_copy(bytes: &[u8], pid: u32) -> AppResult<bool> {
    let value: [u8; 4] = bytes
        .try_into()
        .map_err(|_| "DATA_UNAVAILABLE: 自身复制标记无效，未记录")?;
    let owner = u32::from_le_bytes(value);
    if owner == 0 || pid == 0 {
        return Err("DATA_UNAVAILABLE: 自身复制进程标记无效，未记录".into());
    }
    Ok(owner == pid)
}

/// 构造不会访问系统；只有已启用消费者的Gate才调用Reader。
#[derive(Default)]
pub(crate) struct SystemClipboard {
    formats: Option<(u32, u32, u32)>,
}
#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };
    struct ClipboardLock;
    impl Drop for ClipboardLock {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    struct MemoryLock(HGLOBAL);
    impl Drop for MemoryLock {
        fn drop(&mut self) {
            unsafe {
                GlobalUnlock(self.0);
            }
        }
    }
    fn bytes(format: u32, limit: usize) -> AppResult<Option<Vec<u8>>> {
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return Ok(None);
        }
        let size = unsafe { GlobalSize(handle) };
        if size == 0 {
            return Err("DATA_UNAVAILABLE: 剪贴板格式内存无效，未记录".into());
        }
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            return Ok(None);
        }
        let _lock = MemoryLock(handle);
        Ok(Some(unsafe {
            std::slice::from_raw_parts(pointer.cast::<u8>(), size.min(limit)).to_vec()
        }))
    }
    fn registered(name: &str) -> AppResult<u32> {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
        if format == 0 {
            return Err("CLIPBOARD_ERROR: 无法识别剪贴板排除记录格式".into());
        }
        Ok(format)
    }
    impl Reader for SystemClipboard {
        fn sequence(&mut self) -> AppResult<u32> {
            let value = unsafe { GetClipboardSequenceNumber() };
            if value == 0 {
                return Err("CLIPBOARD_ERROR: 当前会话无法观察剪贴板序号".into());
            }
            Ok(value)
        }
        fn read(&mut self, expected_sequence: u32) -> AppResult<Read> {
            if self.sequence()? != expected_sequence {
                return Ok(Read::Retry);
            }
            let (exclude_format, history_format, self_format) = match self.formats {
                Some(formats) => formats,
                None => {
                    let formats = (
                        registered("ExcludeClipboardContentFromMonitorProcessing")?,
                        registered("CanIncludeInClipboardHistory")?,
                        registered(crate::clipboard_native::system::SELF_FORMAT)?,
                    );
                    self.formats = Some(formats);
                    formats
                }
            };
            if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
                return Ok(Read::Retry);
            }
            let lock = ClipboardLock;
            let result = (|| {
                let all = unsafe { IsClipboardFormatAvailable(exclude_format) } != 0;
                if all {
                    return Ok(Read::Ignore);
                }
                if unsafe { IsClipboardFormatAvailable(self_format) } != 0 {
                    let Some(marker) = bytes(self_format, 4)? else {
                        return Ok(Read::Retry);
                    };
                    if own_copy(&marker, unsafe {
                        windows_sys::Win32::System::Threading::GetCurrentProcessId()
                    })? {
                        return Ok(Read::Ignore);
                    }
                }
                let history = if unsafe { IsClipboardFormatAvailable(history_format) } != 0 {
                    let Some(value) = bytes(history_format, 4)? else {
                        return Ok(Read::Retry);
                    };
                    let value: [u8; 4] = value
                        .try_into()
                        .map_err(|_| "DATA_UNAVAILABLE: 剪贴板历史授权格式长度无效，未记录")?;
                    Some(u32::from_le_bytes(value))
                } else {
                    None
                };
                if excluded(false, history)? {
                    return Ok(Read::Ignore);
                }
                let unicode = windows::Win32::System::Ole::CF_UNICODETEXT.0 as u32;
                if unsafe { IsClipboardFormatAvailable(unicode) } == 0 {
                    return Ok(Read::Ignore);
                }
                let Some(value) = bytes(unicode, 100001 * 2)? else {
                    return Ok(Read::Retry);
                };
                let units = value
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                let text = decode_text(&units)?;
                Ok(if text.is_empty() {
                    Read::Ignore
                } else {
                    Read::Text(text)
                })
            })();
            drop(lock);
            // 关闭后再核对序号；延迟呈现或另一个复制已替换时不投递旧文字。
            if self.sequence()? != expected_sequence {
                return Ok(Read::Retry);
            }
            result
        }
    }
}
#[cfg(not(windows))]
impl Reader for SystemClipboard {
    fn sequence(&mut self) -> AppResult<u32> {
        Err("UNSUPPORTED: 系统剪切板采样当前仅支持Windows".into())
    }
    fn read(&mut self, _expected_sequence: u32) -> AppResult<Read> {
        Err("UNSUPPORTED: 系统剪切板采样当前仅支持Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        sequence: u32,
        sequence_calls: usize,
        reads: usize,
        values: VecDeque<AppResult<Read>>,
    }
    impl Fake {
        fn new(sequence: u32) -> Self {
            Self {
                sequence,
                sequence_calls: 0,
                reads: 0,
                values: VecDeque::new(),
            }
        }
        fn text(&mut self, text: &str) {
            self.values.push_back(Ok(Read::Text(text.into())));
        }
    }
    impl Reader for Fake {
        fn sequence(&mut self) -> AppResult<u32> {
            self.sequence_calls += 1;
            Ok(self.sequence)
        }
        fn read(&mut self, expected_sequence: u32) -> AppResult<Read> {
            assert_eq!(self.sequence, expected_sequence);
            self.reads += 1;
            self.values.pop_front().expect("意外读取剪贴板")
        }
    }
    fn owner(id: &str, revision: u64) -> Consumer {
        Consumer {
            instance: id.into(),
            config_revision: revision,
        }
    }
    #[test]
    fn clipboard_capture_disabled_never_calls_reader_and_new_owner_only_sets_baseline() {
        let mut gate = Gate::default();
        let mut io = Fake::new(10);
        assert!(gate.tick(&[], &mut io).unwrap().deliveries.is_empty());
        assert_eq!(io.sequence_calls, 0);
        assert_eq!(io.reads, 0);
        let owners = [owner("a", 1)];
        gate.tick(&owners, &mut io).unwrap();
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 0);
        gate.tick(&[], &mut io).unwrap();
        let before = io.sequence_calls;
        gate.tick(&[], &mut io).unwrap();
        assert_eq!(io.sequence_calls, before);
        io.sequence = 11;
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 0);
    }
    #[test]
    fn clipboard_capture_one_read_fans_out_and_removing_one_owner_preserves_other() {
        let mut gate = Gate::default();
        let mut io = Fake::new(10);
        let owners = [owner("a", 1), owner("b", 1)];
        gate.tick(&owners, &mut io).unwrap();
        io.sequence = 11;
        io.text("中文🙂");
        let result = gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 1);
        assert_eq!(result.deliveries.len(), 2);
        assert!(result
            .deliveries
            .iter()
            .all(|d| d.text == "中文🙂" && d.consumer.config_revision == 1));
        io.sequence = 12;
        io.text("继续");
        let result = gate.tick(&[owner("b", 1)], &mut io).unwrap();
        assert_eq!(result.deliveries.len(), 1);
        assert_eq!(result.deliveries[0].consumer.instance, "b");
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 2);
        io.sequence = 13;
        io.text("再次");
        assert_eq!(gate.tick(&owners, &mut io).unwrap().deliveries.len(), 2);
        assert_eq!(io.reads, 3);
    }
    #[test]
    fn clipboard_capture_changed_config_does_not_receive_text_copied_before_enabling() {
        let mut gate = Gate::default();
        let mut io = Fake::new(10);
        gate.tick(&[owner("a", 1), owner("b", 1)], &mut io).unwrap();
        io.sequence = 11;
        io.text("旧代次文字");
        let owners = [owner("a", 2), owner("b", 1)];
        let result = gate.tick(&owners, &mut io).unwrap();
        assert_eq!(result.deliveries.len(), 1);
        assert_eq!(result.deliveries[0].consumer.instance, "b");
        io.sequence = 12;
        io.text("新变化");
        let result = gate.tick(&owners, &mut io).unwrap();
        assert_eq!(result.deliveries.len(), 2);
        assert_eq!(
            result
                .deliveries
                .iter()
                .find(|d| d.consumer.instance == "a")
                .unwrap()
                .consumer
                .config_revision,
            2
        );
    }
    #[test]
    fn clipboard_capture_busy_clipboard_is_not_consumed_or_replayed_after_success() {
        let mut gate = Gate::default();
        let mut io = Fake::new(10);
        let owners = [owner("a", 1)];
        gate.tick(&owners, &mut io).unwrap();
        io.sequence = 11;
        io.values.push_back(Ok(Read::Retry));
        assert_eq!(gate.tick(&owners, &mut io).unwrap().deliveries.len(), 0);
        io.text("延迟读取");
        assert_eq!(gate.tick(&owners, &mut io).unwrap().deliveries.len(), 1);
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 2);
    }
    #[test]
    fn clipboard_capture_ignored_or_invalid_sequence_is_consumed_once_without_fake_text() {
        let mut gate = Gate::default();
        let mut io = Fake::new(10);
        let owners = [owner("a", 1)];
        gate.tick(&owners, &mut io).unwrap();
        io.sequence = 11;
        io.values.push_back(Ok(Read::Ignore));
        let r = gate.tick(&owners, &mut io).unwrap();
        assert!(r.deliveries.is_empty());
        assert!(r.failures.is_empty());
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 1);
        io.sequence = 12;
        io.values
            .push_back(Err("DATA_UNAVAILABLE: 文本无效".into()));
        let r = gate.tick(&owners, &mut io).unwrap();
        assert!(r.deliveries.is_empty());
        assert_eq!(r.failures[0].consumer.instance, "a");
        assert!(r.failures[0].error.starts_with("DATA_UNAVAILABLE:"));
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 2);
    }
    #[test]
    fn clipboard_capture_sequence_wrap_and_replaced_during_read_keep_only_current_text() {
        let mut gate = Gate::default();
        let mut io = Fake::new(u32::MAX);
        let owners = [owner("a", 1)];
        gate.tick(&owners, &mut io).unwrap();
        io.sequence = 1;
        io.values.push_back(Ok(Read::Retry));
        gate.tick(&owners, &mut io).unwrap();
        io.sequence = 2;
        io.text("当前文本");
        let r = gate.tick(&owners, &mut io).unwrap();
        assert_eq!(r.deliveries.len(), 1);
        assert_eq!(r.deliveries[0].text, "当前文本");
    }
    #[test]
    fn clipboard_capture_own_write_suppression_does_not_hide_next_external_copy() {
        let mut gate = Gate::default();
        let mut io = Fake::new(10);
        let owners = [owner("a", 1)];
        gate.tick(&owners, &mut io).unwrap();
        io.sequence = 11;
        gate.suppress(11);
        gate.tick(&owners, &mut io).unwrap();
        assert_eq!(io.reads, 0);
        io.sequence = 12;
        io.text("外部复制");
        assert_eq!(gate.tick(&owners, &mut io).unwrap().deliveries.len(), 1);
    }
    #[test]
    fn clipboard_capture_utf16_requires_termination_valid_surrogates_and_complete_bounded_text() {
        let mut value = "中文🙂\r\n".encode_utf16().collect::<Vec<_>>();
        value.extend([0, 88, 89]);
        assert_eq!(decode_text(&value).unwrap(), "中文🙂\r\n");
        assert!(decode_text(&[65]).is_err());
        assert!(decode_text(&[0xd800, 0]).is_err());
        let mut max = "🙂".repeat(50000).encode_utf16().collect::<Vec<_>>();
        max.push(0);
        assert_eq!(decode_text(&max).unwrap().chars().count(), 50000);
        let mut over = "界".repeat(50001).encode_utf16().collect::<Vec<_>>();
        over.push(0);
        assert!(decode_text(&over).is_err());
        assert_eq!(decode_text(&[0]).unwrap(), "");
    }
    #[test]
    fn clipboard_capture_honors_history_exclusion_without_confusing_cloud_upload_setting() {
        assert!(excluded(true, None).unwrap());
        assert!(excluded(false, Some(0)).unwrap());
        assert!(!excluded(false, None).unwrap());
        assert!(!excluded(false, Some(1)).unwrap());
        assert!(excluded(false, Some(2)).is_err());
    }
    #[test]
    fn clipboard_capture_only_own_process_marker_suppresses_copy_without_dropping_next_external_source(
    ) {
        assert!(own_copy(&42u32.to_le_bytes(), 42).unwrap());
        assert!(!own_copy(&43u32.to_le_bytes(), 42).unwrap());
        assert!(own_copy(&[], 42).is_err());
        assert!(own_copy(&[42, 0, 0], 42).is_err());
        assert!(own_copy(&[0, 0, 0, 0], 42).is_err());
    }
}
