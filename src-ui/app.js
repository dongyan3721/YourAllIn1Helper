// All-in-One Desktop Toolbox Frontend Controller (`src-ui/app.js`)
// Supports both Native Multi-Window WKWebView mode (`?mode=native&window=...`)
// and Unified Desktop Stage mode.

const urlParams = new URLSearchParams(window.location.search);
const isNativeWindowMode = urlParams.get("mode") === "native";
const nativeWindowType = urlParams.get("window") || ""; // "dashboard" | "clipboard-popup" | "sticky-note"
const nativeStickyId = urlParams.get("id") || "";

const state = {
  status: null,
  clipboardItems: [],
  stickyNotes: [],
  externalWindows: [],
  foregroundHandle: null,
  pinnedWindows: [],
  activeTab: "overview",
  clipSearch: "",
  popupSearch: "",
  editingStickyIds: new Set(),
};

async function invokeIpc(cmd, args = {}, silentLog = false) {
  const start = performance.now();
  const res = await fetch("/api/ipc", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ cmd, args }),
  });
  const payload = await res.json();
  const elapsed = (performance.now() - start).toFixed(1);
  if (!silentLog) {
    appendIpcLog(cmd, payload.ok, elapsed, payload.error);
  }
  if (!payload.ok) {
    console.error(`IPC Error [${cmd}]:`, payload.error);
    throw new Error(payload.error || "IPC failed");
  }
  return payload.data;
}

function appendIpcLog(cmd, ok, elapsedMs, err) {
  const container = document.getElementById("ipc-log");
  if (!container) return;
  const div = document.createElement("div");
  div.className = "ipc-log-item";
  const statusIcon = ok ? "✅" : "❌";
  div.innerHTML = `${statusIcon} <span class="cmd">${cmd}</span> <span>(${elapsedMs}ms)</span> ${
    err ? `<span style="color:#f87171">${escapeHtml(err)}</span>` : ""
  }`;
  container.prepend(div);
  while (container.children.length > 40) {
    container.removeChild(container.lastChild);
  }
}

