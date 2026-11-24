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
    const mode = currentMode();
    document.querySelectorAll(".mode-btn").forEach((b) => b.classList.toggle("active", b.dataset.mode === mode));
    $("enabledSwitch").hidden = mode === "host";
    $("btnCopyTop").hidden = mode === "host";
    document.querySelectorAll(".tab").forEach((t) => {
      const forMode = t.dataset.for || "both";
      t.hidden = forMode !== "both" && forMode !== mode;
    });
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
    $("pScript").textContent = p.script;
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
      want ? "启用规则注入" : "停用规则注入",
      want
        ? `生成的脚本会把 <b>${n}</b> 条已启用规则插到最前面。<br/>不会写任何文件，点「复制」后自己粘贴进 Clash Verge。`
        : `生成的脚本不再注入任何规则。<br/>已经贴进 Clash Verge 的旧内容需要你手动重新粘贴才会消失。`
    );
    if (!ok) {
      renderHeader();
      return;
    }
    await guard(async () => {
      const snap = await api("set_enabled", { approved: true, enabled: want });
      S.snap = snap;
      renderAll();
      toast(want ? "已启用：复制预览即可使用" : "已停用：重新复制并粘贴");
    });
  }

  async function exportScript() {
    const ok = await confirmDialog(
      "导出到文件",
      `会把当前内容写到 <code>${esc(S.snap.paths.script)}</code>（旧文件自动备份）。<br/>只是给你一个文件副本，不会碰 Clash Verge。`
    );
    if (!ok) return;
    await guard(async () => {
      const snap = await api("regenerate", { approved: true });
      S.snap = snap;
      renderAll();
      const last = snap.last;
      toast(last && last.ok ? "已导出" : "导出失败：" + (last ? last.message : "未知原因"), last && last.ok ? "" : "err", 6000);
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

    // 顶部「Clash 规则 / 本机转发」选单
    $("modeSwitch").addEventListener("click", (e) => {
      const btn = e.target.closest(".mode-btn");
      if (!btn || btn.dataset.mode === currentMode()) return;
      const next = btn.dataset.mode;
      guard(async () => {
        S.snap = await api("set_mode", { mode: next });
        applyMode();
      });
    });

    $("toggleEnabled").addEventListener("change", async (ev) => {
      const want = ev.target.checked;
      ev.target.checked = !want;
      await toggleEnabled(want);
    });

    $("btnExport").addEventListener("click", () => guard(exportScript));
    $("btnOpenOut2").addEventListener("click", () => guard(() => api("open_output_dir")));
    $("btnOpenOut").addEventListener("click", () => guard(() => api("open_output_dir")));
    $("btnOpenData").addEventListener("click", () => guard(() => api("open_data_dir")));
    $("btnOpenBak").addEventListener("click", () => guard(() => api("open_backups_dir")));

    $("btnCopy").addEventListener("click", () => copyScript());
    $("btnCopyTop").addEventListener("click", () => copyScript());

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

    $("btnHostPresetUU").addEventListener("click", () =>
      guard(async () => {
        const p = await loadPresets();
        $("hostDomains").value = (p.uu_endpoints || []).join("\\n");
        HOST.dirty = true;
        toast("已填入 UU 远程端点");
      })
    );

    $("btnPresetUU").addEventListener("click", () =>
      guard(async () => {
        const p = await loadPresets();
        openRuleModal(null);
        $("fName").value = "UU 远程";
        $("fDomains").value = (p.uu_endpoints || []).join("\\n");
        $("fAction").value = "proxy";
        syncGroupVisibility();
        $("fGroup").focus();
      })
    );

    // Host 模式
    $("btnHostSave").addEventListener("click", () => guard(saveHostConfig));
    $("btnHostStart").addEventListener("click", () => guard(startHost));
    $("btnHostStop").addEventListener("click", () => guard(stopHost));
    $("btnHostApplyHosts").addEventListener("click", () => guard(applyHosts));
    $("btnHostClearHosts").addEventListener("click", () => guard(clearHosts));
    $("hostDomains").addEventListener("change", () => (HOST.dirty = true));

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

  // ------------------------------------------------------------ 日志栏拖拽
  const LogResize = {
    key: "uurelaybridge.logbar-h",
    init() {
      const saved = parseInt(localStorage.getItem(this.key) || "", 10);
      if (saved >= 120) this.apply(saved);
      const handle = $("logResize");
      const bar = document.querySelector(".logbar");
      let dragging = false;
      let startY = 0;
      let startH = 0;

      handle.addEventListener("pointerdown", (e) => {
        dragging = true;
        startY = e.clientY;
        startH = bar.getBoundingClientRect().height;
        try { handle.setPointerCapture(e.pointerId); } catch (_) {}
        handle.classList.add("active");
        document.body.classList.add("resizing");
        e.preventDefault();
      });
      handle.addEventListener("pointermove", (e) => {
        if (!dragging) return;
        const max = Math.round(window.innerHeight * 0.7);
        const h = Math.min(Math.max(120, startH - (e.clientY - startY)), max);
        this.apply(h);
      });
      const finish = () => {
        if (!dragging) return;
        dragging = false;
        handle.classList.remove("active");
        document.body.classList.remove("resizing");
        localStorage.setItem(this.key, String(Math.round(bar.getBoundingClientRect().height)));
      };
      handle.addEventListener("pointerup", finish);
      handle.addEventListener("pointercancel", finish);
      handle.addEventListener("dblclick", () => {
        this.apply(220);
        localStorage.setItem(this.key, "220");
      });
    },
    apply(px) {
      document.documentElement.style.setProperty("--logbar-h", px + "px");
    },
  };

  // ------------------------------------------------------------ Host 模式
  const HOST = { view: null, timer: null, dirty: false };
  let PRESETS = null;

  async function loadPresets() {
    if (PRESETS || !hasBackend()) return PRESETS;
    PRESETS = await api("get_presets");
    return PRESETS;
  }

  async function loadHost() {
    if (!hasBackend()) return;
    HOST.view = await api("host_get");
    HOST.dirty = false;
    renderHost();
  }

  function renderHost() {
    const v = HOST.view;
    if (!v) return;
    const { config: c, status: s } = v;
    if (!HOST.dirty) {
      $("hostPort").value = c.listen_port;
      $("hostMode").value = c.fragment_mode;
      $("hostParts").value = c.fragment_parts;
      $("hostDns").value = c.dns_server;
      $("hostDomains").value = (c.domains || []).join("\n");
      $("hostLinkClash").checked = !!c.link_clash;
    }
    const pill = $("hostStatePill");
    pill.className = "state-pill" + (s.running ? " on" : "");
    $("hostStateText").textContent = s.running ? "运行中" : "未启动";
    $("hostListen").textContent = s.listen || "—";
    $("hostHostsState").textContent = s.hosts_applied
      ? `已写入 ${s.hosts_domains.length} 条` + (s.can_write_hosts ? "" : "（需管理员）")
      : "未写入";
    $("hostStats").textContent = `${s.conns} / ${s.fragmented}` +
      (s.failed ? ` · 失败 ${s.failed}` : "") +
      ` · ↑${Math.round(s.bytes_up / 1024)}KB ↓${Math.round(s.bytes_down / 1024)}KB`;
    $("btnHostStart").disabled = s.running;
    $("btnHostStop").disabled = !s.running;
    $("btnHostApplyHosts").disabled = s.hosts_applied;
    $("btnHostClearHosts").disabled = !s.hosts_applied;
  }

  function hostFormConfig() {
    const v = HOST.view;
    const base = v ? clone(v.config) : {};
    return {
      ...base,
      listen_port: Math.max(1, Math.min(65535, parseInt($("hostPort").value || "443", 10))),
      fragment_mode: $("hostMode").value,
      fragment_parts: Math.max(2, Math.min(12, parseInt($("hostParts").value || "4", 10))),
      dns_server: $("hostDns").value.trim() || "223.5.5.5",
      link_clash: $("hostLinkClash").checked,
      domains: $("hostDomains").value
        .split("\n")
        .map((s) => s.trim())
        .filter(Boolean),
    };
  }

  async function saveHostConfig() {
    const cfg = hostFormConfig();
    const ok = await confirmDialog("保存 Host 配置", "将写入 <code>hostmode.json</code>（不改 hosts、不动系统）。");
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_set_config", { config: cfg, approved: true });
      HOST.dirty = false;
      renderHost();
      toast("Host 配置已保存");
    });
  }

  async function startHost() {
    const cfg = hostFormConfig();
    if (!cfg.domains.length) {
      toast("请先填写要走分片的域名", "warn");
      return;
    }
    const ok = await confirmDialog(
      "启动本地转发",
      `将在 <code>127.0.0.1:${cfg.listen_port}</code> 监听 TLS，并对 ${cfg.domains.length} 个域名做 ${
        cfg.fragment_mode === "record" ? "TLS Record 分片" : cfg.fragment_mode === "segment" ? "TCP 分段" : "纯转发"
      }。<br/>不改 hosts、不改系统设置。`
    );
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_set_config", { config: cfg, approved: true });
      HOST.view = await api("host_start", { approved: true });
      renderHost();
      toast("转发已启动");
    });
  }

  async function stopHost() {
    await guard(async () => {
      HOST.view = await api("host_stop");
      renderHost();
      toast("已停止监听");
    });
  }

  async function applyHosts() {
    const cfg = hostFormConfig();
    if (!cfg.domains.length) {
      toast("请先填写域名", "warn");
      return;
    }
    const ok = await confirmDialog(
      "写入系统 Hosts（需要管理员）",
      `将把以下域名指向 127.0.0.1：<br/><code>${cfg.domains.slice(0, 8).join("<br/>")}</code>` +
        (cfg.domains.length > 8 ? `<br/>…共 ${cfg.domains.length} 个` : "") +
        "<br/><br/>这一步会弹 UAC 提权，并且只有在本程序运行且转发已启动时这些域名才可用。"
    );
    if (!ok) return;
    await guard(async () => {
      await api("host_set_config", { config: cfg, approved: true });
      HOST.view = await api("host_apply_hosts", { approved: true });
      renderHost();
      toast("已请求写入 hosts（留意 UAC 与日志）", "", 5000);
    });
  }

  async function clearHosts() {
    const ok = await confirmDialog("移除 Hosts 记录", "会删除 UURelayBridge 写入的那一段，其它行原样保留。");
    if (!ok) return;
    await guard(async () => {
      HOST.view = await api("host_clear_hosts", { approved: true });
      renderHost();
      toast("已请求移除 hosts", "", 4000);
    });
  }

  function pollHost() {
    if (HOST.timer) clearInterval(HOST.timer);
    HOST.timer = setInterval(async () => {
      if (!hasBackend() || !document.getElementById("page-host").classList.contains("active")) return;
      try {
        HOST.view = await api("host_get");
        renderHost();
      } catch (e) {
        /* 忽略轮询错误 */
      }
    }, 1500);
  }

  function currentMode() {
    return S.snap && S.snap.settings.mode === "host" ? "host" : "clash";
  }

  function applyMode() {
    const mode = currentMode();
    document.querySelectorAll(".mode-btn").forEach((b) => b.classList.toggle("active", b.dataset.mode === mode));
    document.querySelectorAll(".tab").forEach((t) => {
      const forMode = t.dataset.for || "both";
      t.hidden = forMode !== "both" && forMode !== mode;
    });
    switchTab(mode === "host" ? "host" : "overview");
    if (mode === "host" && hasBackend()) {
      // 放到本轮 guard 结束之后，否则会被 S.busy 的重入保护吞掉
      setTimeout(() => {
        guard(loadHost);
        pollHost();
      }, 0);
    }
  }

  async function copyScript() {
    const text = ($("previewBox").textContent || "").trim();
    if (!text) { toast("还没有可复制的内容", "warn"); return; }
    try {
      await navigator.clipboard.writeText(text);
      toast("已复制，粘贴进 Clash Verge 的扩展脚本即可");
    } catch {
      toast("复制失败，可手动选中文本", "warn");
    }
  }

  function switchTab(name) {
    document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.tab === name));
    document.querySelectorAll(".page").forEach((p) => p.classList.toggle("active", p.id === "page-" + name));
    if (name === "rules" && S.dirty) renderRules();
    if (name === "host" && hasBackend()) {
      guard(loadHost);
      pollHost();
    }
  }

  // ------------------------------------------------------------ 启动
  async function boot() {
    Theme.init();
    LogResize.init();
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
    // 放在 guard 之外：applyMode 内部会再走一次 guard，嵌套会被 busy 标志挡掉
    applyMode();
  }

  document.addEventListener("DOMContentLoaded", boot);
})();
