//! 前端命令层：所有落盘动作都在这里，并且必须先拿到 approved 授权。

use std::sync::MutexGuard;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use uurelaybridge_core::adapters::{self, RenderInput};
use uurelaybridge_core::compiler;
use uurelaybridge_core::exporter::{self, manifest::Manifest};
use uurelaybridge_core::model::{Project, MODEL_SCHEMA_VERSION};
use uurelaybridge_core::validator::{self, has_errors, Issue};

use crate::platform;
use crate::state::{AppPaths, LastRun, LogEntry, Runtime as AppRuntime, Settings};
use crate::tray;

pub const STATE_EVENT: &str = "rb://state";
pub const NAV_EVENT: &str = "rb://navigate";

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub kind: String,
    pub message: String,
    pub issues: Vec<Issue>,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)?;
        // 附带校验明细：前端即使只拿到字符串，也能看出哪条规则错了
        if !self.issues.is_empty() {
            let detail: Vec<String> = self
                .issues
                .iter()
                .filter(|i| matches!(i.severity, uurelaybridge_core::validator::Severity::Error))
                .map(|i| i.message.clone())
                .collect();
            if !detail.is_empty() {
                write!(f, "：{}", detail.join("；"))?;
            }
        }
        Ok(())
    }
}


impl CommandError {
    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            issues: Vec::new(),
        }
    }
    fn with_issues(kind: &str, message: impl Into<String>, issues: Vec<Issue>) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            issues,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PathsView {
    pub data_dir: String,
    pub output_dir: String,
    pub script: String,
    pub manifest: String,
    pub backups_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub app_version: String,
    pub platform: String,
    pub settings: Settings,
    pub project: Project,
    pub last: Option<LastRun>,
    pub issues: Vec<String>,
    pub paths: PathsView,
    /// 当前设置下"如果现在生成"会得到的脚本内容（不落盘）。
    pub preview: String,
    pub validation: Vec<Issue>,
    pub compiled_rules: usize,
    pub manifest: Option<Manifest>,
    pub service: crate::hostmode::ServiceStatus,
    pub host_config: crate::hostmode::HostConfig,
    pub backups: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PreviewOut {
    pub text: String,
    pub validation: Vec<Issue>,
    pub compiled_rules: usize,
    pub skipped: Vec<String>,
}

// ---------------------------------------------------------------- 内部实现

fn target_file_name(target: &str) -> Option<String> {
    adapters::by_id(target).map(|a| a.file_name().to_string())
}

fn paths_view(rt: &AppRuntime, target: &str) -> PathsView {
    let file = target_file_name(target).unwrap_or_else(|| "relaybridge.js".to_string());
    PathsView {
        data_dir: rt.paths.root.display().to_string(),
        output_dir: rt.paths.output_dir(target).display().to_string(),
        script: rt.paths.script(target, &file).display().to_string(),
        manifest: rt.paths.manifest(target).display().to_string(),
        backups_dir: rt.paths.backups_dir().display().to_string(),
    }
}

/// 从启用的规则里收集域名（去重、保持顺序）。
/// 域名只由「规则」管理，配置页不再单独维护一份。
pub fn rule_domains(project: &Project) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for rule in &project.rules {
        if !rule.enabled {
            continue;
        }
        for d in &rule.match_.domain {
            let d = d.trim().trim_end_matches('.').to_ascii_lowercase();
            if !d.is_empty() && !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// SNI 联动：把 Host 模式里的域名/参数转成适配器能用的形式。
/// Clash 模式走本机 SOCKS5；Host 模式才用 dns.hosts 指到 443。
pub fn sni_link(rt: &AppRuntime) -> adapters::SniLink {
    let h = rt.host_config();
    let mode = rt.settings().mode;
    // 域名以规则为准
    let domains = rule_domains(&rt.project());
    adapters::SniLink {
        enabled: h.link_clash && !domains.is_empty(),
        domains,
        mode: h.fragment_mode.clone(),
        parts: h.fragment_parts,
        socks_port: if mode == "host" { None } else { Some(h.socks_port) },
        socks_name: "UURelayBridge-SNI".to_string(),
    }
}

fn render_preview(
    project: &Project,
    enabled: bool,
    target: &str,
    sni: &adapters::SniLink,
) -> (String, usize, Vec<String>) {
    let Some(adapter) = adapters::by_id(target) else {
        return (String::new(), 0, Vec::new());
    };
    let compiled = compiler::compile(project);
    let text = adapter
        .render(&RenderInput {
            app_version: platform::app_version(),
            project_name: &project.name,
            enabled,
            compiled: &compiled,
            generated_at: &crate::state::now_rfc3339(),
            sni: Some(sni),
        })
        .unwrap_or_default();
    (
        text,
        compiled.rules.len(),
        compiled
            .skipped
            .iter()
            .map(|s| format!("{}（{}）：{}", s.rule_name, s.rule_id, s.reason))
            .collect(),
    )
}

fn build_snapshot(rt: &AppRuntime) -> Snapshot {
    let settings = rt.settings();
    let project = rt.project();
    let validation = validator::validate(&project);
    let sni = sni_link(rt);
    let (preview, compiled_rules, _) = render_preview(&project, settings.enabled, &settings.target, &sni);
    let paths = paths_view(rt, &settings.target);
    let manifest = exporter::manifest::read(&rt.paths.manifest(&settings.target));
    let backups = exporter::backup::list(&rt.paths.backups_dir())
        .map(|v| {
            v.into_iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .take(8)
                .collect()
        })
        .unwrap_or_default();

    Snapshot {
        app_version: platform::app_version().to_string(),
        platform: platform::platform_name().to_string(),
        settings,
        project,
        last: rt.last(),
        issues: rt.issues(),
        paths,
        preview,
        validation,
        compiled_rules,
        manifest,
        service: crate::hostmode::status(),
        host_config: rt.host_config(),
        backups,
    }
}

pub fn emit_state(app: &AppHandle) {
    let rt = app.state::<AppRuntime>();
    let snap = build_snapshot(&rt);
    let _ = app.emit(STATE_EVENT, snap);
}

fn persist_settings(rt: &AppRuntime, settings: &Settings) -> Result<(), CommandError> {
    if let Err(e) = crate::state::settings::save_settings(&rt.paths.settings(), settings) {
        return Err(CommandError::new(
            "io_error",
            format!("保存 settings.json 失败：{}", e),
        ));
    }
    Ok(())
}

/// 在持锁状态下生成一次。成功/失败都会更新 last 与 settings 里的时间戳。
fn generate_locked(rt: &AppRuntime, origin: &str) -> (bool, String) {
    let settings = rt.settings();
    let project = rt.project();

    let Some(adapter) = adapters::by_id(&settings.target) else {
        let msg = format!("不支持的目标内核：{}", settings.target);
        rt.log("error", origin, &msg);
        return (false, msg);
    };
    let paths = rt.paths.export_paths(&settings.target, adapter.file_name());
    if let Err(e) = rt.paths.ensure_dirs(&settings.target) {
        let msg = format!("创建输出目录失败：{}", e);
        rt.log("error", origin, &msg);
        return (false, msg);
    }

    rt.log(
        "info",
        origin,
        format!(
            "开始生成：target={} enabled={} 规则={}",
            settings.target,
            settings.enabled,
            project.rules.len()
        ),
    );

    match exporter::export(
        &project,
        &paths,
        settings.enabled,
        platform::app_version(),
        MODEL_SCHEMA_VERSION,
        &settings.target,
        settings.backup_limit,
        &sni_link(rt),
    ) {
        Ok(outcome) => {
            for step in &outcome.steps {
                rt.log("info", "exporter", step);
            }
            for w in &outcome.warnings {
                rt.log("warn", "validator", w.message.clone());
            }
            rt.log(
                "ok",
                origin,
                format!(
                    "生成成功：{} 条规则 → {}（sha256 {}…）",
                    outcome.rule_count,
                    outcome.script_path.display(),
                    &outcome.sha256[..12.min(outcome.sha256.len())]
                ),
            );
            let last = LastRun {
                at: outcome.generated_at.clone(),
                ok: true,
                message: format!("已生成 {} 条规则", outcome.rule_count),
                sha256: Some(outcome.sha256.clone()),
                bytes: Some(outcome.bytes),
                backup: outcome.backup.as_ref().map(|p| p.display().to_string()),
                rule_count: outcome.rule_count,
                enabled: outcome.enabled,
            };
            rt.set_last(last);

            let mut s = rt.settings();
            s.last_generated_at = Some(outcome.generated_at);
            s.last_generation_ok = Some(true);
            if let Err(e) = crate::state::settings::save_settings(&rt.paths.settings(), &s) {
                rt.log("error", origin, format!("保存 settings.json 失败：{}", e));
            }
            (true, format!("生成成功（{} 条规则）", outcome.rule_count))
        }
        Err(e) => {
            let msg = match &e {
                exporter::ExportError::Validation(issues) => format!(
                    "校验未通过，未写入任何文件（{} 个错误）：{}",
                    issues.len(),
                    issues
                        .iter()
                        .take(3)
                        .map(|i| i.message.clone())
                        .collect::<Vec<_>>()
                        .join("；")
                ),
                other => format!("生成失败：{}", other),
            };
            rt.log("error", origin, msg.clone());
            let last = LastRun {
                at: crate::state::now_rfc3339(),
                ok: false,
                message: msg.clone(),
                sha256: None,
                bytes: None,
                backup: None,
                rule_count: 0,
                enabled: settings.enabled,
            };
            rt.set_last(last);

            let mut s = rt.settings();
            s.last_generation_ok = Some(false);
            let _ = crate::state::settings::save_settings(&rt.paths.settings(), &s);
            (false, msg)
        }
    }
}

fn lock_write(rt: &AppRuntime) -> MutexGuard<'_, ()> {
    match rt.write_lock.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

pub fn open_output_dir_impl(rt: &AppRuntime) -> Result<(), String> {
    let settings = rt.settings();
    let dir = rt.paths.output_dir(&settings.target);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{}", e))?;
    platform::open_dir(&dir).map_err(|e| format!("打开目录失败：{}", e))
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 把「跟随系统 / 浅色 / 深色」应用到原生标题栏。
/// Web 内容由前端 CSS 变量切换；这里只影响系统绘制的窗口边框。
pub fn apply_window_theme(app: &AppHandle, mode: &str) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let theme = match mode {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None, // None = 跟随系统
    };
    if let Err(e) = w.set_theme(theme) {
        log(app, "warn", "ui", format!("设置窗口主题失败：{}", e));
    }
}

pub fn toggle_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        match w.is_visible() {
            Ok(true) => {
                let _ = w.hide();
            }
            _ => show_main_window(app),
        }
    }
}

// ---------------------------------------------------------------- 本地服务

use crate::hostmode::{self, HostConfig, ServiceStatus};

#[derive(serde::Serialize)]
pub struct HostView {
    pub config: HostConfig,
    pub status: ServiceStatus,
}

#[derive(serde::Serialize)]
pub struct Presets {
    /// 实际用于配置的通配符（单条）
    pub uu_domains: Vec<String>,
    /// 展示用的已知端点
    pub uu_endpoints: Vec<String>,
    pub uu_suffix: String,
    pub uu_pattern: String,
}

#[tauri::command]
pub fn get_presets() -> Presets {
    Presets {
        uu_domains: uurelaybridge_core::presets::uu_domains(),
        uu_endpoints: uurelaybridge_core::presets::uu_endpoints(),
        uu_suffix: uurelaybridge_core::presets::UU_SUFFIX.to_string(),
        uu_pattern: uurelaybridge_core::presets::UU_PATTERN.to_string(),
    }
}

#[tauri::command]
pub fn host_get(rt: State<'_, AppRuntime>) -> HostView {
    HostView {
        config: rt.host_config(),
        status: hostmode::status(),
    }
}

fn host_logger(app: &AppHandle) -> std::sync::Arc<dyn Fn(&str) + Send + Sync> {
    let h = app.clone();
    std::sync::Arc::new(move |m: &str| {
        if let Some(rt) = h.try_state::<AppRuntime>() {
            rt.log("info", "engine", m);
        }
    })
}

/// 按当前模式启动服务：clash → SOCKS；host → 80/443。
pub fn start_service(app: &AppHandle, rt: &AppRuntime) -> Result<ServiceStatus, String> {
    let mut cfg = rt.host_config();
    // 域名只来自规则
    cfg.domains = rule_domains(&rt.project());
    if cfg.domains.is_empty() {
        return Err(
            "还没有启用任何域名规则。请到「规则」页添加（可点「填入 UU」快速生成）。"
                .to_string(),
        );
    }
    let mode = rt.settings().mode;
    hostmode::start(&mode, &cfg, &rt.paths.root, host_logger(app)).map(|st| {
        rt.log(
            "ok",
            "host",
            format!(
                "服务已启动（{}）：{}",
                if mode == "host" { "Host 模式" } else { "SOCKS 模式" },
                st.view()
            ),
        );
        st
    })
}

/// host 模式需要管理员权限（绑定 80/443）。
/// 失败时主动弹 UAC，由用户决定是否以管理员身份重启。
pub fn try_elevate_for_service(app: &AppHandle, err: &str) -> bool {
    if !err.contains("needs admin") && !err.to_ascii_lowercase().contains("permission") {
        return false;
    }
    let rt = app.state::<AppRuntime>();
    if rt.settings().mode != "host" {
        return false;
    }
    rt.log(
        "warn",
        "host",
        "host 模式需要管理员权限，正在请求提权（会弹 UAC 确认框）",
    );
    // 记录"提权后要接着启动"，新实例启动时读它
    {
        let mut s = match rt.settings.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // 记录"提权后要接着启动"，按当前模式落位
        s.set_mode_enabled(true);
        let _ = crate::state::settings::save_settings(&rt.paths.settings(), &s);
    }
    match hostmode::elevate::relaunch_elevated(&rt.paths.root) {
        Ok(()) => {
            rt.log("info", "host", "已请求提权，本实例即将退出");
            // 让新实例接管，避免两个进程抢数据目录
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(400));
                std::process::exit(0);
            });
            true
        }
        Err(e) => {
            rt.log("error", "host", format!("提权被取消或失败：{e}"));
            false
        }
    }
}

