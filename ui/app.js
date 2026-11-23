/* RelayBridge UI —— 原生 JS，无构建步骤。
 * 约定：所有会落盘的操作都必须先经过一次显式确认（下方 confirmDialog），
 * 后端还会再校验一次 approved=true，前端绕过也无效。 */
(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
  const listen = TAURI && TAURI.event ? TAURI.event.listen : null;

  const $ = (id) => document.getElementById(id);
  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  };

  const S = {
    snap: null,
    draft: null, // 规则草稿（null = 与已保存内容一致）
    dirty: false,
    editingId: null,
    busy: false,
    modalResolve: null,
  };

  // ------------------------------------------------------------ 主题
  const THEME_MODES = ["system", "light", "dark"];
  const Theme = {
    mode: "system",
    media: window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null,
    init() {
      if (this.media && this.media.addEventListener) {
        this.media.addEventListener("change", () => {
          if (this.mode === "system") this.apply();
        });
      }
    },
    set(mode) {
      this.mode = THEME_MODES.indexOf(mode) >= 0 ? mode : "system";
      this.apply();
    },
    resolved() {
      if (this.mode !== "system") return this.mode;
      return this.media && this.media.matches ? "dark" : "light";
    },
    apply() {
      document.documentElement.setAttribute("data-theme", this.resolved());
    },
  };

  // ------------------------------------------------------------ 基础工具
  const hasBackend = () => typeof invoke === "function";

  async function api(name, args) {
    return invoke(name, args || {});
  }

  function toast(msg, kind, ms) {
    const t = $("toast");
    t.textContent = msg;
    t.className = "toast" + (kind === "err" ? " err" : kind === "warn" ? " warn" : "");
    t.hidden = false;
    clearTimeout(toast._t);
    toast._t = setTimeout(() => (t.hidden = true), ms || 3400);
  }

  function confirmDialog(title, html) {
    return new Promise((resolve) => {
      $("modalTitle").textContent = title;
      $("modalBody").innerHTML = html;
      $("modal").hidden = false;
      $("modalOk").focus();
      S.modalResolve = resolve;
    });
  }

  function closeConfirm(v) {
    $("modal").hidden = true;
    if (S.modalResolve) {
      const r = S.modalResolve;
      S.modalResolve = null;
      r(v);
    }
  }

  function errText(e) {
    if (!e) return "未知错误";
    if (typeof e === "string") return e;
    if (e.message) return e.message;
    return JSON.stringify(e);
  }

  async function guard(fn) {
    if (S.busy) return;
    S.busy = true;
    try {
      return await fn();
    } catch (e) {
      const msg = errText(e);
      toast(msg, "err", 5200);
      addLog({ ts: nowHms(), level: "error", source: "ui", message: msg });
      if (e && e.issues && e.issues.length) showIssues(e.issues, "jsonIssues", "jsonModal");
    } finally {
      S.busy = false;
    }
  }

  function nowHms() {
    const d = new Date();
    const p = (n) => String(n).padStart(2, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }

  const esc = (s) => String(s == null ? "" : s)
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

  // ------------------------------------------------------------ 日志
  function addLog(entry) {
    if (!entry) return;
    const box = $("logBox");
    const line = el("div", "line");
    line.appendChild(el("span", "ts", entry.ts || nowHms()));
    line.appendChild(el("span", "lv " + (entry.level || "info"), (entry.level || "info").toUpperCase()));
    line.appendChild(el("span", "src", entry.source || "-"));
    line.appendChild(el("span", "msg", entry.message || ""));
    box.appendChild(line);
    S.logCount += 1;
    while (box.childElementCount > 600) box.removeChild(box.firstChild);
    $("logCount").textContent = String(S.logCount);
    if ($("logAutoScroll").checked) box.scrollTop = box.scrollHeight;
  }

  // ------------------------------------------------------------ 数据
  function project() {
    return S.draft || (S.snap ? S.snap.project : { schema_version: 1, name: "Default Project", rules: [] });
  }

  function setDraft(p) {
    S.draft = p;
    S.dirty = true;
    renderRules();
    renderDirtyState();
    refreshPreviewSoon();
  }

  function resetDraft() {
    S.draft = null;
    S.dirty = false;
    renderAll();
  }

  let previewTimer = null;
  function refreshPreviewSoon() {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(refreshPreview, 220);
  }

  async function refreshPreview() {
    if (!hasBackend()) return;
    const p = project();
    try {
      const out = await api("preview", { project: p });
      $("previewBox").textContent = out.text || "（没有可生成的规则）";
      renderIssues(out.validation);
      $("ruleCounter").textContent = `规则 ${p.rules.length} 条 · 生成 ${out.compiled_rules} 条`;
    } catch (e) {
      $("previewBox").textContent = "预览失败：" + errText(e);
    }
  }

  // ------------------------------------------------------------ 渲染
  function renderAll() {
    if (!S.snap) return;
    renderHeader();
    renderCards();
    renderPaths();
    renderBanner();
    renderRules();
    renderSettings();
    renderDirtyState();
    $("previewBox").textContent = S.snap.preview || "（没有可生成的规则）";
    renderIssues(S.snap.validation);
  }

  function renderHeader() {
    const s = S.snap.settings;
    const pill = $("statePill");
    const failed = S.snap.last && S.snap.last.ok === false;
    pill.className = "state-pill" + (failed ? " err" : s.enabled ? " on" : "");
    $("stateText").textContent = failed ? "生成失败" : s.enabled ? "ON" : "OFF";
    $("toggleEnabled").checked = !!s.enabled;
    $("appMeta").textContent = `v${S.snap.app_version} · ${S.snap.platform}`;
  }

  function renderCards() {
    const s = S.snap.settings;
    const last = S.snap.last;
    $("ovState").textContent = s.enabled ? "已启用（ON）" : "已停用（OFF）";
    $("ovStateHint").textContent = s.enabled
      ? "生成内容会注入规则；不代表 Clash 已生效"
      : "生成的脚本不会注入任何规则";
    $("ovTarget").textContent = s.target === "clash-verge-rev" ? "Clash Verge Rev" : s.target;
    $("ovTargetHint").textContent = S.snap.manifest ? `目标版本：${S.snap.manifest.target_version}` : "扩展脚本（main(config)）";
    if (last) {
      $("ovLast").textContent = (last.at || "").replace("T", " ").slice(0, 19);
      $("ovLast").className = "card-value" + (last.ok ? "" : " small");
      $("ovLastHint").textContent = last.ok
        ? `${last.rule_count} 条规则${last.backup ? " · 已备份上一版" : ""}`
        : last.message;
    } else {
      $("ovLast").textContent = "从未生成";
      $("ovLast").className = "card-value small";
      $("ovLastHint").textContent = "点右上角开关或「重新生成」";
    }
    $("ovScript").textContent = S.snap.paths.script;
    $("ovSha").textContent = S.snap.manifest
      ? `sha256 ${String(S.snap.manifest.sha256).slice(0, 16)}…`
      : "尚无 manifest";
  }

  function renderPaths() {
    const p = S.snap.paths;
    $("pData").textContent = p.data_dir;
    $("pBackups").textContent = p.backups_dir;
    $("pManifest").textContent = p.manifest;
    $("setData").textContent = p.data_dir;
    $("setOut").textContent = p.output_dir;
    $("setBak").textContent = p.backups_dir;
  }

  function renderBanner() {
    const b = $("consistencyBanner");
    const last = S.snap.last;
    const issues = S.snap.issues || [];
    let msg = null;
    let isErr = false;
    if (last && last.ok === false) {
      msg = `上次生成失败：${last.message}（可点「重新生成」重试）`;
      isErr = true;
    } else if (last && last.enabled !== S.snap.settings.enabled) {
      msg = `状态与文件不一致：开关是 ${S.snap.settings.enabled ? "ON" : "OFF"}，但输出文件是按 ${last.enabled ? "ON" : "OFF"} 生成的，请重新生成。`;
      isErr = true;
    } else if (issues.length) {
      msg = `启动时读取配置有问题：${issues[0]}`;
    } else if (S.snap.settings.enabled && (S.snap.compiled_rules || 0) === 0) {
      msg = "已开启，但当前没有可用规则，生成的脚本不会注入任何内容。";
    }
    b.hidden = !msg;
    b.className = "subtle-banner" + (isErr ? " err" : "");
    if (msg) b.textContent = msg;
  }

  function renderIssues(issues) {
    const box = $("issueBox");
    box.innerHTML = "";
    const list = (issues || []).filter((i) => i.severity === "error" || i.severity === "warning");
    if (!list.length) {
      box.hidden = true;
      return;
    }
    box.hidden = false;
    list.slice(0, 8).forEach((i) => {
      const d = el("div", "issue" + (i.severity === "error" ? " error" : ""));
      d.textContent = `${i.severity === "error" ? "错误" : "提示"}${i.rule_id ? ` [${i.rule_id}]` : ""} ${i.field}：${i.message}`;
      box.appendChild(d);
    });
  }

  function showIssues(issues, boxId, modalId) {
    const box = $(boxId);
    if (!box) return;
    box.innerHTML = "";
    (issues || []).slice(0, 6).forEach((i) => {
      const d = el("div", "issue" + (i.severity === "error" ? " error" : ""));
      d.textContent = `${i.severity === "error" ? "错误" : "提示"}${i.rule_id ? ` [${i.rule_id}]` : ""} ${i.field}：${i.message}`;
      box.appendChild(d);
    });
    box.hidden = false;
    if (modalId) $(modalId).hidden = false;
  }

  function renderDirtyState() {
    const p = project();
    $("ruleCounter").textContent = S.dirty
      ? `规则 ${p.rules.length} 条 · 有未保存修改`
      : `规则 ${p.rules.length} 条`;
    $("btnSaveRules").hidden = !S.dirty;
    $("btnDiscard").hidden = !S.dirty;
  }

  function actionText(a) {
    if (!a) return "—";
    if (a.type === "proxy") return `代理 → ${a.outbound || "(未填)"}`;
    if (a.type === "direct") return "直连 DIRECT";
    if (a.type === "reject") return "拒绝 REJECT";
    return a.type;
  }

  function renderRules() {
    const p = project();
    const body = $("rulesBody");
    body.innerHTML = "";
    $("rulesEmpty").hidden = p.rules.length > 0;

    p.rules.forEach((r, idx) => {
      const tr = el("tr");

      const tdOn = el("td");
      const cb = el("input");
      cb.type = "checkbox";
      cb.checked = !!r.enabled;
      cb.title = "启用/禁用这条规则（改完记得保存）";
      cb.addEventListener("change", () => {
        const next = clone(project());
        next.rules[idx].enabled = cb.checked;
        setDraft(next);
      });
      tdOn.appendChild(cb);
      tr.appendChild(tdOn);

      const tdName = el("td");
      tdName.appendChild(el("div", "rule-name", r.name || r.id));
      tdName.appendChild(el("div", "rule-sub", r.id));
      tr.appendChild(tdName);

      const tdMatch = el("td");
      const doms = (r.match && r.match.domain) || [];
      const ips = (r.match && r.match.ip_cidr) || [];
      const shown = [...doms.map((d) => ["d", d]), ...ips.map((i) => ["i", i])];
      shown.slice(0, 3).forEach(([k, v]) => tdMatch.appendChild(el("span", "match-chip" + (k === "i" ? " ip" : ""), v)));
      if (shown.length > 3) tdMatch.appendChild(el("span", "match-more", `+${shown.length - 3}`));
      if (!shown.length) tdMatch.appendChild(el("span", "rule-sub", "（无匹配条件）"));
      tr.appendChild(tdMatch);

      const tdAct = el("td");
      tdAct.appendChild(el("span", "rule-sub", actionText(r.action)));
      tr.appendChild(tdAct);

      const tdOps = el("td");
      const edit = el("button", "btn tiny ghost", "编辑");
      edit.addEventListener("click", () => openRuleModal(r.id));
      const del = el("button", "btn tiny ghost", "删除");
      del.style.marginLeft = "6px";
      del.addEventListener("click", async () => {
        const ok = await confirmDialog("删除规则", `确定删除 <b>${esc(r.name || r.id)}</b> 吗？<br/>删除只在点「保存规则」后写入磁盘。`);
        if (!ok) return;
        const next = clone(project());
        next.rules = next.rules.filter((x) => x.id !== r.id);
        setDraft(next);
      });
      tdOps.appendChild(edit);
      tdOps.appendChild(del);
      tr.appendChild(tdOps);

      body.appendChild(tr);
    });
  }

  function renderSettings() {
    const s = S.snap.settings;
    const focusInside = document.activeElement && document.activeElement.closest && document.activeElement.closest("#page-settings");
    if (focusInside) return;
    $("setStartMin").checked = !!s.start_minimized;
    $("setCloseToTray").checked = !!s.close_to_tray;
    $("setNotify").checked = !!s.show_notifications;
    $("setTarget").value = s.target;
    $("setTheme").value = THEME_MODES.indexOf(s.theme) >= 0 ? s.theme : "system";
    $("setBackups").value = s.backup_limit;
  }

  // ------------------------------------------------------------ 操作
  function clone(o) {
    return JSON.parse(JSON.stringify(o));
  }

  async function toggleEnabled(want) {
    const n = (project().rules || []).filter((r) => r.enabled).length;
    const ok = await confirmDialog(
      want ? "启用 RelayBridge" : "停用 RelayBridge",
      want
        ? `将把 <b>${n}</b> 条已启用规则写入 <code>relaybridge.js</code>（旧文件自动备份）。<br/>这不会修改 Clash Verge 的任何内容，也不代表 Clash 已生效。`
        : `将生成不再注入规则的新版本，并备份当前文件。<br/>已经加载进 Clash Verge 的规则，需要你重新加载脚本后才会消失。`
    );
    if (!ok) {
      renderHeader();
      return;
    }
    await guard(async () => {
      const snap = await api("set_enabled", { approved: true, enabled: want });
      S.snap = snap;
      renderAll();
      const last = snap.last;
      if (last && !last.ok) toast("状态已切换，但生成失败：" + last.message, "err", 6000);
      else toast(want ? "已启用，配置已生成" : "已停用，配置已生成");
    });
  }

  async function regenerate() {
    const ok = await confirmDialog(
      "重新生成配置",
      `将重新生成 <code>${esc(S.snap.paths.script)}</code>，旧版本自动备份到 backups/。`
    );
    if (!ok) return;
    await guard(async () => {
      const snap = await api("regenerate", { approved: true });
      S.snap = snap;
      renderAll();
      const last = snap.last;
      toast(last && last.ok ? "生成完成" : "生成失败：" + (last ? last.message : "未知原因"), last && last.ok ? "" : "err", 6000);
    });
  }

  async function saveProject() {
    const p = project();
    const ok = await confirmDialog(
      "保存规则并生成",
      `将写入 <code>project.json</code>（${p.rules.length} 条规则）并重新生成扩展脚本。<br/>继续？`
    );
    if (!ok) return;
    await guard(async () => {
      await api("save_project", { project: p, approved: true });
      S.draft = null;
      S.dirty = false;
      S.snap = await api("bootstrap");
      renderAll();
      const last = S.snap.last;
      toast(last && last.ok ? "已保存并生成" : "已保存，但生成失败：" + (last ? last.message : ""), last && last.ok ? "" : "err", 6000);
    });
  }

  async function saveSettings() {
    const next = clone(S.snap.settings);
    next.start_minimized = $("setStartMin").checked;
    next.close_to_tray = $("setCloseToTray").checked;
    next.show_notifications = $("setNotify").checked;
    next.target = $("setTarget").value;
    next.theme = THEME_MODES.indexOf($("setTheme").value) >= 0 ? $("setTheme").value : "system";
    next.backup_limit = Math.max(1, Math.min(200, parseInt($("setBackups").value || "20", 10)));
    const ok = await confirmDialog("保存设置", "将写入 <code>settings.json</code>。若开关 / 目标 / 主题有变化，会同时应用到窗口并重新生成配置。");
    if (!ok) return;
    await guard(async () => {
      const snap = await api("update_settings", { settings: next, approved: true });
      S.snap = snap;
      Theme.set(snap.settings.theme);
      renderAll();
      toast("设置已保存");
    });
  }

  // ------------------------------------------------------------ 规则弹窗
  function openRuleModal(id) {
    S.editingId = id || null;
    const r = id ? project().rules.find((x) => x.id === id) : null;
    $("ruleTitle").textContent = r ? "编辑规则" : "新建规则";
    $("fName").value = r ? r.name || "" : "";
    $("fDomains").value = r && r.match ? (r.match.domain || []).join("\n") : "";
    $("fCidrs").value = r && r.match ? (r.match.ip_cidr || []).join("\n") : "";
    const act = r && r.action ? r.action.type : "proxy";
    $("fAction").value = act;
    $("fGroup").value = r && r.action && r.action.type === "proxy" ? r.action.outbound || "" : "";
    syncGroupVisibility();
    $("ruleModal").hidden = false;
    $("fName").focus();
  }

  function syncGroupVisibility() {
    $("fGroupWrap").style.display = $("fAction").value === "proxy" ? "" : "none";
  }

  function nextRuleId(rules) {
    let n = rules.length + 1;
    const used = new Set(rules.map((r) => r.id));
    while (used.has("rule-" + String(n).padStart(3, "0"))) n += 1;
    return "rule-" + String(n).padStart(3, "0");
  }

  function applyRuleForm() {
    const p = clone(project());
    const domains = $("fDomains").value.split("\n").map((s) => s.trim()).filter(Boolean);
    const cidrs = $("fCidrs").value.split("\n").map((s) => s.trim()).filter(Boolean);
    if (!domains.length && !cidrs.length) {
      toast("至少填一个域名或一个 IP/CIDR", "warn");
      return;
    }
    const action = $("fAction").value;
    const actionObj = action === "proxy"
      ? { type: "proxy", outbound: $("fGroup").value.trim() }
      : { type: action };
    if (action === "proxy" && !actionObj.outbound) {
      toast("代理动作必须填策略组名称", "warn");
      return;
    }
    const name = $("fName").value.trim();
    if (S.editingId) {
      const idx = p.rules.findIndex((r) => r.id === S.editingId);
      if (idx >= 0) {
        p.rules[idx].name = name || p.rules[idx].id;
        p.rules[idx].match = { domain: domains, ip_cidr: cidrs };
        p.rules[idx].action = actionObj;
      }
    } else {
      const id = nextRuleId(p.rules);
      p.rules.push({
        id,
        name: name || id,
        enabled: true,
        match: { domain: domains, ip_cidr: cidrs },
        action: actionObj,
      });
    }
    $("ruleModal").hidden = true;
    setDraft(p);
    toast("已加入草稿，点「保存规则」落盘");
  }

  // ------------------------------------------------------------ JSON 弹窗
  function openJsonModal() {
    $("jsonText").value = JSON.stringify(project(), null, 2);
    $("jsonIssues").hidden = true;
    $("jsonIssues").innerHTML = "";
    $("jsonModal").hidden = false;
  }

  async function applyJson() {
    let parsed;
    try {
      parsed = JSON.parse($("jsonText").value);
    } catch (e) {
      showIssues([{ severity: "error", field: "json", message: "JSON 语法错误：" + e.message }], "jsonIssues", "jsonModal");
      return;
    }
    if (!parsed || typeof parsed !== "object" || !Array.isArray(parsed.rules)) {
      showIssues([{ severity: "error", field: "json", message: "缺少 rules 数组" }], "jsonIssues", "jsonModal");
      return;
    }
    const out = await api("preview", { project: parsed });
    const errors = (out.validation || []).filter((i) => i.severity === "error");
    if (errors.length) {
      showIssues(out.validation, "jsonIssues", "jsonModal");
      return;
    }
    $("jsonModal").hidden = true;
    setDraft(parsed);
    toast("JSON 已校验通过，点「保存规则」落盘");
  }

  // ------------------------------------------------------------ 事件绑定
  function bind() {
    document.querySelectorAll(".tab").forEach((t) => {
      t.addEventListener("click", () => switchTab(t.dataset.tab));
    });

    $("toggleEnabled").addEventListener("change", async (ev) => {
      const want = ev.target.checked;
      ev.target.checked = !want;
      await toggleEnabled(want);
    });

    $("btnRegen").addEventListener("click", () => guard(regenerate));
    $("btnOpenOut").addEventListener("click", () => guard(() => api("open_output_dir")));
    $("btnOpenOut2").addEventListener("click", () => guard(() => api("open_output_dir")));
    $("btnOpenData").addEventListener("click", () => guard(() => api("open_data_dir")));
    $("btnOpenBak").addEventListener("click", () => guard(() => api("open_backups_dir")));

    $("btnCopy").addEventListener("click", async () => {
      const text = $("previewBox").textContent || "";
      try {
        await navigator.clipboard.writeText(text);
        toast("已复制到剪贴板");
      } catch {
        toast("复制失败，可手动选中文本", "warn");
      }
    });

    $("btnRefreshPreview").addEventListener("click", () => guard(refreshPreview));
    $("btnAddRule").addEventListener("click", () => openRuleModal(null));
    $("btnSaveRules").addEventListener("click", () => guard(saveProject));
    $("btnDiscard").addEventListener("click", async () => {
      const ok = await confirmDialog("放弃修改", "未保存的规则改动会丢失，继续？");
      if (ok) resetDraft();
    });
    $("btnJson").addEventListener("click", openJsonModal);
    $("btnSaveSettings").addEventListener("click", () => guard(saveSettings));

    $("fAction").addEventListener("change", syncGroupVisibility);
    $("setTheme").addEventListener("change", () => {
      const mode = $("setTheme").value;
      Theme.set(mode); // 立即预览；点「保存设置」后才写进 settings.json
      toast(
        mode === "system" ? "主题：跟随系统（保存后生效于标题栏）" : `主题：${mode === "light" ? "浅色" : "深色"}（保存后生效于标题栏）`
      );
    });
    $("ruleCancel").addEventListener("click", () => ($("ruleModal").hidden = true));
    $("ruleOk").addEventListener("click", () => applyRuleForm());
    $("jsonCancel").addEventListener("click", () => ($("jsonModal").hidden = true));
    $("jsonOk").addEventListener("click", () => guard(applyJson));

    $("modalCancel").addEventListener("click", () => closeConfirm(false));
    $("modalOk").addEventListener("click", () => closeConfirm(true));
    $("modal").addEventListener("click", (e) => {
      if (e.target === $("modal")) closeConfirm(false);
    });

    $("btnClearLog").addEventListener("click", async () => {
      $("logBox").innerHTML = "";
      S.logCount = 0;
      $("logCount").textContent = "0";
      if (hasBackend()) await guard(async () => { S.snap = await api("clear_logs"); renderAll(); });
    });

    $("logAutoScroll").addEventListener("change", () => {
      if ($("logAutoScroll").checked) $("logBox").scrollTop = $("logBox").scrollHeight;
    });

    window.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        if (!$("modal").hidden) closeConfirm(false);
        $("ruleModal").hidden = true;
        $("jsonModal").hidden = true;
      }
    });

    window.addEventListener("error", (e) => {
      addLog({ ts: nowHms(), level: "error", source: "ui", message: String(e.message || e) });
    });
  }

  function switchTab(name) {
    document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.tab === name));
    document.querySelectorAll(".page").forEach((p) => p.classList.toggle("active", p.id === "page-" + name));
    if (name === "rules" && S.dirty) renderRules();
  }

  // ------------------------------------------------------------ 启动
  async function boot() {
    Theme.init();
    bind();
    if (!hasBackend()) {
      Theme.set("system");
      $("consistencyBanner").hidden = false;
      $("consistencyBanner").className = "subtle-banner err";
      $("consistencyBanner").textContent = "未检测到 Tauri 后端：请通过 RelayBridge 应用启动，而不是直接用浏览器打开 index.html。";
      $("previewBox").textContent = "（离线预览不可用）";
      return;
    }
    if (listen) {
      await listen("rb://log", (e) => addLog(e.payload));
      await listen("rb://state", (e) => {
        S.snap = e.payload;
        Theme.set(S.snap.settings.theme);
        if (!S.dirty) {
          renderAll();
        } else {
          renderHeader();
          renderCards();
          renderPaths();
          renderBanner();
        }
      });
      await listen("rb://navigate", (e) => switchTab(e.payload || "overview"));
    }
    await guard(async () => {
      S.snap = await api("bootstrap");
      Theme.set(S.snap.settings.theme);
      renderAll();
      const logs = await api("get_logs", { limit: 250 });
      logs.forEach(addLog);
    });
  }

  document.addEventListener("DOMContentLoaded", boot);
})();
