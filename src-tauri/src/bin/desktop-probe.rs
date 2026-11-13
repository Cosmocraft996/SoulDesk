#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let widgets = args.iter().any(|arg| arg == "--widgets");
    args.retain(|arg| arg != "--widgets");
    let report = match args.as_slice() {
        [] => match std::env::current_dir() {
            Ok(root) => root
                .join("artifacts")
                .join(format!("desktop-probe-{}.json", uuid::Uuid::new_v4())),
            Err(e) => {
                eprintln!("工作目录不可读：{e}");
                std::process::exit(1);
            }
        },
        [flag, path] if flag == "--report" => std::path::PathBuf::from(path),
        _ => {
            eprintln!("用法：desktop-probe [--widgets] [--report 绝对路径.json]");
            std::process::exit(1);
        }
    };
    println!("隔离探针报告：{}", report.display());
    let result = if widgets {
        match std::env::current_dir() {
            Ok(root) => souldesk_lib::run_desktop_widget_probe(
                report,
                [
                    root.join("artifacts/local.example.clock-0.1.1.zip"),
                    root.join("artifacts/local.example.card-0.1.0.zip"),
                ],
            ),
            Err(error) => Err(error.to_string()),
        }
    } else {
        souldesk_lib::run_desktop_probe(report)
    };
    if let Err(error) = result {
        eprintln!("桌面探针失败：{error}");
        std::process::exit(1);
    }
}