pub fn stop_service(rt: &AppRuntime) {
    hostmode::stop();
    rt.log("info", "host", "服务已停止");
}

/// 单一开关：开 = 起服务并注入配置，关 = 停服务并撤掉配置。
pub fn apply_enabled(app: &AppHandle, enabled: bool, approved: bool, origin: &str) {
    let rt = app.state::<AppRuntime>();
    if !approved {
        rt.log("warn", origin, "缺少授权，已取消本次修改");
        return;
    }
    let guard = lock_write(&rt);
    {
        let mut s = match rt.settings.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // 两种模式各自记住开关状态
        s.set_mode_enabled(enabled);
        if let Err(e) = crate::state::settings::save_settings(&rt.paths.settings(), &s) {
            rt.log("error", origin, format!("保存 settings.json 失败：{e}"));
        }
    }

    if enabled {
        rt.log("info", origin, "已授权：启动本地服务");
        if let Err(e) = start_service(app, &rt) {
            rt.log("error", origin, format!("启动失败：{e}"));
            // 权限不足：按用户设置主动弹 UAC 提权重启
            if try_elevate_for_service(app, &e) {
                drop(guard);
                return;
            }
            // 其它失败：把开关退回，避免界面显示"已开启"却没用
            if let Ok(mut s) = rt.settings.write() {
                s.set_mode_enabled(false);
                let _ = crate::state::settings::save_settings(&rt.paths.settings(), &s);
            }
        }
    } else {
        rt.log("info", origin, "已授权：停止本地服务");
        stop_service(&rt);
    }
    drop(guard);
    tray::refresh(app);
    emit_state(app);
}

