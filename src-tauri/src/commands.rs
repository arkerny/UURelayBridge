//! 前端命令层：所有落盘动作都在这里，并且必须先拿到 approved 授权。

use std::sync::MutexGuard;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use relaybridge_core::adapters::{self, RenderInput};
use relaybridge_core::compiler;
use relaybridge_core::exporter::{self, manifest::Manifest};
use relaybridge_core::model::{Project, MODEL_SCHEMA_VERSION};
use relaybridge_core::validator::{self, has_errors, Issue};

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

fn render_preview(project: &Project, enabled: bool, target: &str) -> (String, usize, Vec<String>) {
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
    let (preview, compiled_rules, _) = render_preview(&project, settings.enabled, &settings.target);
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
        backups,
    }
}

fn emit_state(app: &AppHandle) {
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

/// 供托盘/命令共用的开关实现（approved 代表"用户已经通过点击或确认框授权"）。
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
        s.enabled = enabled;
    }
    rt.log(
        "info",
        origin,
        format!(
            "已授权：{} RelayBridge 规则",
            if enabled { "启用" } else { "停用" }
        ),
    );
    let (ok, _msg) = generate_locked(&rt, origin);
    drop(guard);
    if !ok {
        rt.log("warn", origin, "状态已切换，但文件生成失败，请重试");
    }
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
    rt.log("info", origin, "已授权：重新生成配置");
    let _ = generate_locked(&rt, origin);
    drop(guard);
    tray::refresh(app);
    emit_state(app);
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

// ---------------------------------------------------------------- 命令

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

#[tauri::command]
pub fn preview(project: Project, rt: State<'_, AppRuntime>) -> PreviewOut {
    let settings = rt.settings();
    let (text, compiled_rules, skipped) =
        render_preview(&project, settings.enabled, &settings.target);
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
        rt.log("warn", "ui", "规则校验未通过，未保存");
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
        format!("已授权：保存规则（{} 条）", rt.project().rules.len()),
    );
    let _ = generate_locked(&rt, "ui");
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
        let _ = generate_locked(&rt, "ui");
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
            "RelayBridge v{} 启动（{}）",
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
            if settings.enabled { "ON" } else { "OFF" },
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
