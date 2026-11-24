#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 提权子进程入口：`UURelayBridge.exe --hosts-apply|--hosts-remove|--hosts-restore <数据目录>`。
/// 只做一次 hosts 读写，不启动 GUI。
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args
        .iter()
        .position(|a| a == "--hosts-apply" || a == "--hosts-remove" || a == "--hosts-restore")
    {
        let op = match args[pos].as_str() {
            "--hosts-apply" => "apply",
            "--hosts-restore" => "restore",
            _ => "remove",
        };
        let data_dir = args.get(pos + 1).cloned().unwrap_or_default();
        let code = uurelaybridge_lib::hostmode::elevate::run_helper(op, &data_dir);
        std::process::exit(code);
    }
    if let Some(pos) = args.iter().position(|a| a == "--hosts-selftest") {
        let dir = args.get(pos + 1).cloned().unwrap_or_default();
        let code = uurelaybridge_lib::hostmode::selftest::run(&dir);
        std::process::exit(code);
    }

    uurelaybridge_lib::run()
}
