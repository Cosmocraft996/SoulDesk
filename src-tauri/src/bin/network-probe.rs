use souldesk_lib::network::{self, NetworkConfig};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match args.first().map(String::as_str) {
        Some("system") => network::system_proxy().expect("系统代理解析失败"),
        Some("http") | Some("socks5") => {
            let host = args.get(1).expect("需要代理主机").clone();
            let port = args.get(2).expect("需要端口").parse().expect("端口无效");
            if args[0] == "http" {
                NetworkConfig::Http { host, port }
            } else {
                NetworkConfig::Socks5 { host, port }
            }
        }
        _ => NetworkConfig::Direct,
    };
    let token = CancellationToken::new();
    // 固定公开端点；不接收账户或凭证。
    let rest = network::http_get(&config, "https://fapi.binance.com/fapi/v1/time", &token).await;
    let ws = network::first_ws(
        &config,
        "wss://fstream.binance.com/market/ws/btcusdt@markPrice@1s",
        &token,
    )
    .await;
    let result = match (rest, ws) {
        (Ok(rest), Ok(ws)) => network::validate_probe_payloads(&rest, &ws),
        (rest, ws) => Err(format!("REST={:?}; WS={:?}", rest.err(), ws.err())),
    };
    println!("{}", serde_json::json!({"result":result}));
    if result.is_err() {
        std::process::exit(1);
    }
}