pub fn apply_regenerate(app: &AppHandle, approved: bool, origin: &str) {
    let rt = app.state::<AppRuntime>();
    if !approved {
        rt.log("warn", origin, "缺少授权，已取消本次操作");
        return;
    }
    let guard = lock_write(&rt);
    rt.log("info", origin, "已授权：导出配置到文件");
    let _ = generate_locked(&rt, origin);
    drop(guard);
    tray::refresh(app);
    emit_state(app);
}

#[tauri::command]
pub fn host_set_config(
    config: HostConfig,
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<HostView, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "保存配置会写入 hostmode.json，需要你的确认",
        ));
    }
    let mut cfg = config;
    // 域名由规则页维护，这里保留旧值（前端不再传）
    cfg.domains = rt.host_config().domains;
    cfg.sanitize();
    hostmode::save_config(&rt.paths.root, &cfg)
        .map_err(|e| CommandError::new("io_error", format!("保存 hostmode.json 失败：{e}")))?;
    rt.set_host_config(cfg.clone());
    rt.log(
        "info",
        "host",
        format!(
            "配置已保存：{} 个域名 · {} 段 · SOCKS {} / TLS {}",
            cfg.domains.len(),
            cfg.fragment_parts,
            cfg.socks_port,
            cfg.listen_port
        ),
    );

    // 服务在跑就按新配置重启
    if rt.settings().mode_enabled() {
        stop_service(&rt);
        let _ = start_service(&app, &rt);
        tray::refresh(&app);
        emit_state(&app);
    }

    Ok(HostView {
        config: cfg,
        status: hostmode::status(),
    })
}

