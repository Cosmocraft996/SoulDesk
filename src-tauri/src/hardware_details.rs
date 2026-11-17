//! 监控面板所需的固定本机指标；不返回网卡名称、地址或进程列表。
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AdapterKey {
    pub high: u32,
    pub low: u32,
    pub physical: u32,
}
impl AdapterKey {
    pub fn parse(key: &str) -> Option<Self> {
        let (luid, rest) = key.split_once("_phys_")?;
        let (high, low) = luid.split_once('_')?;
        let physical = rest.split_once("_eng_")?.0.parse().ok()?;
        if physical >= 64 {
            return None;
        }
        Some(Self {
            high: u32::from_str_radix(high.strip_prefix("0x")?, 16).ok()?,
            low: u32::from_str_radix(low.strip_prefix("0x")?, 16).ok()?,
            physical,
        })
    }
    fn label(self) -> String {
        format!("{:08x}:{:08x}/{}", self.high, self.low, self.physical)
    }
}
fn temperature(deci_celsius: u32) -> Option<f64> {
    (deci_celsius > 0 && deci_celsius <= 2000).then_some(f64::from(deci_celsius) / 10.0)
}
#[derive(Clone, Copy)]
struct Counter {
    received: u64,
    sent: u64,
}
fn rates(
    previous: &BTreeMap<u64, Counter>,
    current: &BTreeMap<u64, Counter>,
    elapsed: Duration,
) -> Option<(f64, f64)> {
    if current.is_empty()
        || elapsed.is_zero()
        || elapsed > Duration::from_secs(10)
        || !previous.keys().eq(current.keys())
    {
        return None;
    }
    let (received, sent) =
        current
            .iter()
            .try_fold((0u64, 0u64), |(received, sent), (id, row)| {
                Some((
                    received.checked_add(row.received.checked_sub(previous[id].received)?)?,
                    sent.checked_add(row.sent.checked_sub(previous[id].sent)?)?,
                ))
            })?;
    Some((
        received as f64 / elapsed.as_secs_f64(),
        sent as f64 / elapsed.as_secs_f64(),
    ))
}
#[derive(Default)]
pub(crate) struct Sampler {
    previous: Option<(Instant, BTreeMap<u64, Counter>)>,
}
impl Sampler {
    pub fn sample(&mut self, memory_total: Option<u64>, adapter: Option<AdapterKey>) -> Value {
        let network = match platform::network() {
            Ok(rows) if !rows.is_empty() => {
                let at = Instant::now();
                let rate = self.previous.as_ref().and_then(|(before, previous)| {
                    rates(previous, &rows, at.duration_since(*before))
                });
                let received = rows
                    .values()
                    .try_fold(0u64, |sum, row| sum.checked_add(row.received));
                let sent = rows
                    .values()
                    .try_fold(0u64, |sum, row| sum.checked_add(row.sent));
                self.previous = Some((at, rows));
                json!({"downloadBytesPerSecond":rate.map(|value|value.0),"uploadBytesPerSecond":rate.map(|value|value.1),"receivedBytes":received.map(|value|value.to_string()),"sentBytes":sent.map(|value|value.to_string()),"error":rate.is_none().then_some("网络正在采样，或接口/计数器已变化")})
            }
            result => {
                self.previous = None;
                let error = result.err().unwrap_or("没有已连接的非回环接口".into());
                json!({"downloadBytesPerSecond":null,"uploadBytesPerSecond":null,"receivedBytes":null,"sentBytes":null,"error":error})
            }
        };
        let temperature = adapter
            .ok_or_else(|| "GPU正在采样，暂不能匹配温度适配器".to_owned())
            .and_then(platform::gpu_temperature);
        json!({"cpuLogicalCount":platform::cpu_count(),"memoryTotalBytes":memory_total.map(|value|value.to_string()),"gpuAdapter":adapter.map(AdapterKey::label),"gpuTemperature":{"celsius":temperature.as_ref().ok(),"error":temperature.err()},"network":network})
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{mem::size_of, ptr::null_mut};
    use windows_sys::{
        Wdk::Graphics::Direct3D::*,
        Win32::{
            NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
            System::Threading::{GetActiveProcessorCount, ALL_PROCESSOR_GROUPS},
        },
    };
    pub fn cpu_count() -> Option<u32> {
        let count = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
        (count > 0).then_some(count)
    }
    pub fn network() -> Result<BTreeMap<u64, Counter>, String> {
        let mut pointer = null_mut();
        let status = unsafe { GetIfTable2(&mut pointer) };
        if status != 0 {
            return Err(format!("网络计数器读取失败（{status:#x}）"));
        }
        if pointer.is_null() {
            return Err("网络计数器未返回数据".into());
        }
        struct Table(*mut MIB_IF_TABLE2);
        impl Drop for Table {
            fn drop(&mut self) {
                unsafe { FreeMibTable(self.0.cast()) };
            }
        }
        let table = Table(pointer);
        let count = unsafe { (*table.0).NumEntries } as usize;
        if count > 8192 {
            return Err("网络接口数量超限".into());
        }
        let rows = unsafe { std::slice::from_raw_parts((*table.0).Table.as_ptr(), count) };
        Ok(rows
            .iter()
            .filter(|row| row.OperStatus == IfOperStatusUp && row.Type != IF_TYPE_SOFTWARE_LOOPBACK)
            .map(|row| {
                (
                    unsafe { row.InterfaceLuid.Value },
                    Counter {
                        received: row.InOctets,
                        sent: row.OutOctets,
                    },
                )
            })
            .collect())
    }
    pub fn gpu_temperature(key: AdapterKey) -> Result<f64, String> {
        let mut query = D3DKMT_ENUMADAPTERS2::default();
        let status = unsafe { D3DKMTEnumAdapters2(&mut query) };
        if status < 0 {
            return Err(format!("GPU适配器枚举失败（{status:#x}）"));
        }
        if query.NumAdapters == 0 || query.NumAdapters > 64 {
            return Err("GPU适配器数量不可用或超限".into());
        }
        struct Adapters(Vec<D3DKMT_ADAPTERINFO>);
        impl Drop for Adapters {
            fn drop(&mut self) {
                for adapter in &self.0 {
                    if adapter.hAdapter != 0 {
                        unsafe {
                            D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER {
                                hAdapter: adapter.hAdapter,
                            });
                        }
                    }
                }
            }
        }
        let mut adapters = Adapters(vec![
            D3DKMT_ADAPTERINFO::default();
            query.NumAdapters as usize
        ]);
        query.pAdapters = adapters.0.as_mut_ptr();
        let status = unsafe { D3DKMTEnumAdapters2(&mut query) };
        if status < 0 {
            return Err(format!("GPU适配器已变化或枚举失败（{status:#x}）"));
        }
        if query.NumAdapters as usize > adapters.0.len() {
            return Err("GPU适配器返回数量异常".into());
        }
        let adapter = adapters.0[..query.NumAdapters as usize]
            .iter()
            .find(|adapter| {
                adapter.AdapterLuid.HighPart as u32 == key.high
                    && adapter.AdapterLuid.LowPart == key.low
            })
            .ok_or("未找到占用率对应的GPU适配器")?;
        let mut data = D3DKMT_ADAPTER_PERFDATA {
            PhysicalAdapterIndex: key.physical,
            ..Default::default()
        };
        let mut info = D3DKMT_QUERYADAPTERINFO {
            hAdapter: adapter.hAdapter,
            Type: KMTQAITYPE_ADAPTERPERFDATA,
            pPrivateDriverData: (&mut data as *mut D3DKMT_ADAPTER_PERFDATA).cast(),
            PrivateDriverDataSize: size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
        };
        let status = unsafe { D3DKMTQueryAdapterInfo(&mut info) };
        if status < 0 {
            return Err(format!("GPU驱动未提供主温度（{status:#x}）"));
        }
        temperature(data.Temperature).ok_or_else(|| "GPU驱动主温度缺失或异常".into())
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn cpu_count() -> Option<u32> {
        None
    }
    pub fn network() -> Result<BTreeMap<u64, Counter>, String> {
        Err("当前平台不支持网络计数器".into())
    }
    pub fn gpu_temperature(_: AdapterKey) -> Result<f64, String> {
        Err("当前平台不支持GPU主温度".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(values: &[(u64, u64, u64)]) -> BTreeMap<u64, Counter> {
        values
            .iter()
            .map(|(id, received, sent)| {
                (
                    *id,
                    Counter {
                        received: *received,
                        sent: *sent,
                    },
                )
            })
            .collect()
    }
    #[test]
    fn network_uses_elapsed_time_and_rewarms_after_reset_or_interface_changes() {
        let first = rows(&[(1, 1000, 200), (2, 500, 100)]);
        let second = rows(&[(1, 3000, 500), (2, 1500, 400)]);
        assert_eq!(
            rates(&first, &second, Duration::from_secs(2)),
            Some((1500.0, 300.0))
        );
        assert_eq!(
            rates(&first, &first, Duration::from_secs(2)),
            Some((0.0, 0.0))
        );
        assert_eq!(
            rates(
                &first,
                &rows(&[(1, 1, 300), (2, 2000, 500)]),
                Duration::from_secs(1)
            ),
            None
        );
        assert_eq!(
            rates(&first, &rows(&[(1, 3000, 500)]), Duration::from_secs(1)),
            None
        );
        assert_eq!(rates(&first, &second, Duration::ZERO), None);
        assert_eq!(rates(&first, &second, Duration::from_secs(11)), None);
    }
    #[test]
    fn gpu_adapter_identity_preserves_luid_and_physical_index() {
        assert_eq!(
            AdapterKey::parse("0x00000000_0x0000a123_phys_1_eng_0_engtype_3D"),
            Some(AdapterKey {
                high: 0,
                low: 0xa123,
                physical: 1
            })
        );
        assert_eq!(
            AdapterKey::parse("0xffffffff_0x0000a123_phys_0_eng_2_engtype_Compute"),
            Some(AdapterKey {
                high: u32::MAX,
                low: 0xa123,
                physical: 0
            })
        );
        assert_eq!(AdapterKey::parse("x_phys_0_eng_0_engtype_3D"), None);
        assert_eq!(AdapterKey::parse("0x0_0x1_phys_99_eng_0_engtype_3D"), None);
    }
    #[test]
    fn temperature_uses_documented_tenths_and_rejects_missing_or_abnormal_data() {
        assert_eq!(temperature(435), Some(43.5));
        assert_eq!(temperature(0), None);
        assert_eq!(temperature(u32::MAX), None);
    }
}
