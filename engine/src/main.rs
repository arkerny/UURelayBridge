//! uuengine —— UURelayBridge 的内核（CLI）。
//!
//! 用法：
//!   uuengine run --config <path>     按配置启动（mode = socks | host）
//!   uuengine check --config <path>   只校验配置
//!   uuengine print-config            打印一份带注释的默认配置
//!   uuengine version                 版本
//!
//! 配置默认是 YAML（`.yaml` / `.yml`）；`.json` 仍然能读，方便老脚本。

mod config;
mod service;

use std::process::ExitCode;

use config::{Config, FileConfig};

fn usage() {
    println!(
        "uuengine {}\n\
         用法:\n  \
         uuengine run --config <engine.yaml> [--mode host|socks] [--port N]\n  \
         uuengine check --config <engine.yaml>\n  \
         uuengine print-config\n  \
         uuengine version\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn load(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取配置失败（{path}）：{e}"))?;
    let text = text.trim_start_matches('\u{feff}');
    let fc: FileConfig = if path.to_ascii_lowercase().ends_with(".json") {
        serde_json::from_str(text).map_err(|e| format!("解析 JSON 配置失败：{e}"))?
    } else {
        serde_yaml::from_str(text).map_err(|e| format!("解析 YAML 配置失败：{e}"))?
    };
    Config::from_file(fc)
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
            let mut cfg = Config {
                kind: "socks".to_string(),
                domains: uurelaybridge_core::presets::uu_endpoints(),
                ..Default::default()
            };
            cfg.sanitize();
            println!("# 最少只要 mode + domains 两行；下面这些都可以删。");
            println!("# fragment: 4 表示 TLS 记录分 4 段；off 不分片；tcp:6 用 TCP 分段 6 段。");
            println!("# dns 支持 223.5.5.5 或 https://dns.alidns.com/dns-query。");
            print!("{}", cfg.to_yaml());
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
                            "OK mode={} domains={} fragment={} dns={}",
                            cfg.kind,
                            cfg.domains.len(),
                            cfg.fragment.as_yaml(),
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
            if let Some(mode) = get("--mode") {
                cfg.kind = if mode.eq_ignore_ascii_case("socks") {
                    "socks".to_string()
                } else {
                    "host".to_string()
                };
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
                "启动 v{} mode={} domains={} fragment={} dns={}",
                env!("CARGO_PKG_VERSION"),
                cfg.kind,
                cfg.domains.len(),
                cfg.fragment.as_yaml(),
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