#[tauri::command]
pub fn service_start(
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<HostView, CommandError> {
    apply_enabled(&app, true, approved, "ui");
    Ok(HostView {
        config: rt.host_config(),
        status: hostmode::status(),
    })
}

#[tauri::command]
pub fn service_stop(app: AppHandle, rt: State<'_, AppRuntime>) -> HostView {
    {
        let mut s = match rt.settings.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        s.set_mode_enabled(false);
        let _ = crate::state::settings::save_settings(&rt.paths.settings(), &s);
    }
    stop_service(&rt);
    tray::refresh(&app);
    emit_state(&app);
    HostView {
        config: rt.host_config(),
        status: hostmode::status(),
    }
}

/// 写 hosts（Host 模式一键配置）。没权限时拉起提权子进程。
#[tauri::command]
pub fn host_apply_hosts(
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<HostView, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "修改系统 hosts 需要管理员权限，需要你的确认",
        ));
    }
    let domains = rule_domains(&rt.project());
    if domains.is_empty() {
        return Err(CommandError::new(
            "bad_config",
            "还没有启用任何域名规则，请先到「规则」页添加",
        ));
    }
    // 保存配置时把规则域名同步进 hostmode.json，供内核与提权子进程读取
    let mut cfg = rt.host_config();
    cfg.domains = domains.clone();
    cfg.sanitize();
    hostmode::save_config(&rt.paths.root, &cfg).ok();

    let root = rt.paths.root.clone();
    // 写之前先原样备份，用户随时能「恢复原 hosts」
    if let Err(e) = hostmode::hosts::backup_current(&root) {
        rt.log("warn", "host", format!("备份 hosts 失败（继续执行）：{e}"));
    }

    let handle = app.clone();
    std::thread::spawn(move || {
        let rt = handle.state::<AppRuntime>();
        let _ = std::fs::remove_file(hostmode::elevate::op_result_path(&root));
        rt.log("info", "host", "正在写入 hosts（可能需要管理员授权）…");
        let result = if hostmode::can_write_hosts() {
            hostmode::hosts::apply(&domains).map(|n| format!("已写入 {n} 条 hosts 记录"))
        } else {
            match hostmode::elevate::elevate_op("apply", &root) {
                Ok(()) => hostmode::elevate::wait_result(&root, std::time::Duration::from_secs(90))
                    .map(|r| r.message),
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(msg) => rt.log("ok", "host", format!("hosts：{msg}")),
            Err(e) => rt.log("error", "host", format!("写入 hosts 失败：{e}")),
        }
        let _ = handle.emit(STATE_EVENT, ());
    });

    Ok(HostView {
        config: cfg,
        status: hostmode::status(),
    })
}

