/* UURelayBridge 前端逻辑（重写版）
 *
 * 约定：
 *  - 所有落盘操作先经过 confirmDialog（用户显式授权）
 *  - 顶部开关 = 启停内核；模式切换 = 内核以 socks / host 运行
 *  - 日志独立成页，不再挤压主区高度
 */
(function () {
  "use strict";

  const S = { snap: null, busy: false, tab: "overview" };
  const HOST = { view: null, dirty: false, timer: null };
  const RULE = { editing: null };
  let PRESETS = null;

  const $ = (id) => document.getElementById(id);
  const hasBackend = () => !!window.__TAURI__;

  async function api(cmd, args) {
    if (!hasBackend()) throw new Error("没有后端（浏览器预览模式）");
    return window.__TAURI__.core.invoke(cmd, args || {});
  }

  /* ---------------- 提示 ---------------- */
  let toastTimer = null;
  function toast(msg, kind, ms) {
    const el = $("toast");
    el.textContent = msg;
    el.className = "toast" + (kind ? " " + kind : "");
    el.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (el.hidden = true), ms || 2600);
  }

  function confirmDialog(title, html) {
    return new Promise((resolve) => {
      $("confirmTitle").textContent = title;
      $("confirmBody").innerHTML = html;
      $("confirmModal").hidden = false;
      const done = (v) => {
        $("confirmModal").hidden = true;
        $("confirmOk").removeEventListener("click", ok);
        $("confirmCancel").removeEventListener("click", cancel);
        resolve(v);
      };
      const ok = () => done(true);
      const cancel = () => done(false);
      $("confirmOk").addEventListener("click", ok);
      $("confirmCancel").addEventListener("click", cancel);
    });
  }

  async function guard(fn) {
    if (S.busy) return;
    S.busy = true;
    try {
      await fn();
    } catch (e) {
      const msg = typeof e === "string" ? e : (e && e.message) || JSON.stringify(e);
      toast(msg, "err", 5000);
      console.error(e);
    } finally {
      S.busy = false;
    }
  }

  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, (c) => (
      { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]
    ));
  }

  /* ---------------- 模式 / 标签 ---------------- */
  function currentMode() {
    return S.snap && S.snap.settings.mode === "host" ? "host" : "socks";
  }

  function applyMode() {
    const mode = currentMode();
    document.body.dataset.mode = mode;
    document.querySelectorAll(".mode-btn").forEach((b) => {
      b.classList.toggle("active", b.dataset.mode === mode);
    });
    document.querySelectorAll(".nav-item").forEach((n) => {
      const forMode = n.dataset.for || "both";
      n.hidden = forMode !== "both" && forMode !== mode;
    });
    // 配置页内部：两种模式各自的区块
    $("socksPane").hidden = mode !== "socks";
    $("hostPane").hidden = mode !== "host";
    $("modeBadge").textContent = mode === "host" ? "HOST" : "SOCKS";
    $("modeBadge").className = "mode-badge " + mode;
    renderScriptBox();
    if (mode === "host" && hasBackend()) {
      setTimeout(() => guard(loadHost), 0);
    }
  }

  function renderScriptBox() {
    const box = $("scriptBox");
    if (!box) return;
    const text = (S.snap && S.snap.preview) || "";
    box.textContent = text || "（暂无内容）";
  }

  const TITLES = {
    overview: "概览",
    config: "配置",
    rules: "规则",
    logs: "日志",
    settings: "设置",
  };

  function switchTab(name) {
    const target = document.getElementById("page-" + name);
    if (!target) return;          // 不存在的页直接忽略，避免空白界面
    // 目标页不属于当前模式时，回落到该模式的首页。
    // 注意：不能用 navItem.hidden 判断 —— 点击瞬间它可能还是上一次的旧值。
    const navItem = document.querySelector('.nav-item[data-tab="' + name + '"]');
    const forMode = navItem ? (navItem.dataset.for || "both") : "both";
    if (forMode !== "both" && forMode !== currentMode()) {
      name = currentMode() === "host" ? "host" : "overview";
      if (!document.getElementById("page-" + name)) return;
    }
    S.tab = name;
    document.querySelectorAll(".nav-item").forEach((n) => {
      n.classList.toggle("active", n.dataset.tab === name);
    });
    const shown = "page-" + name;
    document.querySelectorAll(".page").forEach((p) => {
      p.classList.toggle("active", p.id === shown);
    });
    $("pageTitle").textContent = TITLES[name] || "";
  }

  /* ---------------- 渲染 ---------------- */
  function renderAll() {
    if (!S.snap) return;
    const s = S.snap.settings;
    const svc = S.snap.service || {};

    $("toggleEnabled").checked = !!s.enabled;
    $("appMeta").textContent = "v" + S.snap.app_version;

    // 侧栏状态
    const dot = $("sideDot");
    const running = !!svc.running;
    dot.className = "dot" + (running ? " on" : svc.last_error ? " err" : "");
    const modeName = currentMode() === "host" ? "Host 模式" : "SOCKS 模式";
    $("sideText").textContent = running ? modeName + " · 运行中" : modeName + " · 未启动";
    $("sideHint").textContent = running
      ? (svc.socks || svc.https || "")
      : (svc.last_error || "服务未启动");

    // 概览卡片
    $("ovService").textContent = running
      ? (svc.socks ? "SOCKS " + svc.socks : (svc.https || "已监听"))
      : "未启动";
    $("ovServiceHint").textContent = running
      ? "内核进程已就绪"
      : (svc.last_error ? "上次失败：" + svc.last_error : "打开右上角开关启动");
    $("ovRules").textContent = String(S.snap.compiled_rules || 0);
    $("ovHosts").textContent = svc.hosts_applied ? "已写入" : "未写入";
    $("ovHostsHint").textContent = svc.hosts_applied
      ? (svc.hosts_domains || []).length + " 个域名指向本机"
      : "Host 模式下才需要";

    renderQuickSteps(svc, s);
    renderIssues(S.snap.validation || []);

    // 设置页
    $("setStartMin").checked = !!s.start_minimized;
    $("setCloseToTray").checked = !!s.close_to_tray;
    $("setNotify").checked = !!s.show_notifications;
    $("setAutostart").checked = !!s.autostart;
    $("setKeepalive").checked = !!s.keepalive;
    $("setTheme").value = s.theme;
    $("setData").textContent = (S.snap.paths || {}).data_dir || "—";
    $("setEngineLog").textContent = (S.snap.paths || {}).data_dir
      ? (S.snap.paths.data_dir + "\\uuengine.log") : "—";

    applyMode();
  }

  function renderQuickSteps(svc, s) {
    const mode = currentMode();
    const steps = mode === "host"
      ? [
          { t: "打开右上角开关，启动本机 443 转发", done: !!svc.running },
          { t: "点「写入 Hosts」，把域名指到 127.0.0.1", done: !!svc.hosts_applied },
          { t: "启动 UU 远程，保持本程序运行", done: false },
        ]
      : [
          { t: "打开右上角开关，启动本地 SOCKS 服务", done: !!svc.running },
          { t: "在 Clash Verge 的扩展脚本里粘贴生成内容", done: false },
          { t: "重载配置，让域名走本地 SOCKS", done: false },
        ];
    $("quickSteps").innerHTML = steps
      .map((st, i) => (
        '<div class="step' + (st.done ? " done" : "") + '">' +
        '<span class="step-num">' + (st.done ? "✓" : i + 1) + "</span>" +
        "<span>" + st.t + "</span></div>"
      ))
      .join("");
  }

  function renderIssues(list) {
    const pane = $("issuePane");
    const box = $("issueBox");
    const bad = (list || []).filter((i) => i.level !== "info");
    if (!bad.length) {
      pane.hidden = true;
      return;
    }
    pane.hidden = false;
    box.innerHTML = bad
      .map((i) => (
        '<div class="issue ' + (i.level === "error" ? "err" : "warn") + '">' +
        "<span>" + (i.level === "error" ? "✕" : "!") + "</span>" +
        "<span>" + esc(i.message) + "</span></div>"
      ))
      .join("");
  }

  /* ---------------- 日志 ---------------- */
  const LOG_MAX = 800;
  function addLog(entry) {
    const box = $("logBox");
    if (!box) return;
    const line = document.createElement("div");
    line.className = "log-line " + (entry.level || "info");
    line.textContent = entry.text || (
      (entry.ts ? entry.ts + " " : "") +
      "[" + (entry.level || "info") + "] [" + (entry.source || "app") + "] " +
      (entry.message || "")
    );
    box.appendChild(line);
    while (box.childElementCount > LOG_MAX) box.removeChild(box.firstChild);
    box.scrollTop = box.scrollHeight;
  }

  /* ---------------- Host 页 ---------------- */
  async function loadHost() {
    const v = await api("host_get");
    HOST.view = v;
    renderHost();
  }

  function renderHost() {
    const v = HOST.view;
    if (!v) return;
    const c = v.config || {};
    const st = v.status || {};
    if (!HOST.dirty) {
      $("hostDomains").value = (c.domains || []).join("\n");
      $("hostPort").value = c.listen_port;
      $("hostMode").value = c.fragment_mode;
      $("hostParts").value = c.fragment_parts;
      $("hostSocksPort").value = c.socks_port;
      $("hostDns").value = c.dns_server;
      $("hostLinkClash").checked = !!c.link_clash;
    }
    $("hostListen").textContent = st.running
      ? (st.https || st.socks || "—")
      : "未启动";
    $("hostHostsState").textContent = st.hosts_applied
      ? "已写入 " + (st.hosts_domains || []).length + " 条"
      : "未写入";
    $("hostStats").textContent = (st.conns || 0) + " / " + (st.fragmented || 0);
  }

  function hostFormConfig() {
    return {
      enabled: !!(HOST.view && HOST.view.config.enabled),
      listen_port: parseInt($("hostPort").value || "443", 10),
      http_port: 80,
      socks_port: parseInt($("hostSocksPort").value || "7899", 10),
      domains: $("hostDomains").value.split("\n").map((x) => x.trim()).filter(Boolean),
      fragment_mode: $("hostMode").value,
      fragment_parts: parseInt($("hostParts").value || "4", 10),
      dns_server: $("hostDns").value.trim() || "223.5.5.5",
      link_clash: $("hostLinkClash").checked,
    };
  }

  async function saveHost() {
    const ok = await confirmDialog("保存 Host 配置", "写入 <code>hostmode.json</code>。");
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_set_config", { config: hostFormConfig(), approved: true });
      HOST.dirty = false;
      S.snap = await api("bootstrap");
      renderHost();
      renderAll();
      toast("已保存");
    });
  }

  async function applyHosts() {
    const ok = await confirmDialog(
      "写入 Hosts",
      "会把域名指向 127.0.0.1，需要管理员权限（会弹 UAC）。" +
      "退出前请记得「移除 Hosts」。"
    );
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_apply_hosts", { approved: true });
      renderHost();
      toast("已请求写入，留意 UAC", "", 5000);
    });
  }

  async function clearHosts() {
    const ok = await confirmDialog("移除 Hosts", "只删除本程序写入的那一段，其它行保留。");
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_clear_hosts", { approved: true });
      renderHost();
      toast("已请求移除", "", 4000);
    });
  }

  async function restoreHosts() {
    const ok = await confirmDialog("恢复原 hosts", "用写入前的备份覆盖当前 hosts。");
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_restore_hosts", { approved: true });
      renderHost();
      toast("已请求恢复", "", 4000);
    });
  }

  /* ---------------- 开关 ---------------- */
  async function toggleEnabled(want) {
    const mode = currentMode();
    const ok = await confirmDialog(
      want ? "启动服务" : "停止服务",
      want
        ? (mode === "host"
            ? "会启动内核进程并监听本机 <b>80/443</b>。若当前不是管理员，会弹 UAC 请求提权。"
            : "会启动内核进程并在本机提供 <b>SOCKS5</b>（默认 127.0.0.1:7899）。")
        : "会<b>结束内核进程</b>；Hosts 不会被自动移除。"
    );
    if (!ok) {
      $("toggleEnabled").checked = !want;
      return;
    }
    await guard(async () => {
      S.snap = await api("set_enabled", { approved: true, enabled: want });
      renderAll();
      if (currentMode() === "host") await loadHost();
      toast(want ? "已启动" : "已停止");
    });
  }

  /* ---------------- 规则页 ---------------- */
  function renderRules() {
    if (!S.snap) return;
    const rules = (S.snap.project && S.snap.project.rules) || [];
    const body = $("rulesBody");
    body.innerHTML = rules
      .map((r) => (
        "<tr>" +
        '<td><input type="checkbox" data-rule="' + esc(r.id) + '"' + (r.enabled ? " checked" : "") + " /></td>" +
        "<td>" + esc(r.name || r.id) + "</td>" +
        "<td><code>" + esc((r.match && r.match.domains || []).join("<br>")) + "</code></td>" +
        "<td>" + esc(r.action) + "</td>" +
        '<td>' +
        '<button class="btn tiny ghost" data-edit="' + esc(r.id) + '">编辑</button> ' +
        '<button class="btn tiny ghost danger" data-del="' + esc(r.id) + '">删</button>' +
        "</td></tr>"
      ))
      .join("");
    $("rulesEmpty").hidden = rules.length > 0;
  }


  /* ---------------- 规则增删改 ---------------- */
  function openRule(rule) {
    RULE.editing = rule ? rule.id : null;
    $("ruleTitle").textContent = rule ? "编辑规则" : "新建规则";
    $("fName").value = rule ? (rule.name || "") : "";
    $("fDomains").value = rule ? ((rule.match && rule.match.domains) || []).join("\n") : "";
    $("fAction").value = rule ? rule.action : "proxy";
    $("fGroup").value = rule && rule.outbound ? rule.outbound : "";
    $("fGroupRow").hidden = $("fAction").value !== "proxy";
    $("ruleModal").hidden = false;
  }

  async function applyRuleForm() {
    const domains = $("fDomains").value.split("\n").map((x) => x.trim()).filter(Boolean);
    if (!domains.length) {
      toast("至少填一个域名", "warn");
      return;
    }
    const action = $("fAction").value;
    const outbound = $("fGroup").value.trim();
    if (action === "proxy" && !outbound) {
      toast("代理动作需要填策略组名", "warn");
      return;
    }
    const rule = {
      id: RULE.editing || ("rule-" + Date.now()),
      name: $("fName").value.trim() || domains[0],
      enabled: true,
      action,
      outbound: action === "proxy" ? outbound : null,
      match: { domains },
    };
    const rules = ((S.snap.project && S.snap.project.rules) || []).slice();
    const idx = rules.findIndex((r) => r.id === rule.id);
    if (idx >= 0) rules[idx] = rule;
    else rules.push(rule);
    const project = Object.assign({}, S.snap.project, { rules });
    S.snap = await api("save_project", { project, approved: true });
    $("ruleModal").hidden = true;
    renderRules();
    renderAll();
    toast("规则已保存");
  }

  async function deleteRule(id) {
    const rules = ((S.snap.project && S.snap.project.rules) || []).filter((r) => r.id !== id);
    const project = Object.assign({}, S.snap.project, { rules });
    S.snap = await api("save_project", { project, approved: true });
    renderRules();
    renderAll();
    toast("已删除");
  }

  async function toggleRule(id, enabled) {
    const rules = ((S.snap.project && S.snap.project.rules) || []).map((r) =>
      r.id === id ? Object.assign({}, r, { enabled }) : r
    );
    const project = Object.assign({}, S.snap.project, { rules });
    S.snap = await api("save_project", { project, approved: true });
    renderRules();
    renderAll();
  }

  /* ---------------- 设置 ---------------- */
  async function saveSettings() {
    const ok = await confirmDialog("保存设置", "写入 <code>settings.json</code>。");
    if (!ok) return;
    await guard(async () => {
      const s = Object.assign({}, S.snap.settings, {
        start_minimized: $("setStartMin").checked,
        close_to_tray: $("setCloseToTray").checked,
        show_notifications: $("setNotify").checked,
        autostart: $("setAutostart").checked,
        keepalive: $("setKeepalive").checked,
        theme: $("setTheme").value,
      });
      S.snap = await api("update_settings", { settings: s, approved: true });
      Theme.set(s.theme);
      renderAll();
      toast("设置已保存");
    });
  }

  /* ---------------- 主题 ---------------- */
  const Theme = {
    set(mode) {
      const mql = window.matchMedia("(prefers-color-scheme: dark)");
      const apply = () => {
        const dark = mode === "dark" || (mode === "system" && mql.matches);
        document.documentElement.dataset.theme = dark ? "dark" : "light";
      };
      apply();
      mql.onchange = mode === "system" ? apply : null;
    },
  };

  /* ---------------- 绑定 ---------------- */
  function bind() {
    document.querySelectorAll(".nav-item").forEach((n) => {
      n.addEventListener("click", () => switchTab(n.dataset.tab));
    });

    $("modeSwitch").addEventListener("click", (e) => {
      const btn = e.target.closest(".mode-btn");
      if (!btn || btn.dataset.mode === currentMode()) return;
      const next = btn.dataset.mode;
      guard(async () => {
        // 第一次尝试：如果旧模式还在跑，后端会要求确认
        let r = await api("set_mode", { mode: next, confirm_stop: false });
        if (r.needs_confirm) {
          const label = r.running_mode === "host" ? "Host 模式" : "SOCKS 模式";
          const ok = await confirmDialog(
            "切换模式",
            "当前 <b>" + label + "</b> 的内核仍在运行。<br/>" +
            "切换前需要先关闭它，关闭后该模式的连接会立即中断。"
          );
          if (!ok) return;
          r = await api("set_mode", { mode: next, confirm_stop: true });
        }
        S.snap = r.snapshot;
        renderAll();
        if (next === "host") await loadHost();
        toast(next === "host" ? "已切到 Host 模式" : "已切到 SOCKS 模式");
      });
    });

    $("btnRestart").addEventListener("click", () => guard(async () => {
      const ok = await confirmDialog("重启内核", "先停止再重新启动当前模式的内核，已建立的连接会断开。");
      if (!ok) return;
      HOST.view = await api("restart_service", { approved: true });
      S.snap = await api("bootstrap");
      renderAll();
      renderHost();
      toast("内核已重启");
    }));

    $("btnCopyScript").addEventListener("click", () => guard(async () => {
      const text = (S.snap && S.snap.preview) || "";
      if (!text) { toast("还没有可复制的内容", "warn"); return; }
      try {
        await navigator.clipboard.writeText(text);
        toast("已复制，粘贴进 Clash Verge 扩展脚本即可", "", 4000);
      } catch { toast("复制失败", "warn"); }
    }));

    $("btnExportScript").addEventListener("click", () => guard(async () => {
      const ok = await confirmDialog("导出脚本", "会写入输出目录，并生成备份与清单。");
      if (!ok) return;
      await api("generate", { approved: true });
      toast("已导出");
    }));

    $("toggleEnabled").addEventListener("change", (ev) => {
      toggleEnabled(ev.target.checked);
    });

    $("btnQuickStart").addEventListener("click", () => guard(async () => {
      if (currentMode() === "clash") {
        const text = (S.snap.preview || "").trim();
        if (!text) { toast("还没有可复制的内容", "warn"); return; }
        try {
          await navigator.clipboard.writeText(text);
          toast("已复制脚本，粘贴进 Clash Verge 扩展脚本即可", "", 4200);
        } catch {
          toast("复制失败，请到「规则」页手动查看", "warn");
        }
      } else {
        toast("请先打开开关，再点「写入 Hosts」", "", 3600);
      }
    }));

    $("btnHostSave").addEventListener("click", () => saveHost());
    $("btnHostApplyHosts").addEventListener("click", () => applyHosts());
    $("btnHostClearHosts").addEventListener("click", () => clearHosts());
    $("btnHostRestore").addEventListener("click", () => restoreHosts());
    ["hostDomains", "hostPort", "hostMode", "hostParts", "hostSocksPort", "hostDns", "hostLinkClash"]
      .forEach((id) => $(id).addEventListener("change", () => (HOST.dirty = true)));

    $("btnSaveSettings").addEventListener("click", () => saveSettings());
    $("setTheme").addEventListener("change", () => Theme.set($("setTheme").value));
    $("btnOpenData").addEventListener("click", () => guard(() => api("open_output_dir")));

    $("btnClearLogs").addEventListener("click", () => ($("logBox").innerHTML = ""));
    $("btnOpenLogFile").addEventListener("click", () => guard(() => api("open_output_dir")));

    $("btnPresetUU").addEventListener("click", () => guard(async () => {
      const p = await loadPresets();
      const rules = (S.snap.project && S.snap.project.rules) || [];
      const rule = {
        id: "uu-" + Date.now(),
        name: "UU 远程",
        enabled: true,
        action: "proxy",
        match: { domains: p.uu_domains || ["+.nrd.nie.163.com"] },
      };
      const project = Object.assign({}, S.snap.project, { rules: rules.concat([rule]) });
      S.snap = await api("save_project", { project, approved: true });
      renderRules();
      toast("已加入 UU 规则");
    }));

    $("btnJson").addEventListener("click", () => {
      $("jsonBox").value = JSON.stringify(S.snap.project, null, 2);
      $("jsonModal").hidden = false;
    });
    $("jsonCancel").addEventListener("click", () => ($("jsonModal").hidden = true));
    $("jsonOk").addEventListener("click", () => guard(async () => {
      const project = JSON.parse($("jsonBox").value);
      S.snap = await api("save_project", { project, approved: true });
      $("jsonModal").hidden = true;
      renderRules();
      renderAll();
      toast("已应用");
    }));

    $("btnAddRule").addEventListener("click", () => openRule(null));
    $("ruleCancel").addEventListener("click", () => ($("ruleModal").hidden = true));
    $("ruleOk").addEventListener("click", () => guard(applyRuleForm));
    $("rulesBody").addEventListener("click", (e) => {
      const edit = e.target.closest("[data-edit]");
      if (edit) {
        const r = ((S.snap.project && S.snap.project.rules) || [])
          .find((x) => x.id === edit.dataset.edit);
        if (r) openRule(r);
        return;
      }
      const del = e.target.closest("[data-del]");
      if (del) guard(() => deleteRule(del.dataset.del));
    });
    $("rulesBody").addEventListener("change", (e) => {
      const cb = e.target.closest("[data-rule]");
      if (cb) guard(() => toggleRule(cb.dataset.rule, cb.checked));
    });
    $("fAction").addEventListener("change", () => {
      $("fGroupRow").hidden = $("fAction").value !== "proxy";
    });

    // 事件广播
    if (hasBackend()) {
      window.__TAURI__.event.listen("state-changed", async () => {
        S.snap = await api("bootstrap");
        renderAll();
        renderRules();
      });
    }
  }

  async function loadPresets() {
    if (PRESETS || !hasBackend()) return PRESETS || { uu_domains: ["+.nrd.nie.163.com"] };
    PRESETS = await api("get_presets");
    return PRESETS;
  }

  /* ---------------- 启动 ---------------- */
  async function boot() {
    if (!hasBackend()) {
      $("appMeta").textContent = "预览模式";
      return;
    }
    bind();
    try {
      S.snap = await api("bootstrap");
      window.__LAST_PREVIEW__ = S.snap.preview || "";
      Theme.set(S.snap.settings.theme);
      renderAll();
      renderRules();
      switchTab("overview");
      const logs = await api("get_logs", { limit: 300 });
      logs.forEach(addLog);
      if (S.snap.settings.mode === "host") await loadHost();
    } catch (e) {
      document.body.insertAdjacentHTML(
        "afterbegin",
        '<div style="padding:12px;background:#dc2626;color:#fff">前端初始化失败：' +
        esc((e && e.message) || e) + "</div>"
      );
      console.error(e);
    }
  }

  window.addEventListener("DOMContentLoaded", boot);
})();
