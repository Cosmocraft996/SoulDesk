use super::import::{segment, ssh_tool, ConnectionInfo, Server};
use crate::store::AppResult;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};

const LIMIT: usize = 4 * 1024 * 1024;
const COLLECTOR: &str = include_str!("collector.py");

pub(super) fn stamp(value: &Value) -> Option<f64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|time| time.timestamp_millis() as f64 / 1000.)
}

pub(super) fn now() -> f64 {
    Utc::now().timestamp_millis() as f64 / 1000.
}

pub(super) fn public_fields(value: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .map(|&key| (key.to_owned(), value[key].clone()))
            .collect(),
    )
}

fn percent(used: &Value, total: &Value) -> Option<f64> {
    let total = total.as_f64()?;
    (total > 0.).then_some(used.as_f64()? / total * 100.)
}

pub(super) fn client() -> AppResult<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "无法初始化 Instabox 连接".into())
}

struct Instabox<'a> {
    client: &'a Client,
    id: &'a str,
    org: &'a str,
    key: &'a str,
}

impl Instabox<'_> {
    async fn request(&self, url: String, action: Option<&str>) -> AppResult<Value> {
        let mut key =
            reqwest::header::HeaderValue::from_str(self.key).map_err(|_| "Instabox 凭据无效")?;
        key.set_sensitive(true);
        let method = if action.is_some() {
            Method::POST
        } else {
            Method::GET
        };
        let mut request = self
            .client
            .request(method, format!("https://api.instabox.sh/v1/{url}"))
            .header("X-API-Key", key)
            .header("Accept", "application/json");
        if let Some(id) = action {
            request = request.header("Idempotency-Key", id).json(&json!({}));
        }
        let mut response = request.send().await.map_err(|_| "无法连接 Instabox 接口")?;
        if !response.status().is_success() {
            return Err(format!(
                "Instabox 接口返回 HTTP {}",
                response.status().as_u16()
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Instabox 响应中断或超时")?
        {
            if chunk.len() > LIMIT.saturating_sub(body.len()) {
                return Err("Instabox 响应超过限制".into());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| "Instabox 响应格式无效".into())
    }

    async fn machine(&self) -> AppResult<Value> {
        let machine = self.request(format!("machines/{}", self.id), None).await?;
        validate_machine(&machine, self.id, self.org)?;
        Ok(machine)
    }

    async fn metrics(&self, first: bool) -> AppResult<Value> {
        let since = Utc::now() - ChronoDuration::seconds(if first { 86300 } else { 300 });
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("since", &since.to_rfc3339())
            .finish();
        self.request(format!("machines/{}/metrics?{query}", self.id), None)
            .await
    }
}

fn validate_machine(value: &Value, machine: &str, org: &str) -> AppResult<()> {
    if value["id"].as_str() != Some(machine) || value["orgId"].as_str() != Some(org) {
        return Err("机器或工作区与原配置不一致".into());
    }
    if value["state"].as_str().is_none_or(|state| !segment(state)) {
        return Err("平台实例状态无效".into());
    }
    Ok(())
}

pub(super) fn wake_action(machine: &Value, at: f64) -> AppResult<Option<&'static str>> {
    if machine["state"] == "running" {
        return Ok(None);
    }
    if !machine["operationId"].is_null() {
        return Err("平台已有操作正在执行".into());
    }
    let action = match machine["state"].as_str() {
        Some("suspended") => "resume",
        Some("stopped") => "start",
        _ => return Err("当前实例状态不支持唤醒".into()),
    };
    if stamp(&machine["cooldownUntil"]).is_some_and(|expires| expires > at) {
        return Err("实例处于平台冷却期".into());
    }
    Ok(Some(action))
}

fn metrics_row(row: &Value) -> Value {
    json!({"measured_at":row["timestamp"],"cpu_pct":row["cpuUsedPct"],"vcpus":row["cpuCount"],
        "memory_used":row["memUsed"],"memory_total":row["memTotal"],"memory_pct":percent(&row["memUsed"],&row["memTotal"]),
        "net_in":row["netInBytesPerSec"],"net_out":row["netOutBytesPerSec"],"disk_read":row["diskReadBytesPerSec"],
        "disk_write":row["diskWriteBytesPerSec"],"disk_pct":percent(&row["diskUsed"],&row["diskTotal"]),
        "memory_method":"宿主机统计的已触碰内存"})
}

fn apply_disk(current: &mut Value, raw: &Value) {
    let Some(filesystems) = raw["filesystems"].as_array() else {
        return;
    };
    current["disks"] = Value::Array(filesystems.iter().take(128).map(|item| json!({
        "mount":item["mount"],"total":item["sizeBytes"],"used":item["usedBytes"],
        "available":item["availableBytes"],"used_pct":item["usedPct"],"level":item["level"],"inode_pct":item["inodesUsedPct"]
    })).collect());
    current["disk_measured_at"] = raw["measuredAt"].clone();
}

async fn collect_instabox(source: Instabox<'_>, current: &mut Value) -> AppResult<Vec<Value>> {
    let machine = source.machine().await?;
    current["status"] = machine["state"].clone();
    current["spec"] = public_fields(&machine["spec"], &["vcpus", "memMiB", "diskGiB"]);
    current["template"] = machine["template"]["id"].clone();
    current["runtime_ready"] = machine["runtime"]["ready"].clone();
    current["limits"] = public_fields(
        &machine,
        &[
            "autoSuspendAfterSeconds",
            "maxRunMinutes",
            "runStartedAt",
            "cooldownUntil",
            "oomKills",
        ],
    );
    let first = stamp(&current["metrics"]["measured_at"]).is_none();
    let fetch_disk = async {
        if machine["state"] == "running" {
            Some(
                source
                    .request(format!("machines/{}/disk", source.id), None)
                    .await,
            )
        } else {
            None
        }
    };
    let fetch_usage = async {
        if stamp(&current["usage_checked_at"]).is_none_or(|time| now() - time > 600.) {
            Some(
                source
                    .request(format!("usage?org={}", source.org), None)
                    .await,
            )
        } else {
            None
        }
    };
    let operation = current["operation"].clone();
    let fetch_operation = async {
        if matches!(operation["state"].as_str(), Some("succeeded" | "failed")) {
            return None;
        }
        let id = operation["id"].as_str().filter(|id| segment(id))?;
        Some(source.request(format!("operations/{id}"), None).await)
    };
    let (metrics, disk, usage, op) = tokio::join!(
        source.metrics(first),
        fetch_disk,
        fetch_usage,
        fetch_operation
    );
    let mut warnings = Vec::new();
    let mut samples = Vec::new();
    match metrics {
        Ok(metrics) => {
            let rows = metrics["metrics"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            samples.extend(
                rows.iter()
                    .rev()
                    .take(2880)
                    .map(metrics_row)
                    .filter(|row| stamp(&row["measured_at"]).is_some()),
            );
            let latest = samples
                .iter()
                .max_by_key(|row| (stamp(&row["measured_at"]).unwrap_or_default() * 1000.) as i64);
            if let Some(latest) = latest {
                if stamp(&latest["measured_at"]) >= stamp(&current["metrics"]["measured_at"]) {
                    current["metrics"] = latest.clone();
                }
            }
            apply_disk(current, &metrics["disk"]);
        }
        Err(error) => warnings.push(format!("指标采集：{error}")),
    }
    if let Some(disk) = disk {
        match disk {
            Ok(value) => apply_disk(current, &value),
            Err(error) => warnings.push(format!("磁盘采集：{error}")),
        }
    }
    if let Some(usage) = usage {
        match usage {
            Ok(value) if value["org"]["id"].as_str() == Some(source.org) => {
                current["usage"] = public_fields(
                    &value,
                    &[
                        "machines",
                        "snapshots",
                        "allocatedDiskGiB",
                        "cost",
                        "limits",
                        "computedAt",
                    ],
                );
                current["usage_checked_at"] = json!(Utc::now().to_rfc3339());
            }
            Ok(_) => warnings.push("额度工作区与配置不一致".into()),
            Err(error) => warnings.push(format!("额度采集：{error}")),
        }
    }
    if let Some(op) = op {
        match op {
            Ok(value) if value["id"] == operation["id"] => {
                let mut value = public_fields(&value, &["id", "state", "phase"]);
                if value["state"] == "failed" {
                    value["error"] = json!("实例唤醒失败，请检查平台状态");
                }
                current["operation"] = value;
            }
            Ok(_) => warnings.push("平台操作标识不一致".into()),
            Err(error) => warnings.push(format!("唤醒进度：{error}")),
        }
    }
    current["warnings"] = json!(warnings);
    Ok(samples)
}

struct PrivateFile(PathBuf);

impl PrivateFile {
    fn create(root: &Path, bytes: &[u8]) -> AppResult<Self> {
        let path = root.join(format!("cloud-ssh-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = owner_only_file(&path)?;
        let path = Self(path);
        let written = file.write_all(bytes);
        drop(file);
        written.map_err(|_| "无法写入当前用户的临时 SSH 凭据")?;
        Ok(path)
    }
}

impl Drop for PrivateFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(windows)]
fn owner_only_file(path: &Path) -> AppResult<std::fs::File> {
    use std::{
        mem::{size_of, zeroed},
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GENERIC_ALL, GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Security::{
            AddAccessAllowedAce, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
            SetSecurityDescriptorControl, SetSecurityDescriptorDacl, SetSecurityDescriptorOwner,
            TokenUser, ACL, ACL_REVISION, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
            SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
        },
        Storage::FileSystem::{CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err("无法确认 Windows 凭据所有者".into());
    }
    let mut required = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required);
    }
    let mut token_user = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            token_user.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    if ok == 0 || (required as usize) < size_of::<TOKEN_USER>() {
        return Err("无法确认 Windows 凭据所有者".into());
    }
    let sid = unsafe { (*(token_user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut acl_buffer = [0u64; 32];
    let acl = acl_buffer.as_mut_ptr().cast::<ACL>();
    let mut sd: SECURITY_DESCRIPTOR = unsafe { zeroed() };
    let pointer = (&mut sd as *mut SECURITY_DESCRIPTOR).cast();
    if unsafe {
        InitializeAcl(acl, size_of::<[u64; 32]>() as u32, ACL_REVISION) == 0
            || AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_ALL, sid) == 0
            || InitializeSecurityDescriptor(pointer, 1) == 0
            || SetSecurityDescriptorOwner(pointer, sid, 0) == 0
            || SetSecurityDescriptorDacl(pointer, 1, acl, 0) == 0
            || SetSecurityDescriptorControl(pointer, SE_DACL_PROTECTED, SE_DACL_PROTECTED) == 0
    } {
        return Err("无法限制 SSH 临时凭据访问权限".into());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: pointer,
        bInheritHandle: 0,
    };
    let name: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_TEMPORARY,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle == null::<core::ffi::c_void>() as *mut _ {
        return Err("无法创建仅当前用户可读的 SSH 临时文件".into());
    }
    Ok(unsafe { std::fs::File::from_raw_handle(handle) })
}

#[cfg(not(windows))]
fn owner_only_file(_path: &Path) -> AppResult<std::fs::File> {
    Err("SSH 凭据执行当前仅支持 Windows".into())
}

async fn collect_aliyun(
    server: &Server,
    root: &Path,
    current: &mut Value,
    interval: u64,
) -> AppResult<Vec<Value>> {
    let ConnectionInfo::Aliyun {
        host,
        user,
        port,
        key,
        pinned,
        services,
        health_urls,
    } = &server.connection
    else {
        return Err("服务器连接类型无效".into());
    };
    let key = PrivateFile::create(root, key.as_bytes())?;
    let pinned = PrivateFile::create(root, pinned.as_bytes())?;
    let options = STANDARD.encode(
        serde_json::to_vec(&json!({"services":services,"health_urls":health_urls}))
            .map_err(|_| "服务检查配置无效")?,
    );
    let mut command = Command::new(ssh_tool("ssh")?);
    command
        .args([
            "-F",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
            "-q",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "UpdateHostKeys=no",
            "-o",
            "GlobalKnownHostsFile=none",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
        ])
        .arg(format!(
            "UserKnownHostsFile=\"{}\"",
            pinned.0.to_string_lossy().replace('\\', "/")
        ))
        .arg("-i")
        .arg(&key.0)
        .args([
            "-p",
            &port.to_string(),
            "-l",
            user,
            "--",
            host,
            "python3",
            "-",
            &options,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|_| "无法启动系统 OpenSSH 客户端")?;
    let mut stdin = child.stdin.take().ok_or("无法写入只读采集程序")?;
    let stdout = child.stdout.take().ok_or("无法读取 SSH 采集结果")?;
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(40), async {
        stdin
            .write_all(COLLECTOR.as_bytes())
            .await
            .map_err(|_| "SSH 连接或采集程序写入失败")?;
        drop(stdin);
        stdout
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "无法读取 SSH 采集结果")?;
        if bytes.len() > LIMIT {
            return Err("SSH 采集结果超限");
        }
        if !child
            .wait()
            .await
            .map_err(|_| "SSH 客户端执行失败")?
            .success()
        {
            return Err("SSH 采集失败，请检查密钥、已信任主机或远端采集环境");
        }
        Ok(())
    })
    .await
    .map_err(|_| "SSH 采集超时")??;
    let mut data: Value = serde_json::from_slice(&bytes).map_err(|_| "SSH 返回了无效采集结果")?;
    let timestamp = stamp(&data["measured_at"]).ok_or("SSH 采集时间无效")?;
    let previous = &current["metrics"];
    let duration = stamp(&previous["measured_at"]).map(|time| timestamp - time);
    for (field, total) in [("net_in", "net_rx_total"), ("net_out", "net_tx_total")] {
        let rate = duration
            .filter(|duration| *duration > 0. && *duration <= (interval * 3) as f64)
            .filter(|_| !data["boot_id"].is_null() && data["boot_id"] == previous["boot_id"])
            .and_then(|duration| {
                let delta = data[total].as_f64()? - previous[total].as_f64()?;
                (delta >= 0.).then_some(delta / duration)
            });
        data[field] = json!(rate);
    }
    let Some(map) = data.as_object_mut() else {
        return Err("SSH 采集数据结构无效".into());
    };
    for key in [
        "services",
        "health",
        "containers",
        "tasks",
        "containers_available",
        "disks",
    ] {
        current[key] = map.remove(key).unwrap_or(Value::Null);
    }
    let disk = current["disks"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row["used_pct"].as_f64())
        .reduce(f64::max);
    data["disk_pct"] = json!(disk);
    data["memory_method"] = json!("总内存减去可用内存");
    current["disk_measured_at"] = data["measured_at"].clone();
    current["spec"] =
        json!({"vcpus":data["vcpus"],"memMiB":data["memory_total"].as_f64().map(|v| v / 1048576.)});
    current["status"] = json!("running");
    current["metrics"] = data.clone();
    Ok(vec![data])
}

pub(super) async fn collect(
    client: &Client,
    server: &Server,
    root: &Path,
    current: &mut Value,
    interval: u64,
) -> AppResult<Vec<Value>> {
    match &server.connection {
        ConnectionInfo::Aliyun { .. } => collect_aliyun(server, root, current, interval).await,
        ConnectionInfo::Instabox {
            machine_id,
            org_id,
            key,
            ..
        } => {
            collect_instabox(
                Instabox {
                    client,
                    id: machine_id,
                    org: org_id,
                    key,
                },
                current,
            )
            .await
        }
    }
}

pub(super) async fn wake(
    client: &Client,
    server: &Server,
    request: &str,
) -> AppResult<Option<(String, Value)>> {
    let ConnectionInfo::Instabox {
        machine_id,
        org_id,
        key,
        ..
    } = &server.connection
    else {
        return Err("只有 Instabox 实例支持手动唤醒".into());
    };
    let source = Instabox {
        client,
        id: machine_id,
        org: org_id,
        key,
    };
    let machine = source.machine().await?;
    let Some(action) = wake_action(&machine, now())? else {
        return Ok(None);
    };
    let operation = source
        .request(format!("machines/{machine_id}/{action}"), Some(request))
        .await
        .map_err(|_| "唤醒提交结果未确认，请刷新平台状态后再决定")?;
    if operation["id"].as_str().is_none_or(|id| !segment(id)) {
        return Err("唤醒响应缺少有效操作标识，请刷新确认".into());
    }
    Ok(Some((
        if action == "resume" {
            "resuming"
        } else {
            "starting"
        }
        .into(),
        public_fields(&operation, &["id", "state", "phase"]),
    )))
}