/// 恢复写入前的 hosts 备份。
#[tauri::command]
pub fn host_restore_hosts(
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<HostView, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "恢复 hosts 备份需要管理员权限，需要你的确认",
        ));
    }
    let root = rt.paths.root.clone();
    let handle = app.clone();
    std::thread::spawn(move || {
        let rt = handle.state::<AppRuntime>();
        let _ = std::fs::remove_file(hostmode::elevate::op_result_path(&root));
        rt.log("info", "host", "正在恢复 hosts 备份…");
        let result = if hostmode::can_write_hosts() {
            hostmode::hosts::restore_latest(&root).map(|p| match p {
                Some(path) => format!("已恢复到 {}", path.display()),
                None => "没有可用的备份".to_string(),
            })
        } else {
            match hostmode::elevate::elevate_op("restore", &root) {
                Ok(()) => hostmode::elevate::wait_result(&root, std::time::Duration::from_secs(90))
                    .map(|r| r.message),
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(msg) => rt.log("ok", "host", format!("hosts：{msg}")),
            Err(e) => rt.log("error", "host", format!("恢复 hosts 失败：{e}")),
        }
        let _ = handle.emit(STATE_EVENT, ());
    });

    Ok(HostView {
        config: rt.host_config(),
        status: hostmode::status(),
    })
}

