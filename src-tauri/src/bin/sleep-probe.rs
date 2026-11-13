#[tokio::main]
async fn main() {
    let root = std::env::current_dir().expect("工作目录不可读");
    let path = root
        .join("artifacts")
        .join(format!("g2-sleep-recovery-{}.json", uuid::Uuid::new_v4()));
    println!("报告：{}", path.display());
    println!("先等待行情准备就绪；每轮按提示手动选择Windows睡眠并唤醒。探针不会自动休眠，不修改电源设置。");
    if let Err(e) = souldesk_lib::run_sleep_probe(path).await {
        eprintln!("休眠检查失败：{e}");
        std::process::exit(1);
    }
    println!("5次休眠核心恢复检查完成。");
}
