//! 显式长期验收：只记录本次隔离应用，不修改正常用户数据。
use crate::{
    store::{err, now_ms, AppResult},
    Core,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::Manager;

#[cfg(windows)]
fn process_metrics() -> AppResult<Value> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{
                K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
            },
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let process = unsafe { GetCurrentProcess() };
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(format!(
            "读取进程CPU失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
    memory.cb = std::mem::size_of_val(&memory) as u32;
    if unsafe {
        K32GetProcessMemoryInfo(
            process,
            &mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            memory.cb,
        )
    } == 0
    {
        return Err(format!(
            "读取进程内存失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
    Ok(
        json!({"cpuTicks":ticks(kernel)+ticks(user),"workingSetBytes":memory.WorkingSetSize,"privateBytes":memory.PrivateUsage,"pid":std::process::id()}),
    )
}
#[cfg(not(windows))]
fn process_metrics() -> AppResult<Value> {
    Err("当前平台未实现Windows资源采样".into())
}

pub async fn check(
    app: &tauri::AppHandle,
    path: &Path,
    report: &mut Value,
    seconds: u64,
) -> AppResult<()> {
    let started = Instant::now();
    // 保持一条真正启用的重复规则参与每笔Mark判断，避免长期检查只遍历已完成规则。
    let watching = crate::alerts::Rule {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 1,
        name: "长期采样等待侧规则".into(),
        key: crate::alerts::MarketKey::binance("BTCUSDT", crate::alerts::PriceKind::Mark),
        direction: crate::alerts::Direction::Up,
        threshold: "79228162514264337593543950335".into(),
        mode: crate::alerts::Mode::Repeat,
        cooldown_ms: 1000,
        enabled: true,
        notification: false,
        sound: false,
    };
    crate::alert_store::save_rule(&app.state::<Core>().store, &watching, None)?;
    let initial_events = crate::alert_store::list_events(&app.state::<Core>().store, 0, 100)?.len();
    let sample_path = path.with_extension("samples.jsonl");
    let mut samples = std::fs::OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&sample_path)
        .map_err(err)?;
    report["phase"] = json!("g3-soak");
    report["stage"] = json!("正在执行长期零视图后台检查");
    report["soakStarted"] = json!(now_ms());
    report["soakSeconds"] = json!(seconds);
    report["soakPassed"] = json!(false);
    report["full24Hours"] = json!(false);
    report["sampleFile"] = json!(sample_path);
    report["monitoringRuleId"] = json!(watching.id);
    report["scope"]=json!("发布资源应用；20个真实合约，零视图后台；不主动发送通知声音；CPU和内存只计Rust宿主进程，视图场景需另验");
    let warmup = (seconds / 4).min(300);
    let mut private = Vec::new();
    let mut working = Vec::new();
    let mut cpu = Vec::new();
    let mut previous: Option<(u64, u64)> = None;
    let mut health = HealthWindow::default();
    let mut count = 0u64;
    let mut previous_sample = Instant::now();
    let mut gaps = 0u64;
    loop {
        let core = app.state::<Core>();
        let elapsed = started.elapsed().as_secs();
        if count > 0 && previous_sample.elapsed() > Duration::from_secs(90) {
            gaps += 1;
        }
        previous_sample = Instant::now();
        let snap = core.business.feed.snapshot()?;
        let sessions = core.sessions.0.lock().map_err(err)?.len();
        let subscriptions = core.business.subscriptions.lock().map_err(err)?.len();
        let views = app.webview_windows().len();
        let tasks = core.window_tasks.load(Ordering::Relaxed);
        let trials = core.trials.lock().map_err(err)?.len();
        let mut metric = process_metrics()?;
        let mono = core.business.feed.origin.elapsed().as_millis() as u64;
        let ticks = metric["cpuTicks"].as_u64().ok_or("CPU采样无效")?;
        let percent = previous.and_then(|(at, before)| {
            let ms = mono.saturating_sub(at);
            (ms > 0)
                .then_some(ticks.saturating_sub(before) as f64 / (ms.max(1) as f64 * 10000.) * 100.)
        });
        previous = Some((mono, ticks));
        metric["cpuPercentOneCore"] = json!(percent);
        let fresh_marks = snap["quotes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|q| q["quality"] == "fresh" && q["key"]["priceKind"] == "mark")
            .count();
        let events = crate::alert_store::list_events(&core.store, 0, 100)?.len();
        let rule_state = crate::alert_store::list_rules(&core.store)?
            .into_iter()
            .find(|(r, _)| r.id == watching.id)
            .ok_or("长期检查的有效规则意外消失")?
            .1;
        if rule_state.sequence != 0
            || rule_state.completed
            || rule_state.pending
            || (elapsed >= 30 && (!rule_state.initialized || !rule_state.waiting_side))
        {
            return Err("长期采样的有效等待侧规则状态不符".into());
        }
        let healthy = snap["status"] == "connected" && fresh_marks == 20;
        health.observe(started.elapsed().as_millis() as u64, healthy)?;
        if sessions != 0
            || subscriptions != 0
            || views != 0
            || tasks != 0
            || trials != 0
            || snap["subscriptions"] != 62
            || snap["queueOverflows"] != 0
            || events != initial_events
        {
            return Err(format!("长期运行的资源或事件边界变化：session={sessions},temporary={subscriptions},views={views},tasks={tasks},trials={trials},streams={},overflows={},events={events}/{initial_events}",snap["subscriptions"],snap["queueOverflows"]));
        }
        if elapsed >= warmup {
            private.push(metric["privateBytes"].as_f64().ok_or("私有内存无效")?);
            working.push(metric["workingSetBytes"].as_f64().ok_or("工作集无效")?);
            if let Some(percent) = percent {
                cpu.push(percent);
            }
        }
        let row = json!({"time":now_ms(),"elapsedSeconds":elapsed,"process":metric,"sessions":sessions,"temporarySubscriptions":subscriptions,"views":views,"windowTasks":tasks,"trials":trials,"streamCount":snap["subscriptions"],"quoteCount":snap["quotes"].as_array().unwrap().len(),"tickerCount":snap["tickers"].as_object().unwrap().len(),"generation":snap["generation"],"queueOverflows":snap["queueOverflows"],"queueDepth":snap["queueDepth"],"received":snap["received"],"freshMarks":fresh_marks,"healthy":healthy,"events":events,"error":snap["error"]});
        writeln!(samples, "{row}").map_err(err)?;
        samples.flush().map_err(err)?;
        count += 1;
        report["latestSample"] = row;
        report["sampleCount"] = json!(count);
        report["elapsedSeconds"] = json!(elapsed);
        report["warmupSeconds"] = json!(warmup);
        report["samplingGapsOver90s"] = json!(gaps);
        report["healthObservationGapsOver3s"] = json!(health.gaps);
        report["privateBytesSummary"] = summary(&private);
        report["workingSetSummary"] = summary(&working);
        report["cpuPercentOneCoreSummary"] = summary(&cpu);
        std::fs::write(path, serde_json::to_vec_pretty(report).map_err(err)?).map_err(err)?;
        if elapsed >= seconds {
            break;
        }
        let next = Duration::from_secs((elapsed + 30).min(seconds));
        while started.elapsed() < next {
            tokio::time::sleep(Duration::from_secs(1)).await;
            health.observe(
                started.elapsed().as_millis() as u64,
                core.business.feed.fresh_mark_count()? == 20,
            )?;
        }
    }
    let growth = report["privateBytesSummary"]["growth"]
        .as_f64()
        .unwrap_or_default();
    let initial = private.first().copied().unwrap_or_default();
    let review =
        (growth > 64. * 1024. * 1024. && growth > initial * 0.25) || gaps > 0 || health.gaps > 0;
    report["resourceReviewRequired"] = json!(review);
    report["full24Hours"] =
        json!(seconds == 86400 && started.elapsed() >= Duration::from_secs(86400));
    report["soakPassed"] = json!(seconds == 86400 && !review);
    report["shortCheckPassed"] = json!(seconds < 86400 && !review);
    report["stage"] = json!(if review {
        "长期检查采样完成；内存增长需定位复核"
    } else if seconds == 86400 {
        "24小时零视图检查完成；首版其他验收项仍需核对"
    } else {
        "短时采样检查完成；不能作为24小时验收"
    });
    report["soakFinished"] = json!(now_ms());
    Ok(())
}
#[derive(Default)]
struct HealthWindow {
    since: Option<u64>,
    previous: Option<u64>,
    gaps: u64,
}
impl HealthWindow {
    fn observe(&mut self, now: u64, healthy: bool) -> AppResult<()> {
        if self
            .previous
            .is_some_and(|before| now.saturating_sub(before) > 3000)
        {
            self.gaps += 1;
        }
        self.previous = Some(now);
        if self
            .since
            .is_some_and(|before| now.saturating_sub(before) > 60000)
        {
            return Err("60秒内必要Mark流未恢复；即使当前已恢复也不能抹去超时记录".into());
        }
        if healthy {
            self.since = None;
        } else if self.since.is_none() {
            self.since = Some(now);
        }
        Ok(())
    }
}
fn summary(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = if sorted.len() % 2 == 0 {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.
    } else {
        sorted[sorted.len() / 2]
    };
    json!({"count":values.len(),"min":sorted[0],"median":median,"max":sorted[sorted.len()-1],"first":values[0],"last":values[values.len()-1],"growth":values[values.len()-1]-values[0],"perSampleSlope":if values.len()>1{(values[values.len()-1]-values[0])/(values.len()-1) as f64}else{0.}})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returning_to_healthy_cannot_erase_an_overdue_outage() {
        let mut guard = HealthWindow::default();
        assert!(guard.observe(0, true).is_ok());
        assert!(guard.observe(1000, false).is_ok());
        assert!(guard.observe(60000, false).is_ok());
        assert!(guard.observe(76000, true).is_err());
    }
    #[test]
    fn even_sample_median_and_health_observation_gaps_are_explicit() {
        assert_eq!(summary(&[1., 3., 7., 9.])["median"], 5.);
        let mut guard = HealthWindow::default();
        guard.observe(0, true).unwrap();
        guard.observe(4000, true).unwrap();
        assert_eq!(guard.gaps, 1);
    }
}