#[tauri::command]
pub fn host_clear_hosts(
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<HostView, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "移除 hosts 记录需要管理员权限，需要你的确认",
        ));
    }
    let root = rt.paths.root.clone();
    let handle = app.clone();
    std::thread::spawn(move || {
        let rt = handle.state::<AppRuntime>();
        let _ = std::fs::remove_file(hostmode::elevate::op_result_path(&root));
        rt.log("info", "host", "正在移除 hosts 记录（可能需要管理员授权）…");
        let result = if hostmode::can_write_hosts() {
            hostmode::hosts::remove().map(|changed| {
                if changed {
                    "已移除 UURelayBridge 的 hosts 记录".to_string()
                } else {
                    "hosts 里没有 UURelayBridge 的记录".to_string()
                }
            })
        } else {
            match hostmode::elevate::elevate_op("remove", &root) {
                Ok(()) => hostmode::elevate::wait_result(&root, std::time::Duration::from_secs(90))
                    .map(|r| r.message),
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(msg) => rt.log("ok", "host", format!("hosts：{msg}")),
            Err(e) => rt.log("error", "host", format!("移除 hosts 失败：{e}")),
        }
        let _ = handle.emit(STATE_EVENT, ());
    });

    Ok(HostView {
        config: rt.host_config(),
        status: hostmode::status(),
    })
}

// ---------------------------------------------------------------- 原有命令


#[tauri::command]
pub fn bootstrap(rt: State<'_, AppRuntime>) -> Snapshot {
    // 只在第一次调用时记一条，用来确认前端确实连上了后端（也方便排障）
    static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    if FIRST.swap(false, std::sync::atomic::Ordering::SeqCst) {
        rt.log("info", "ui", "界面已就绪，前后端 IPC 正常");
    }
    build_snapshot(&rt)
}

#[tauri::command]
pub fn get_logs(rt: State<'_, AppRuntime>, limit: Option<usize>) -> Vec<LogEntry> {
    rt.log_tail(limit.unwrap_or(300))
}

/// 前端把结构化日志写进同一条实时日志（排障用，界面本身也会调用）。
#[tauri::command]
pub fn frontend_log(level: String, message: String, rt: State<'_, AppRuntime>) {
    let lv = match level.as_str() {
        "warn" => "warn",
        "error" => "error",
        "ok" => "ok",
        _ => "info",
    };
    rt.log(lv, "ui", message);
}

#[tauri::command]
pub fn clear_logs(app: AppHandle, rt: State<'_, AppRuntime>) -> Snapshot {
    rt.clear_logs();
    rt.log("info", "ui", "日志已清空");
    emit_state(&app);
    build_snapshot(&rt)
}

