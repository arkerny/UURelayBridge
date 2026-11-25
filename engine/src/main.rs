//! uuengine —— UURelayBridge 的内核（CLI）。
//!
//! 用法：
//!   uuengine run --config <path>     按配置启动（kind = host | socks）
//!   uuengine check --config <path>   只校验配置，打印结果后退出
//!   uuengine print-config            打印一份默认配置样例
//!   uuengine version                 版本
//!
//! 内核只做数据面：监听 / 解析 ClientHello / 按域名分片 / 转发。
//! 修改 hosts、开关服务、UI 都由前端负责，两者通过配置文件与进程生命周期解耦。

mod config;
mod service;

use std::process::ExitCode;

use config::Config;

fn usage() {
    println!(
        "uuengine {}\n\
         用法:\n  \
         uuengine run --config <path> [--kind host|socks] [--port N]\n  \
         uuengine check --config <path>\n  \
         uuengine print-config\n  \
         uuengine version\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn load(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读取配置失败（{path}）：{e}"))?;
    let mut cfg: Config =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| {
            format!("解析配置失败（{path}）：{e}")
        })?;
    cfg.sanitize();
    Ok(cfg)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        return ExitCode::from(2);
    }

    let get = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    match args[0].as_str() {
        "version" | "--version" | "-V" => {
            println!("uuengine {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "print-config" => {
            let cfg = Config {
                kind: "host".to_string(),
                domains: uurelaybridge_core::presets::uu_endpoints(),
                ..Default::default()
            };
            println!("{}", serde_json::to_string_pretty(&cfg).unwrap_or_default());
            ExitCode::SUCCESS
        }
        "check" => {
            let Some(path) = get("--config") else {
                eprintln!("缺少 --config");
                return ExitCode::from(2);
            };
            match load(&path) {
                Ok(cfg) => {
                    let errs = cfg.validate();
                    if errs.is_empty() {
                        println!(
                            "OK kind={} domains={} mode={} parts={} dns={}",
                            cfg.kind,
                            cfg.domains.len(),
                            cfg.fragment_mode,
                            cfg.fragment_parts,
                            cfg.dns_server
                        );
                        ExitCode::SUCCESS
                    } else {
                        for e in errs {
                            eprintln!("ERROR {e}");
                        }
                        ExitCode::from(1)
                    }
                }
                Err(e) => {
                    eprintln!("ERROR {e}");
                    ExitCode::from(1)
                }
            }
        }
        "run" => {
            let Some(path) = get("--config") else {
                eprintln!("缺少 --config");
                return ExitCode::from(2);
            };
            let mut cfg = match load(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("ERROR {e}");
                    return ExitCode::from(1);
                }
            };
            if let Some(kind) = get("--kind") {
                cfg.kind = kind;
            }
            if let Some(port) = get("--port").and_then(|p| p.parse::<u16>().ok()) {
                if cfg.kind == "socks" {
                    cfg.socks_port = port;
                } else {
                    cfg.listen_port = port;
                }
            }
            let errs = cfg.validate();
            if !errs.is_empty() {
                for e in errs {
                    eprintln!("ERROR {e}");
                }
                return ExitCode::from(1);
            }
            service::log(&format!(
                "启动 v{} kind={} domains={} mode={}x{} dns={}",
                env!("CARGO_PKG_VERSION"),
                cfg.kind,
                cfg.domains.len(),
                cfg.fragment_mode,
                cfg.fragment_parts,
                cfg.dns_server
            ));
            match service::run(cfg) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    service::log(&format!("启动失败：{e}"));
                    ExitCode::from(1)
                }
            }
        }
        other => {
            eprintln!("未知命令：{other}");
            usage();
            ExitCode::from(2)
        }
    }
}
