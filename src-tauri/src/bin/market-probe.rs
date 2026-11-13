use serde_json::{json, Value};
use souldesk_lib::{
    market,
    network::{self, NetworkConfig},
};
use std::{collections::HashSet, time::Duration};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let result = probe().await;
    match result {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(e) => {
            eprintln!("行情探针失败：{e}");
            std::process::exit(1);
        }
    }
}
async fn probe() -> Result<Value, String> {
    let config = if std::env::args().any(|a| a == "direct") {
        NetworkConfig::Direct
    } else {
        network::system_proxy()?
    };
    let cancel = CancellationToken::new();
    let catalog = market::parse_catalog(
        &network::http_get(
            &config,
            "https://fapi.binance.com/fapi/v1/exchangeInfo",
            &cancel,
        )
        .await?,
    )?;
    let mut symbols = Vec::new();
    for s in [
        "BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT", "XRPUSDT", "DOGEUSDT", "ADAUSDT", "LINKUSDT",
        "AVAXUSDT", "LTCUSDT", "BCHUSDT", "DOTUSDT", "SUIUSDT", "TRXUSDT", "NEARUSDT", "UNIUSDT",
        "AAVEUSDT", "ETCUSDT", "FILUSDT",
    ] {
        if catalog
            .iter()
            .any(|c| c.symbol == s && c.status == "TRADING")
        {
            symbols.push(s.to_owned());
        }
    }
    if let Some(c) = catalog
        .iter()
        .find(|c| c.contract_type == "TRADIFI_PERPETUAL" && c.status == "TRADING")
    {
        symbols.push(c.symbol.clone());
    }
    for c in &catalog {
        if symbols.len() >= 20 {
            break;
        }
        if c.status == "TRADING" && !symbols.contains(&c.symbol) {
            symbols.push(c.symbol.clone());
        }
    }
    symbols.truncate(20);
    if symbols.len() != 20 {
        return Err("不足20个有效合约".into());
    }
    let keys = symbols
        .iter()
        .flat_map(|s| {
            [
                souldesk_lib::alerts::MarketKey::binance(s, souldesk_lib::alerts::PriceKind::Last),
                souldesk_lib::alerts::MarketKey::binance(s, souldesk_lib::alerts::PriceKind::Mark),
            ]
        })
        .collect::<HashSet<_>>();
    let (feed, rx, tx, mut input) = market::Feed::new(config);
    feed.desired.send_modify(|d| d.keys = keys.clone());
    let runner = tokio::spawn(market::run(feed.clone(), rx, tx));
    let mut seen = HashSet::new();
    let mut samples = Vec::new();
    let result=tokio::time::timeout(Duration::from_secs(100),async{while let Some(input)=input.recv().await{if seen.insert(input.observation.key.clone()){samples.push(json!({"key":input.observation.key,"price":input.observation.price,"sourceUtcMs":input.observation.source_utc_ms,"sourceNow":input.context.source_now_utc_ms,"generation":input.observation.generation}));}if seen.len()==40{break;}}}).await;
    let before = feed.snapshot()?;
    let mut recovered = false;
    if result.is_ok() && seen.len() == 40 {
        let old = feed.generation.load(std::sync::atomic::Ordering::SeqCst);
        feed.desired.send_modify(|d| d.reconnect += 1);
        recovered = tokio::time::timeout(Duration::from_secs(45), async {
            while let Some(input) = input.recv().await {
                if input.observation.generation > old && input.observation.recovered {
                    return;
                }
            }
        })
        .await
        .is_ok();
    }
    feed.cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), runner)
        .await
        .map_err(|_| "取消连接超时")?
        .map_err(|e| e.to_string())?;
    let complete = seen.len() == 40 && recovered;
    let report = json!({"phase":"live-market-probe","passed":complete,"symbols":symbols,"observedKeys":seen.len(),"catalogCount":catalog.len(),"tradfi":catalog.iter().filter(|c|symbols.contains(&c.symbol)&&c.contract_type=="TRADIFI_PERPETUAL").collect::<Vec<_>>(),"samples":samples,"snapshot":before,"recovered":recovered,"cancelled":true});
    println!("{}", serde_json::to_string(&report).unwrap());
    if !complete {
        return Err(format!(
            "20合约验证未完成，实际{}/40价格键，恢复={recovered}，状态={}",
            seen.len(),
            report["snapshot"]["error"]
        ));
    }
    Ok(report)
}