/// 切换模式。若当前模式的服务仍在运行，先停掉它（需要调用方确认）。
#[tauri::command]
pub fn set_mode(
    mode: String,
    confirm_stop: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<ModeSwitchOut, CommandError> {
    let mut s = rt.settings();
    let next = if uurelaybridge_core::settings::MODES.contains(&mode.as_str()) {
        mode
    } else {
        "socks".to_string()
    };
    if s.mode == next {
        return Ok(ModeSwitchOut {
            switched: true,
            needs_confirm: false,
            running_mode: None,
            snapshot: build_snapshot(&rt),
        });
    }

    // 另一个模式正在跑，必须先关掉
    let running = hostmode::status().running;
    if running && !confirm_stop {
        return Ok(ModeSwitchOut {
            switched: false,
            needs_confirm: true,
            running_mode: Some(s.mode.clone()),
            snapshot: build_snapshot(&rt),
        });
    }
    if running {
        stop_service(&rt);
        rt.log("info", "ui", format!("已停止 {} 模式的服务", s.mode));
    }

    s.mode = next.clone();
    s.sanitize();
    crate::state::settings::save_settings(&rt.paths.settings(), &s)
        .map_err(|e| CommandError::new("io_error", format!("保存设置失败：{e}")))?;
    if let Ok(mut g) = rt.settings.write() {
        *g = s.clone();
    }
    rt.log(
        "info",
        "ui",
        format!(
            "使用方式：{}",
            if next == "host" { "Host 模式" } else { "SOCKS 模式" }
        ),
    );

    // 新模式如果之前是开着的，这里不自动启动，交给用户拨开关
    tray::refresh(&app);
    emit_state(&app);
    Ok(ModeSwitchOut {
        switched: true,
        needs_confirm: false,
        running_mode: None,
        snapshot: build_snapshot(&rt),
    })
}

#[derive(serde::Serialize)]
pub struct ModeSwitchOut {
    pub switched: bool,
    pub needs_confirm: bool,
    pub running_mode: Option<String>,
    pub snapshot: Snapshot,
}

/// 重启内核（同一模式下先停再起）。
#[tauri::command]
pub fn restart_service(
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<HostView, CommandError> {
    if !approved {
        return Err(CommandError::new("needs_approval", "重启内核需要你的确认"));
    }
    rt.log("info", "host", "正在重启内核…");
    stop_service(&rt);
    std::thread::sleep(std::time::Duration::from_millis(250));
    match start_service(&app, &rt) {
        Ok(st) => {
            rt.log("ok", "host", format!("内核已重启：{}", st.view()));
            let mut s = match rt.settings.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            s.set_mode_enabled(true);
            let _ = crate::state::settings::save_settings(&rt.paths.settings(), &s);
        }
        Err(e) => {
            rt.log("error", "host", format!("重启失败：{e}"));
            if let Ok(mut s) = rt.settings.write() {
                s.set_mode_enabled(false);
                let _ = crate::state::settings::save_settings(&rt.paths.settings(), &s);
            }
        }
    }
    tray::refresh(&app);
    emit_state(&app);
    Ok(HostView {
        config: rt.host_config(),
        status: hostmode::status(),
    })
}

#[tauri::command]
pub fn preview(project: Project, rt: State<'_, AppRuntime>) -> PreviewOut {
    let settings = rt.settings();
    let sni = sni_link(&rt);
    let (text, compiled_rules, skipped) =
        render_preview(&project, settings.enabled, &settings.target, &sni);
    PreviewOut {
        text,
        validation: validator::validate(&project),
        compiled_rules,
        skipped,
    }
}

#[tauri::command]
pub fn set_enabled(
    approved: bool,
    enabled: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<Snapshot, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "切换 RelayBridge 开关会重写配置文件，需要你的确认",
        ));
    }
    apply_enabled(&app, enabled, true, "ui");
    Ok(build_snapshot(&rt))
}

#[tauri::command]
pub fn regenerate(
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<Snapshot, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "重新生成会覆盖输出文件（旧版本自动备份），需要你的确认",
        ));
    }
    apply_regenerate(&app, true, "ui");
    Ok(build_snapshot(&rt))
}

#[tauri::command]
pub fn save_project(
    project: Project,
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<Snapshot, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "保存规则会写入 project.json，需要你的确认",
        ));
    }
    let issues = validator::validate(&project);
    if has_errors(&issues) {
        for i in issues
            .iter()
            .filter(|i| i.severity == uurelaybridge_core::validator::Severity::Error)
        {
            rt.log(
                "warn",
                "ui",
                format!(
                    "校验失败：{}（{}）{}",
                    i.message,
                    i.field,
                    i.rule_id.clone().unwrap_or_default()
                ),
            );
        }
        return Err(CommandError::with_issues(
            "validation",
            "规则有错误，未保存",
            issues,
        ));
    }
    let guard = lock_write(&rt);
    if let Err(e) = crate::state::settings::save_project(&rt.paths.project(), &project) {
        return Err(CommandError::new(
            "io_error",
            format!("保存 project.json 失败：{}", e),
        ));
    }
    {
        let mut g = match rt.project.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *g = project;
    }
    rt.log(
        "info",
        "ui",
        format!("已授权：保存规则（{} 条）；生成内容已更新，不写文件", rt.project().rules.len()),
    );
    drop(guard);
    tray::refresh(&app);
    emit_state(&app);
    Ok(build_snapshot(&rt))
}

