//! 无界面端到端自检：`relaybridge --selftest`
//!
//! 用真实的 App 进程跑一遍"保存规则 → 开启 → 生成 → 备份 → 关闭 → 校验文件"，
//! 覆盖 state / adapter / exporter / 授权门控，便于在 CI 或排障时确认产物正确。
//! 不创建托盘交互，不需要点界面；退出码 0 表示全部通过。

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager};

use crate::commands;
use crate::state::Runtime as AppRuntime;

fn read_script(app: &AppHandle) -> String {
    let rt = app.state::<AppRuntime>();
    let s = rt.settings();
    let path = rt.paths.script(&s.target, "relaybridge.js");
    std::fs::read_to_string(path).unwrap_or_default()
}

/// 返回 0 = 通过。
pub fn run(app: &AppHandle) -> i32 {
    let rt = app.state::<AppRuntime>();
    rt.log("info", "selftest", "开始自检");

    let mut failures: Vec<String> = Vec::new();
    let mut check = |name: &str, ok: bool, detail: String| {
        if ok {
            println!("[selftest] PASS {name} {detail}");
            rt.log("ok", "selftest", format!("PASS {name} {detail}"));
        } else {
            println!("[selftest] FAIL {name} {detail}");
            rt.log("error", "selftest", format!("FAIL {name} {detail}"));
            failures.push(name.to_string());
        }
    };

    // 1) 准备一份规则并"保存"（等价于界面里点保存后写入 project.json）
    let project: uurelaybridge_core::model::Project = match serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "name": "selftest",
        "rules": [{
            "id": "selftest-rule",
            "name": "selftest",
            "enabled": true,
            "match": { "domain": ["example.com"], "ip_cidr": ["10.0.0.0/8"] },
            "action": { "type": "proxy", "outbound": "SELFTEST" }
        }]
    })) {
        Ok(p) => p,
        Err(e) => {
            println!("[selftest] FAIL parse-project {e}");
            return 1;
        }
    };
    let saved = uurelaybridge_core::settings::save_project(&rt.paths.project(), &project);
    {
        let mut g = match rt.project.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *g = project.clone();
    }
    check("save-project", saved.is_ok(), format!("{:?}", saved.err()));

    // 2) 未授权时必须拒绝
    let before = app.state::<AppRuntime>().settings().enabled;
    commands::apply_enabled(app, !before, false, "selftest");
    let after = app.state::<AppRuntime>().settings().enabled;
    check(
        "authorization-gate",
        before == after,
        "无授权时状态不变".to_string(),
    );

    // 3) 授权开启 → 生成 ON 版本
    commands::apply_enabled(app, true, true, "selftest");
    let on = read_script(app);
    check("enabled", on.contains("var RB_ENABLED = true;"), "RB_ENABLED=true".into());
    check(
        "injected-rule",
        on.contains("DOMAIN-SUFFIX,example.com,SELFTEST")
            && on.contains("IP-CIDR,10.0.0.0/8,SELFTEST"),
        "两条规则都在".into(),
    );
    check(
        "manifest-written",
        rt.paths.manifest(&rt.settings().target).exists(),
        "manifest.json 存在".into(),
    );

    // 4) 再次生成 → 必须产生备份
    commands::apply_regenerate(app, true, "selftest");
    let backups = uurelaybridge_core::exporter::backup::list(&rt.paths.backups_dir())
        .map(|v| v.len())
        .unwrap_or(0);
    check("backup-created", backups >= 1, format!("备份 {backups} 份"));

    // 5) 授权关闭 → OFF 版本仍然列出规则（这样才能在下次加载时摘掉）
    commands::apply_enabled(app, false, true, "selftest");
    let off = read_script(app);
    check("disabled", off.contains("var RB_ENABLED = false;"), "RB_ENABLED=false".into());
    check(
        "cleanup-list-kept",
        off.contains("DOMAIN-SUFFIX,example.com,SELFTEST"),
        "OFF 版本保留规则清单用于清理".into(),
    );

    // 6) 状态与文件一致性
    let last = rt.last();
    check(
        "last-state-consistent",
        last.as_ref().map(|l| l.ok && !l.enabled).unwrap_or(false),
        format!("{:?}", last),
    );

    // 7) 主题持久化 + 应用到原生窗口（不校验观感，只校验不报错、能落地）
    {
        let mut s = rt.settings();
        s.theme = "light".to_string();
        let saved_settings = uurelaybridge_core::settings::save_settings(&rt.paths.settings(), &s);
        {
            let mut g = match rt.settings.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            *g = s;
        }
        commands::apply_window_theme(app, "light");
        let text = std::fs::read_to_string(rt.paths.settings()).unwrap_or_default();
        let failed_theme = rt
            .log_tail(80)
            .iter()
            .any(|l| l.message.contains("设置窗口主题失败"));
        check("theme-persisted", saved_settings.is_ok() && text.contains("\"theme\": \"light\""), "settings.json 写入 light".into());
        check("theme-applied-without-error", !failed_theme, "窗口主题调用无错误".into());
        // 复位，避免污染后续使用
        let mut back = rt.settings();
        back.theme = "system".to_string();
        let _ = uurelaybridge_core::settings::save_settings(&rt.paths.settings(), &back);
        {
            let mut g = match rt.settings.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            *g = back;
        }
        commands::apply_window_theme(app, "system");
    }

    rt.quitting.store(true, Ordering::SeqCst);
    if failures.is_empty() {
        println!("[selftest] SELFTEST OK");
        rt.log("ok", "selftest", "SELFTEST OK");
        0
    } else {
        println!("[selftest] SELFTEST FAILED: {}", failures.join(", "));
        rt.log("error", "selftest", format!("SELFTEST FAILED: {:?}", failures));
        1
    }
}