function escapeHtml(str) {
  return String(str ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// Lightweight Markdown + LaTeX formula renderer for Sticky Notes
function renderMarkdownWithLatex(md) {
  let html = escapeHtml(md || "");
  html = html.replace(/\$\$([\s\S]+?)\$\$/g, (_, expr) => {
    return `<div class="math-block">∑ LaTeX: ${expr.trim()}</div>`;
  });
  html = html.replace(/\$([^\$\n]+?)\$/g, (_, expr) => {
    return `<span class="math-inline">${expr.trim()}</span>`;
  });
  html = html
    .replace(/^### (.+)$/gm, "<strong>$1</strong>")
    .replace(/^# (.+)$/gm, "<strong>$1</strong>")
    .replace(/^- \[x\] (.+)$/gm, "☑️ <del>$1</del>")
    .replace(/^- \[ \] (.+)$/gm, "⬜ $1")
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/\n/g, "<br/>");
  return html;
}

async function refreshAll(silent = false) {
  const [status, clipItems, notes, winpinData] = await Promise.all([
    invokeIpc("kernel_get_status", {}, silent),
    invokeIpc("clipboard_list_items", { keyword: state.clipSearch, limit: 100 }, silent),
    invokeIpc("sticky_list_notes", { only_open: false }, silent),
    invokeIpc("winpin_list_candidates", {}, silent),
  ]);

  state.status = status;
  state.clipboardItems = clipItems;
  state.stickyNotes = notes;
  state.externalWindows = winpinData.candidates || [];
  state.foregroundHandle = winpinData.foreground_handle;
  state.pinnedWindows = winpinData.pinned || [];

  if (!isNativeWindowMode) {
    renderTopBar();
    renderHud();
  }
  renderActiveWebViewWindows();
  if (!isNativeWindowMode || nativeWindowType === "dashboard") {
    renderDashboardContent();
  }
}

function renderTopBar() {
  const s = state.status;
  if (!s) return;
  document.getElementById("os-badge").textContent =
    `Target OS: ${s.target_os.toUpperCase()} | Paste Count: ${s.paste_trigger_count}`;

  const meter = document.getElementById("webview-meter");
  const label = document.getElementById("webview-count-label");
  if (s.active_window_count === 0) {
    meter.classList.remove("active-wv");
    label.textContent = "Active WebViews: 0 (Level 0 零窗口静默常驻 <25MB)";
  } else {
    meter.classList.add("active-wv");
    label.textContent = `Active WebViews: ${s.active_window_count} 个原生窗口运行中`;
  }
}

function renderHud() {
  const s = state.status;
  if (!s) return;

  document.getElementById("hud-win-count").textContent = String(s.active_window_count);
  const winListEl = document.getElementById("hud-window-list");
  if (s.active_windows.length === 0) {
    winListEl.innerHTML = `<div class="hud-hint">当前无任何 WebView 实例（内存占用最低 Level 0 状态）。可点击顶部快捷键或系统托盘唤醒窗口。</div>`;
  } else {
    winListEl.innerHTML = s.active_windows
      .map((entry) => {
        const isTransient = Boolean(entry.spec.policy && entry.spec.policy.transient);
        const policyBadge = isTransient ? "Transient" : "PinnedResident";
        return `
          <div class="wv-pill">
            <div class="wv-pill-meta">
              <strong>${escapeHtml(entry.spec.label)} <span style="color:#38bdf8">[${policyBadge}]</span></strong>
              <span class="wv-pill-route">${escapeHtml(entry.spec.route)} | Topmost: ${entry.always_on_top ? "YES" : "NO"}</span>
            </div>
            <button class="mini-btn" onclick="destroyWindowByLabel('${escapeHtml(entry.spec.label)}')">销毁</button>
          </div>
        `;
      })
      .join("");
  }

  const pickerActive = s.active_windows.some((w) => w.spec.label === "window-pin-picker");
  const stageEl = document.getElementById("desktop-stage");
  const bannerEl = document.getElementById("picker-banner");
  if (pickerActive) {
    stageEl.classList.add("picker-active");
    bannerEl.classList.remove("hidden");
  } else {
    stageEl.classList.remove("picker-active");
    bannerEl.classList.add("hidden");
  }

  document.getElementById("hud-pin-count").textContent = `${state.pinnedWindows.length} 已置顶`;
  const extContainer = document.getElementById("external-windows-stage");
  extContainer.innerHTML = state.externalWindows
    .map((w) => {
      const isFg = w.handle === state.foregroundHandle;
      const centerX = w.x + Math.floor(w.width / 2);
      const centerY = w.y + Math.floor(w.height / 2);
      return `
        <div class="ext-win-card ${isFg ? "is-fg" : ""} ${w.is_topmost ? "is-topmost" : ""}"
             onclick="onExternalWindowClicked(${w.handle}, ${centerX}, ${centerY})">
          <div class="ext-win-top">
            <span>${isFg ? "🟢 [当前活动] " : ""}${escapeHtml(w.title)}</span>
            ${w.is_topmost ? `<span class="deskpin-badge">📌 TOPMOST</span>` : ""}
          </div>
          <div class="ext-win-sub">
            <span>WinID: #${w.handle} | ${escapeHtml(w.process_name)} (PID ${w.process_id})</span>
            <button class="mini-btn" onclick="event.stopPropagation(); toggleWindowPinByHandle(${w.handle})">
              ${w.is_topmost ? "取消置顶" : "📌 置顶最前"}
            </button>
          </div>
        </div>
      `;
    })
    .join("");
}

function renderActiveWebViewWindows() {
  const s = state.status;
  if (!s) return;

  const dashWin = document.getElementById("win-dashboard");
  const clipWin = document.getElementById("win-clipboard-popup");
  const stickyContainer = document.getElementById("sticky-windows-container");

  if (isNativeWindowMode) {
    document.body.classList.add("native-single-window");
    if (nativeWindowType === "dashboard") {
      dashWin.classList.remove("hidden");
      clipWin.classList.add("hidden");
      stickyContainer.classList.add("hidden");
      return;
    }
    if (nativeWindowType === "clipboard-popup") {
      dashWin.classList.add("hidden");
      clipWin.classList.remove("hidden");
      stickyContainer.classList.add("hidden");
      renderClipboardPopupItems();
      return;
    }
    if (nativeWindowType === "sticky-note") {
      dashWin.classList.add("hidden");
      clipWin.classList.add("hidden");
      stickyContainer.classList.remove("hidden");
      const targetNote = state.stickyNotes.find((n) => n.id === nativeStickyId);
      if (targetNote && !state.editingStickyIds.has(targetNote.id)) {
        stickyContainer.innerHTML = renderStickyCardHtml(targetNote);
      }
      return;
    }
  }

  const activeLabels = new Set(s.active_windows.map((w) => w.spec.label));

  if (activeLabels.has("dashboard")) {
    dashWin.classList.remove("hidden");
  } else {
    dashWin.classList.add("hidden");
  }

  if (activeLabels.has("clipboard-popup")) {
    clipWin.classList.remove("hidden");
    renderClipboardPopupItems();
  } else {
    clipWin.classList.add("hidden");
  }

  const openNotes = state.stickyNotes.filter(
    (n) => n.is_open && activeLabels.has(`sticky-note-${n.id}`)
  );

  // Avoid overwriting textarea while user is actively typing in a sticky note
  if (state.editingStickyIds.size === 0) {
    stickyContainer.innerHTML = openNotes.map((n) => renderStickyCardHtml(n)).join("");
  }
}

function renderStickyCardHtml(n) {
  return `
    <div class="sticky-win theme-${escapeHtml(n.theme_color)}" id="sticky-card-${n.id}">
      <div class="sticky-top">
        <span class="wv-tag resident">${n.is_pinned ? "📌 已置顶最前" : "普通层级"}</span>
        <input type="text" class="sticky-title-inp" id="note-title-${n.id}"
               value="${escapeHtml(n.title)}"
               onfocus="state.editingStickyIds.add('${n.id}')"
               onblur="state.editingStickyIds.delete('${n.id}'); saveStickyNote('${n.id}')" />
        <select onchange="changeStickyTheme('${n.id}', this.value)" title="切换便笺配色">
          <option value="amber" ${n.theme_color === "amber" ? "selected" : ""}>琥珀黄</option>
          <option value="sky" ${n.theme_color === "sky" ? "selected" : ""}>天空蓝</option>
          <option value="emerald" ${n.theme_color === "emerald" ? "selected" : ""}>翡翠绿</option>
          <option value="rose" ${n.theme_color === "rose" ? "selected" : ""}>玫瑰红</option>
        </select>
        <button class="mini-btn" onclick="saveStickyNote('${n.id}')">💾 保存</button>
        <button class="wv-close" onclick="closeStickyNote('${n.id}')" title="关闭便笺并销毁此原生窗口">✕</button>
      </div>

      <div class="sticky-controls-bar">
        <label title="切换原生 NSWindow.Level.floating 真实窗口最前状态">
          <input type="checkbox" ${n.is_pinned ? "checked" : ""}
                 onchange="toggleStickyNotePin('${n.id}', this.checked)" />
          <span>📌 始终置顶最前 (Always on Top)</span>
        </label>
        <label title="调节原生窗口透明度 (alphaValue)">
          <span>透明度 ${Math.round(n.opacity * 100)}%</span>
          <input type="range" min="35" max="100" value="${Math.round(n.opacity * 100)}"
                 oninput="changeStickyOpacity('${n.id}', this.value)" />
        </label>
      </div>

      <textarea class="sticky-editor" id="note-md-${n.id}"
        placeholder="输入 Markdown 或 LaTeX 公式 ($E=mc^2$)..."
        onfocus="state.editingStickyIds.add('${n.id}')"
        onblur="state.editingStickyIds.delete('${n.id}'); saveStickyNote('${n.id}')"
        oninput="updateStickyPreview('${n.id}')">${escapeHtml(n.content_md)}</textarea>
      <div class="sticky-preview" id="note-prev-${n.id}">
        ${renderMarkdownWithLatex(n.content_md)}
      </div>
    </div>
  `;
}

function renderClipboardPopupItems() {
  const kw = state.popupSearch.trim().toLowerCase();
  const filtered = state.clipboardItems.filter((item) => {
    if (!kw) return true;
    return (
      (item.content_text || "").toLowerCase().includes(kw) ||
      (item.source_app_name || "").toLowerCase().includes(kw)
    );
  });

  document.getElementById("popup-paste-stat").textContent =
    `累计粘贴上屏 (Cmd+V): ${state.status ? state.status.paste_trigger_count : 0} 次`;

  const container = document.getElementById("popup-clip-items");
  if (filtered.length === 0) {
    container.innerHTML = `<div class="hud-hint">暂无匹配的剪贴板历史记录（在任意应用按 ⌘+C 复制文本即可自动捕获）</div>`;
    return;
  }

  container.innerHTML = filtered
    .map((item) => {
      return `
        <div class="clip-item ${item.is_pinned ? "pinned" : ""}">
          <div>
            <div class="clip-text">${escapeHtml(item.content_text)}</div>
            <div class="clip-meta">
              #${item.id} · 来源: ${escapeHtml(item.source_app_name || "macOS Clipboard")} ·
              ${item.is_pinned ? "📌 已固定(免淘汰)" : "LRU 动态"}
            </div>
          </div>
          <div class="clip-actions">
            <button class="mini-btn" onclick="toggleClipPin(${item.id})">${item.is_pinned ? "取消固定" : "📌 固定"}</button>
            <button class="primary-btn" onclick="pasteClipItem(${item.id})">粘贴上屏 ↵</button>
          </div>
        </div>
      `;
    })
    .join("");
}

function renderDashboardContent() {
  const s = state.status;
  if (!s) return;

  document.getElementById("overview-stats").innerHTML = `
    <div class="stat-box">
      <div class="label">活跃原生窗口 (WKWebView)</div>
      <div class="val">${s.active_window_count}</div>
    </div>
    <div class="stat-box">
      <div class="label">已挂载业务模块</div>
      <div class="val">${s.modules.length} / 3</div>
    </div>
    <div class="stat-box">
      <div class="label">剪贴板历史条数 (上限 ${s.config.clipboard.max_history_items})</div>
      <div class="val">${state.clipboardItems.length}</div>
    </div>
    <div class="stat-box">
      <div class="label">已置顶最前外部窗口</div>
      <div class="val">${state.pinnedWindows.length}</div>
    </div>
  `;

  const tbody = document.querySelector("#shortcuts-table tbody");
  tbody.innerHTML = s.shortcuts
    .map((sc) => {
      return `
        <tr>
          <td><code>${escapeHtml(sc.spec.module_id)}</code></td>
          <td><code>${escapeHtml(sc.spec.action_id)}</code></td>
          <td><code>${escapeHtml(sc.active_logical_hotkey)}</code></td>
          <td><kbd>${escapeHtml(sc.display_label)}</kbd> (<code>${escapeHtml(sc.physical_accelerator)}</code>)</td>
          <td>
            <button class="mini-btn" onclick="triggerShortcutAction('${escapeHtml(sc.spec.action_id)}')">⚡ 立即触发</button>
          </td>
        </tr>
      `;
    })
    .join("");

  document.getElementById("env-probe-list").innerHTML = (s.env_tools || [])
    .map((t) => {
      return `
        <div class="probe-card">
          <div><strong>${escapeHtml(t.tool)}</strong> ${t.available ? "✅ 已就绪" : "⚪ 未安装"}</div>
          <div style="color:#94a3b8;font-size:11px;margin-top:3px">
            ${escapeHtml(t.resolved_path || "未在 PATH 中探测到")}
          </div>
        </div>
      `;
    })
    .join("");

  document.getElementById("chk-suppress-clip").checked = Boolean(s.clipboard_suppressed);
  if (document.activeElement !== document.getElementById("inp-lru-max")) {
    document.getElementById("inp-lru-max").value = s.config.clipboard.max_history_items;
  }

  const dashClipEl = document.getElementById("dash-clip-list");
  dashClipEl.innerHTML = state.clipboardItems
    .map((item) => {
      return `
        <div class="clip-item ${item.is_pinned ? "pinned" : ""}">
          <div>
            <div class="clip-text">${escapeHtml(item.content_text)}</div>
            <div class="clip-meta">
              ID: #${item.id} | 来源: ${escapeHtml(item.source_app_name || "未知")} | Hash: ${item.content_hash.slice(0, 10)}…
            </div>
          </div>
          <div class="clip-actions">
            <button class="mini-btn" onclick="toggleClipPin(${item.id})">${item.is_pinned ? "取消固定" : "📌 固定"}</button>
            <button class="mini-btn" onclick="pasteClipItem(${item.id})">写入系统剪贴板并粘贴</button>
            <button class="mini-btn" onclick="deleteClipItem(${item.id})">删除</button>
          </div>
        </div>
      `;
    })
    .join("");

  const notesWrap = document.getElementById("dash-notes-list");
  notesWrap.innerHTML = `
    <table class="data-table">
      <thead>
        <tr>
          <th>标题</th>
          <th>配色与透明度</th>
          <th>真实窗口最前状态</th>
          <th>桌面窗口生命周期</th>
          <th>操作</th>
        </tr>
      </thead>
      <tbody>
        ${state.stickyNotes
          .map(
            (n) => `
          <tr>
            <td><strong>${escapeHtml(n.title)}</strong></td>
            <td>${escapeHtml(n.theme_color)} (${Math.round(n.opacity * 100)}%)</td>
            <td>${n.is_pinned ? `<span class="deskpin-badge">📌 窗口最前 (.floating)</span>` : "普通层级 (.normal)"}</td>
            <td>${
              n.is_open
                ? `<span style="color:#22c55e">🟢 独立原生窗口运行中</span>`
                : `<span style="color:#94a3b8">⚪ 已销毁关闭 (0 内存占用)</span>`
            }</td>
            <td>
              ${
                n.is_open
                  ? `<button class="mini-btn" onclick="closeStickyNote('${n.id}')">关闭并销毁窗口</button>`
                  : `<button class="mini-btn" onclick="openStickyNote('${n.id}')">唤醒独立置顶窗口</button>`
              }
              <button class="mini-btn" onclick="deleteStickyNote('${n.id}')">彻底删除</button>
            </td>
          </tr>
        `
          )
          .join("")}
      </tbody>
    </table>
  `;

  const winpinWrap = document.getElementById("dash-winpin-list");
  winpinWrap.innerHTML = `
    <p class="pane-desc" style="margin-bottom:8px">
      以下列表实时同步自 macOS <code>CGWindowListCopyWindowInfo</code> 真实桌面窗口。点击「📌 置顶最前」将在该窗口右上角挂载原生置顶红标并开启防遮挡实时置顶守护；也可以按全局快捷键 <kbd>⌘+⌥+P</kbd> 在屏幕上直接用准星点击任意真实窗口将其置顶。
    </p>
    <table class="data-table">
      <thead>
        <tr>
          <th>真实窗口 ID</th>
          <th>应用进程</th>
          <th>窗口标题</th>
          <th>屏幕坐标区域</th>
          <th>置顶状态</th>
          <th>操作</th>
        </tr>
      </thead>
      <tbody>
        ${state.externalWindows
          .map((w) => {
            const isFg = w.handle === state.foregroundHandle;
            return `
              <tr>
                <td><code>#${w.handle}</code></td>
                <td><strong>${escapeHtml(w.process_name)}</strong> (PID ${w.process_id})</td>
                <td>${escapeHtml(w.title)} ${isFg ? `<span class="badge">焦点窗口</span>` : ""}</td>
                <td>(${w.x}, ${w.y}) ${w.width}×${w.height}</td>
                <td>${w.is_topmost ? `<span class="deskpin-badge">📌 置顶最前生效中</span>` : "普通层级"}</td>
                <td>
                  <button class="primary-btn" onclick="toggleWindowPinByHandle(${w.handle})">${
              w.is_topmost ? "取消置顶" : "📌 置顶最前"
            }</button>
                </td>
              </tr>
            `;
          })
          .join("")}
      </tbody>
    </table>
  `;
}

// =========================================================================
// Interactive Handlers
// =========================================================================

async function triggerShortcutAction(actionId) {
  await invokeIpc("kernel_trigger_shortcut", { action_id: actionId });
  await refreshAll();
}

async function destroyWindowByLabel(label) {
  if (label.startsWith("sticky-note-")) {
    const noteId = label.replace("sticky-note-", "");
    await invokeIpc("sticky_close_note", { id: noteId });
  } else {
    await invokeIpc("kernel_destroy_window", { label });
  }
  await refreshAll();
}

async function toggleClipPin(id) {
  await invokeIpc("clipboard_toggle_pin", { id });
  await refreshAll();
}

async function pasteClipItem(id) {
  await invokeIpc("clipboard_paste_item", { id, as_plain_text: true });
  await refreshAll();
}

async function deleteClipItem(id) {
  await invokeIpc("clipboard_delete_item", { id });
  await refreshAll();
}

function updateStickyPreview(noteId) {
  const mdEl = document.getElementById(`note-md-${noteId}`);
  const prevEl = document.getElementById(`note-prev-${noteId}`);
  if (mdEl && prevEl) {
    prevEl.innerHTML = renderMarkdownWithLatex(mdEl.value);
  }
}

async function saveStickyNote(noteId) {
  const titleEl = document.getElementById(`note-title-${noteId}`);
  const mdEl = document.getElementById(`note-md-${noteId}`);
  if (!mdEl) return;
  await invokeIpc("sticky_save_content", {
    id: noteId,
    title: titleEl ? titleEl.value : undefined,
    content_md: mdEl.value,
  });
  await refreshAll(true);
}

async function toggleStickyNotePin(noteId, isPinned) {
  await invokeIpc("sticky_update_window_state", {
    id: noteId,
    is_pinned: Boolean(isPinned),
  });
  await refreshAll();
}

async function changeStickyOpacity(noteId, opacityPercent) {
  const opacity = Math.max(0.35, Math.min(1.0, Number(opacityPercent) / 100.0));
  const card = document.getElementById(`sticky-card-${noteId}`);
  if (card && !isNativeWindowMode) {
    card.style.opacity = String(opacity);
  }
  await invokeIpc(
    "sticky_update_window_state",
    {
      id: noteId,
      opacity,
    },
    true
  );
  await refreshAll(true);
}

async function changeStickyTheme(noteId, newColor) {
  await saveStickyNote(noteId);
  await invokeIpc("sticky_update_theme", {
    id: noteId,
    theme_color: newColor,
  });
  state.editingStickyIds.delete(noteId);
  await refreshAll();
}

async function closeStickyNote(noteId) {
  state.editingStickyIds.delete(noteId);
  await invokeIpc("sticky_close_note", { id: noteId });
  await refreshAll();
}

async function openStickyNote(noteId) {
  await invokeIpc("sticky_open_note", { id: noteId });
  await refreshAll();
}

async function deleteStickyNote(noteId) {
  state.editingStickyIds.delete(noteId);
  await invokeIpc("sticky_delete_note", { id: noteId });
  await refreshAll();
}

async function onExternalWindowClicked(handle, centerX, centerY) {
  const pickerActive =
    state.status &&
    state.status.active_windows.some((w) => w.spec.label === "window-pin-picker");
  if (pickerActive) {
    await invokeIpc("winpin_pick_at_cursor", { x: centerX, y: centerY });
  } else {
    await invokeIpc("winpin_set_foreground", { handle });
  }
  await refreshAll();
}

async function setForegroundWindow(handle) {
  await invokeIpc("winpin_set_foreground", { handle });
  await refreshAll();
}

async function toggleWindowPinByHandle(handle) {
  await invokeIpc("winpin_toggle_handle", { handle });
  await refreshAll();
}

// Bind DOM Listeners
window.addEventListener("DOMContentLoaded", async () => {
  document.getElementById("btn-hk-clipboard").addEventListener("click", () =>
    triggerShortcutAction("clipboard:toggle_popup")
  );
  document.getElementById("btn-hk-sticky").addEventListener("click", () =>
    triggerShortcutAction("sticky_note:create")
  );
  document.getElementById("btn-hk-winpin-fg").addEventListener("click", () =>
    triggerShortcutAction("window_pin:toggle_foreground")
  );
  document.getElementById("btn-hk-winpin-pick").addEventListener("click", () =>
    triggerShortcutAction("window_pin:pick_cursor")
  );

  document.getElementById("btn-open-dashboard").addEventListener("click", async () => {
    await invokeIpc("kernel_open_dashboard", { route: "/overview" });
    await refreshAll();
  });

  document.getElementById("btn-close-dashboard").addEventListener("click", async () => {
    await invokeIpc("kernel_destroy_window", { label: "dashboard" });
    await refreshAll();
  });

  document.getElementById("btn-close-clip-popup").addEventListener("click", async () => {
    await invokeIpc("kernel_destroy_window", { label: "clipboard-popup" });
    await refreshAll();
  });

  document.getElementById("btn-blur-clip-popup").addEventListener("click", async () => {
    await invokeIpc("kernel_blur_window", { label: "clipboard-popup" });
    await refreshAll();
  });

  document.getElementById("btn-cancel-picker").addEventListener("click", async () => {
    await invokeIpc("kernel_blur_window", { label: "window-pin-picker" });
    await refreshAll();
  });

  document.getElementById("btn-shutdown").addEventListener("click", async () => {
    await invokeIpc("kernel_shutdown");
    await refreshAll();
  });

  document.getElementById("btn-clear-log").addEventListener("click", () => {
    document.getElementById("ipc-log").innerHTML = "";
  });

  document.querySelectorAll(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll(".nav-item").forEach((b) => b.classList.remove("active"));
      document.querySelectorAll(".tab-pane").forEach((p) => p.classList.remove("active"));
      btn.classList.add("active");
      const tab = btn.getAttribute("data-tab");
      document.getElementById(`tab-${tab}`).classList.add("active");
    });
  });

  document.getElementById("btn-capture-clip").addEventListener("click", async () => {
    const inp = document.getElementById("inp-clip-text");
    const appSel = document.getElementById("sel-clip-app");
    if (!inp.value.trim()) return;
    await invokeIpc("clipboard_capture_text", {
      text: inp.value,
      source_app_name: appSel.value,
    });
    inp.value = "";
    await refreshAll();
  });

  document.getElementById("btn-clear-unpinned").addEventListener("click", async () => {
    await invokeIpc("clipboard_clear_unpinned");
    await refreshAll();
  });

  document.getElementById("chk-suppress-clip").addEventListener("change", async (e) => {
    await invokeIpc("clipboard_set_suppression", {
      suppressed: e.target.checked,
      reason: "ui_manual_toggle",
    });
    await refreshAll();
  });

  document.getElementById("btn-save-lru").addEventListener("click", async () => {
    const maxVal = parseInt(document.getElementById("inp-lru-max").value, 10) || 100;
    await invokeIpc("kernel_update_clipboard_config", {
      max_history_items: maxVal,
      enabled: true,
    });
    await refreshAll();
  });

  document.getElementById("inp-clip-search").addEventListener("input", async (e) => {
    state.clipSearch = e.target.value;
    await refreshAll(true);
  });

  const popupSearchEl = document.getElementById("inp-popup-search");
  popupSearchEl.addEventListener("input", (e) => {
    state.popupSearch = e.target.value;
    renderClipboardPopupItems();
  });
  popupSearchEl.addEventListener("keydown", async (e) => {
    if (e.key === "Enter" && state.clipboardItems.length > 0) {
      await pasteClipItem(state.clipboardItems[0].id);
    }
  });

  document.getElementById("btn-dash-new-note").addEventListener("click", async () => {
    await invokeIpc("sticky_create_note", {
      title: "新建置顶便笺",
      content_md: "支持 **Markdown** 与行内公式 $\\alpha + \\beta = \\gamma$\n\n该便笺运行在独立的原生 macOS 置顶窗口中！",
      theme_color: "sky",
    });
    await refreshAll();
  });

  document.getElementById("btn-dash-pin-fg").addEventListener("click", async () => {
    await invokeIpc("winpin_toggle_foreground");
    await refreshAll();
  });

  document.getElementById("btn-dash-restore-pins").addEventListener("click", async () => {
    await invokeIpc("winpin_restore_all");
    await refreshAll();
  });

  await refreshAll();

  // Live poll so native multi-window changes, global hotkeys, and system clipboard copies stay in sync
  setInterval(() => {
    refreshAll(true).catch(() => {});
  }, 600);
});