#[tauri::command]
pub fn update_settings(
    settings: Settings,
    approved: bool,
    app: AppHandle,
    rt: State<'_, AppRuntime>,
) -> Result<Snapshot, CommandError> {
    if !approved {
        return Err(CommandError::new(
            "needs_approval",
            "保存设置会写入 settings.json，需要你的确认",
        ));
    }
    let mut next = settings;
    next.sanitize();
    let before = rt.settings();
    let needs_regen = before.enabled != next.enabled
        || before.target != next.target
        || before.backup_limit != next.backup_limit;

    let guard = lock_write(&rt);
    {
        let mut g = match rt.settings.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *g = next.clone();
    }
    persist_settings(&rt, &next)?;
    rt.log("info", "ui", "已授权：保存设置");
    apply_window_theme(&app, &next.theme);
    rt.log(
        "info",
        "ui",
        format!(
            "主题：{}",
            match next.theme.as_str() {
                "light" => "浅色",
                "dark" => "深色",
                _ => "跟随系统",
            }
        ),
    );
    if needs_regen {
        rt.log(
            "info",
            "ui",
            "设置已保存；生成内容随开关实时更新，不再自动写文件",
        );
    }
    drop(guard);
    tray::refresh(&app);
    emit_state(&app);
    Ok(build_snapshot(&rt))
}

#[tauri::command]
pub fn open_output_dir(app: AppHandle, rt: State<'_, AppRuntime>) -> Result<(), CommandError> {
    open_output_dir_impl(&rt).map_err(|e| {
        rt.log("error", "ui", e.clone());
        CommandError::new("io_error", e)
    })?;
    let _ = app;
    Ok(())
}

#[tauri::command]
pub fn open_data_dir(rt: State<'_, AppRuntime>) -> Result<(), CommandError> {
    platform::open_dir(std::path::Path::new(&rt.paths.root))
        .map_err(|e| CommandError::new("io_error", format!("打开数据目录失败：{}", e)))
}

#[tauri::command]
pub fn open_backups_dir(rt: State<'_, AppRuntime>) -> Result<(), CommandError> {
    let dir = rt.paths.backups_dir();
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::new("io_error", e.to_string()))?;
    platform::open_dir(&dir)
        .map_err(|e| CommandError::new("io_error", format!("打开备份目录失败：{}", e)))
}

#[tauri::command]
pub fn hide_main_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command]
pub fn show_main_window_cmd(app: AppHandle) {
    show_main_window(&app);
}

#[tauri::command]
pub fn quit_app(app: AppHandle, rt: State<'_, AppRuntime>) {
    rt.begin_quit();
    rt.log("info", "ui", "用户请求退出 RelayBridge");
    tray::teardown();
    app.exit(0);
}

#[tauri::command]
pub fn resolve_paths(rt: State<'_, AppRuntime>) -> PathsView {
    let settings = rt.settings();
    paths_view(&rt, &settings.target)
}

/// 供 lib.rs 在窗口事件里读取设置。
pub fn close_to_tray(app: &AppHandle) -> bool {
    app.state::<AppRuntime>().settings().close_to_tray
}

pub fn log(app: &AppHandle, level: &str, source: &str, message: impl Into<String>) {
    app.state::<AppRuntime>().log(level, source, message);
}

/// 启动时把数据目录和数据状态写进日志（只读操作）。
pub fn log_startup(app: &AppHandle, paths: &AppPaths) {
    let rt = app.state::<AppRuntime>();
    rt.log(
        "info",
        "app",
        format!(
            "UURelayBridge v{} 启动（{}）",
            platform::app_version(),
            platform::platform_name()
        ),
    );
    rt.log("info", "app", format!("数据目录：{}", paths.display()));
    let settings = rt.settings();
    rt.log(
        "info",
        "app",
        format!(
            "当前状态：{} · 目标内核：{} · 规则 {} 条",
            if settings.mode_enabled() { "ON" } else { "OFF" },
            settings.target,
            rt.project().rules.len()
        ),
    );
    for issue in rt.issues() {
        rt.log("warn", "app", issue);
    }
    rt.log(
        "info",
        "app",
        "提示：RelayBridge 只生成自己的文件，不会修改 Clash Verge 的订阅或配置",
    );
}