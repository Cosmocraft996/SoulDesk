#[tokio::main]
async fn main() {
    let root = std::env::current_dir().expect("工作目录不可读");
    let path = root.join("artifacts").join(format!(
        "g2-transport-recovery-{}.json",
        uuid::Uuid::new_v4()
    ));
    println!("报告：{}", path.display());
    if let Err(e) = souldesk_lib::run_recovery_probe(path).await {
        eprintln!("恢复检查失败：{e}");
        std::process::exit(1);
    }
}
