//! uuengine —— UURelayBridge 的内核（CLI）。
//!
//! 用法：
//!   uuengine                          用同目录的 engine.yaml（没有就自动建）启动
//!   uuengine run [--config <path>]    同上，可指定配置文件
//!   uuengine check [--config <path>]  只校验配置
//!   uuengine print-config             打印一份带注释的默认配置
//!   uuengine version                  版本
//!
//! 默认配置就在可执行文件旁边：`engine.yaml`。不存在时自动创建一份，
//! 这样双击/命令行直接跑也有配置可用。

mod config;
mod service;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use config::{Config, FileConfig};

const DEFAULT_FILE: &str = "engine.yaml";

fn usage() {
    println!(
        "uuengine {}\n\
         用法:\n  \
         uuengine [run] [--config <engine.yaml>] [--mode host|socks] [--port N]\n  \
         uuengine check [--config <engine.yaml>]\n  \
         uuengine print-config\n  \
         uuengine version\n\
         \n\
         默认配置：{}（不存在会自动创建）\n",
        env!("CARGO_PKG_VERSION"),
        default_config_path().display()
    );
}

/// 默认配置：可执行文件同目录的 engine.yaml。
fn default_config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join(DEFAULT_FILE);
        }
    }
    PathBuf::from(DEFAULT_FILE)
}

fn template() -> String {
    let mut cfg = Config {
        kind: "socks".to_string(),
        domains: uurelaybridge_core::presets::uu_endpoints(),
        ..Default::default()
    };
    cfg.sanitize();
    format!(
        "# uuengine 配置（首次运行自动生成，可直接修改）\n\
         # 最少两行就够：mode + domains；其余都可以删。\n\
         # fragment: 4 表示 TLS 记录分 4 段；off 不分片；tcp:6 用 TCP 分段 6 段。\n\
         # dns 支持 223.5.5.5 或 https://dns.alidns.com/dns-query。\n{}",
        cfg.to_yaml()
    )
}

/// 确保配置文件存在；返回实际使用的路径。
fn ensure_config(path: &Path) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, template())
        .map_err(|e| format!("自动创建配置失败（{}）：{e}", path.display()))?;
    Ok(true)
}

fn resolve_config(arg: Option<String>) -> Result<(PathBuf, bool), String> {
    let path = match arg {
        Some(p) => PathBuf::from(p),
        None => default_config_path(),
    };
    let created = ensure_config(&path)?;
    Ok((path, created))
}

fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读取配置失败（{}）：{e}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    let fc: FileConfig = if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        serde_json::from_str(text).map_err(|e| format!("解析 JSON 配置失败：{e}"))?
    } else {
        serde_yaml::from_str(text).map_err(|e| format!("解析 YAML 配置失败：{e}"))?
    };
    Config::from_file(fc)
}

fn do_run(arg_config: Option<String>, mode: Option<String>, port: Option<u16>) -> ExitCode {
    let (path, created) = match resolve_config(arg_config) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR {e}");
            return ExitCode::from(1);
        }
    };
    if created {
        service::log(&format!("已创建默认配置：{}", path.display()));
    }
    let mut cfg = match load(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(m) = mode {
        cfg.kind = if m.eq_ignore_ascii_case("socks") {
            "socks".to_string()
        } else {
            "host".to_string()
        };
    }
    if let Some(p) = port {
        if cfg.kind == "socks" {
            cfg.socks_port = p;
        } else {
            cfg.listen_port = p;
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
        "配置：{}（v{} mode={} domains={} fragment={} dns={}）",
        path.display(),
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let get = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "" => {
            // 没参数：把默认配置准备好，然后直接启动
            do_run(None, None, None)
        }
        "version" | "--version" | "-V" => {
            println!("uuengine {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "print-config" => {
            print!("{}", template());
            ExitCode::SUCCESS
        }
        "check" => {
            let (path, created) = match resolve_config(get("--config")) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("ERROR {e}");
                    return ExitCode::from(1);
                }
            };
            if created {
                println!("已创建默认配置：{}", path.display());
            }
            match load(&path) {
                Ok(cfg) => {
                    let errs = cfg.validate();
                    if errs.is_empty() {
                        println!(
                            "OK {} mode={} domains={} fragment={} dns={}",
                            path.display(),
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
            let port = get("--port").and_then(|p| p.parse::<u16>().ok());
            do_run(get("--config"), get("--mode"), port)
        }
        "help" | "--help" | "-h" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("未知命令：{other}");
            usage();
            ExitCode::from(2)
        }
    }
}
