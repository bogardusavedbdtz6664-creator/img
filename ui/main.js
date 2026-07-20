import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";

const dropzone = document.getElementById("dropzone");
const fileList = document.getElementById("fileList");
const compressBtn = document.getElementById("compressBtn");
const clearBtn = document.getElementById("clearBtn");
const openOutBtn = document.getElementById("openOutBtn");
const formatSelect = document.getElementById("format");
const modeSelect = document.getElementById("mode");
const apiKeyInput = document.getElementById("apiKey");
const keyLabelInput = document.getElementById("keyLabel");
const addKeyBtn = document.getElementById("addKey");
const deleteKeyBtn = document.getElementById("deleteKey");
const keySelect = document.getElementById("keySelect");
const keyList = document.getElementById("keyList");
const keyStatus = document.getElementById("keyStatus");
const periodHint = document.getElementById("periodHint");
const results = document.getElementById("results");
const resultList = document.getElementById("resultList");
const zipDownload = document.getElementById("zipDownload");
const switchHint = document.getElementById("switchHint");
const progressBarWrap = document.getElementById("progressBarWrap");
const progressBar = document.getElementById("progressBar");
const progressText = document.getElementById("progressText");

/** @type {{ path: string, name: string, size?: number }[]} */
let selectedFiles = [];
let lastZip = null;
/** @type {{ name: string, status: string, error?: string, inputSize?: number, outputSize?: number, ratio?: number, outputPath?: string }[]} */
let progressItems = [];

function formatBytes(n) {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

function escapeHtml(s) {
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function isSupportedPath(p) {
  const ext = p.split(".").pop()?.toLowerCase();
  return ["png", "jpg", "jpeg", "webp", "avif"].includes(ext);
}

function syncFilesUI() {
  fileList.hidden = selectedFiles.length === 0;
  clearBtn.disabled = selectedFiles.length === 0;
  compressBtn.disabled = selectedFiles.length === 0;
  fileList.innerHTML = selectedFiles
    .map(
      (f) =>
        `<li><span>${escapeHtml(f.name)}</span><span class="meta">${
          f.size != null ? formatBytes(f.size) : ""
        }</span></li>`
    )
    .join("");
}

function addPaths(paths) {
  const map = new Map(selectedFiles.map((f) => [f.path, f]));
  for (const p of paths) {
    if (!isSupportedPath(p)) continue;
    const name = p.replace(/^.*[\\/]/, "");
    map.set(p, { path: p, name });
  }
  selectedFiles = [...map.values()];
  syncFilesUI();
}

function statusLabel(status) {
  switch (status) {
    case "pending":
      return "等待中";
    case "start":
    case "running":
      return "压缩中…";
    case "ok":
      return "完成";
    case "fail":
      return "失败";
    default:
      return status;
  }
}

function renderProgressList() {
  resultList.innerHTML = progressItems
    .map((item, i) => {
      const cls =
        item.status === "ok"
          ? "ok"
          : item.status === "fail"
            ? "fail"
            : item.status === "start" || item.status === "running"
              ? "running"
              : "pending";
      let meta = "";
      if (item.status === "ok") {
        meta = `${formatBytes(item.inputSize)} → ${formatBytes(item.outputSize)} · 节省 ${item.ratio}%`;
      } else if (item.status === "fail") {
        meta = escapeHtml(item.error || "压缩失败");
      }
      const action =
        item.status === "ok" && item.outputPath
          ? `<button type="button" class="btn ghost tiny reveal-one" data-path="${escapeHtml(
              item.outputPath
            )}">打开</button>`
          : `<span class="status-tag">${statusLabel(item.status)}</span>`;
      return `<li class="result-item ${cls}" data-index="${i}">
        <div>
          <div class="name">${escapeHtml(item.name)}</div>
          <div class="meta">${meta}</div>
        </div>
        ${action}
      </li>`;
    })
    .join("");
}

function updateOverallProgress() {
  const total = progressItems.length || 1;
  const done = progressItems.filter((i) => i.status === "ok" || i.status === "fail").length;
  const running = progressItems.findIndex((i) => i.status === "start" || i.status === "running");
  const pct = Math.round((done / total) * 100);
  progressBarWrap.hidden = false;
  progressText.hidden = false;
  progressBar.style.width = `${pct}%`;
  if (running >= 0) {
    progressText.textContent = `正在压缩 ${running + 1}/${total}：${progressItems[running].name}`;
  } else if (done === total) {
    progressText.textContent = `全部完成 ${done}/${total}`;
  } else {
    progressText.textContent = `进度 ${done}/${total}`;
  }
}

function initProgress(files) {
  progressItems = files.map((f) => ({
    name: f.name,
    status: "pending",
  }));
  results.hidden = false;
  zipDownload.hidden = true;
  switchHint.hidden = true;
  progressText.className = "hint";
  renderProgressList();
  updateOverallProgress();
}

function applyProgressEvent(p) {
  const item = progressItems[p.index];
  if (!item) return;
  item.status = p.status;
  if (p.error) item.error = p.error;
  if (p.inputSize != null) item.inputSize = p.inputSize;
  if (p.outputSize != null) item.outputSize = p.outputSize;
  if (p.ratio != null) item.ratio = p.ratio;
  if (p.outputPath) item.outputPath = p.outputPath;
  renderProgressList();
  updateOverallProgress();
}

function renderKeys(data) {
  const keys = data.keys || [];

  periodHint.textContent = `额度周期 ${data.quotaPeriod || "—"} · 每月 ${data.refreshDay || 1} 日刷新`;
  if (data.refresh?.refreshed) {
    keyStatus.textContent =
      data.refresh.cleared > 0
        ? `已进入新周期，已清除 ${data.refresh.cleared} 个「已用光」标记`
        : "已进入新的额度周期";
    keyStatus.className = "hint ok";
  }

  keySelect.innerHTML = keys.length
    ? keys
        .map((k) => {
          const tag = k.exhausted ? "（已用光）" : "";
          const label = k.label ? `${escapeHtml(k.label)} · ` : "";
          return `<option value="${k.id}" ${
            k.id === data.activeKeyId ? "selected" : ""
          } ${k.exhausted ? "disabled" : ""}>${label}${k.masked}${tag}</option>`;
        })
        .join("")
    : `<option value="">尚未添加 API Key</option>`;

  deleteKeyBtn.disabled = !keySelect.value;

  keyList.innerHTML = keys
    .map((k) => {
      const status = k.exhausted
        ? `<span class="badge bad">已用光</span>`
        : k.active
          ? `<span class="badge ok">使用中</span>`
          : `<span class="badge">可用</span>`;
      const label = k.label
        ? `<span class="key-label">${escapeHtml(k.label)}</span>`
        : "";
      return `<li data-id="${k.id}" class="${k.exhausted ? "exhausted" : ""}">
        <div class="key-main">
          <code>${escapeHtml(k.masked)}</code>
          ${label}
          ${status}
        </div>
        <button type="button" class="btn ghost tiny del-one" data-id="${k.id}">删除</button>
      </li>`;
    })
    .join("");

  if (!keys.length) {
    keyStatus.textContent = "请添加至少一个 TinyPNG API Key";
    keyStatus.className = "hint";
  } else if (!keys.some((k) => !k.exhausted)) {
    keyStatus.textContent = "全部 Key 本月已用光，可添加新 Key 或等待刷新日";
    keyStatus.className = "hint err";
  } else if (!data.refresh?.refreshed) {
    const active = keys.find((k) => k.id === data.activeKeyId);
    keyStatus.textContent = active
      ? `当前使用：${active.masked}${active.label ? `（${active.label}）` : ""}`
      : "请选择可用的 API Key";
    keyStatus.className = "hint ok";
  }
}

async function loadKeys() {
  const data = await invoke("list_keys");
  renderKeys(data);
}

addKeyBtn.addEventListener("click", async () => {
  const api_key = apiKeyInput.value.trim();
  if (!api_key) {
    keyStatus.textContent = "请输入 API Key";
    keyStatus.className = "hint err";
    return;
  }
  try {
    const data = await invoke("add_api_key", {
      apiKey: api_key,
      label: keyLabelInput.value.trim() || null,
    });
    apiKeyInput.value = "";
    keyLabelInput.value = "";
    renderKeys(data);
    keyStatus.textContent = "已添加 API Key";
    keyStatus.className = "hint ok";
  } catch (err) {
    keyStatus.textContent = String(err);
    keyStatus.className = "hint err";
  }
});

async function deleteKeyById(id) {
  if (!id) return;
  if (!confirm("确定删除该 API Key？")) return;
  try {
    const data = await invoke("remove_api_key", { id });
    renderKeys(data);
  } catch (err) {
    keyStatus.textContent = String(err);
    keyStatus.className = "hint err";
  }
}

deleteKeyBtn.addEventListener("click", () => deleteKeyById(keySelect.value));

keyList.addEventListener("click", (e) => {
  const btn = e.target.closest(".del-one");
  if (btn) deleteKeyById(btn.dataset.id);
});

keySelect.addEventListener("change", async () => {
  const id = keySelect.value;
  if (!id) return;
  try {
    const data = await invoke("set_active_api_key", { id });
    renderKeys(data);
  } catch (err) {
    keyStatus.textContent = String(err);
    keyStatus.className = "hint err";
  }
});

async function pickFiles() {
  const selected = await open({
    multiple: true,
    filters: [
      {
        name: "Images",
        extensions: ["png", "jpg", "jpeg", "webp", "avif"],
      },
    ],
  });
  if (!selected) return;
  const paths = Array.isArray(selected) ? selected : [selected];
  addPaths(paths);
}

dropzone.addEventListener("click", () => pickFiles());
dropzone.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    pickFiles();
  }
});

getCurrentWindow().onDragDropEvent((event) => {
  if (event.payload.type === "over") {
    dropzone.classList.add("dragover");
  } else if (event.payload.type === "leave" || event.payload.type === "drop") {
    dropzone.classList.remove("dragover");
  }
  if (event.payload.type === "drop") {
    addPaths(event.payload.paths || []);
  }
});

clearBtn.addEventListener("click", () => {
  selectedFiles = [];
  lastZip = null;
  progressItems = [];
  syncFilesUI();
  results.hidden = true;
  resultList.innerHTML = "";
  zipDownload.hidden = true;
  switchHint.hidden = true;
  progressBarWrap.hidden = true;
  progressText.hidden = true;
});

openOutBtn.addEventListener("click", async () => {
  try {
    await invoke("open_output_dir");
  } catch (err) {
    alert(String(err));
  }
});

zipDownload.addEventListener("click", async () => {
  if (!lastZip?.path) return;
  const dest = await save({
    defaultPath: lastZip.name || "compressed.zip",
    filters: [{ name: "ZIP", extensions: ["zip"] }],
  });
  if (!dest) return;
  try {
    await invoke("copy_zip_to", { source: lastZip.path, dest });
    zipDownload.textContent = "已保存压缩包";
  } catch (err) {
    alert(String(err));
  }
});

function showSwitches(switches) {
  if (!switches?.length) return;
  const names = switches.map((s) => s.masked).join("、");
  switchHint.hidden = false;
  switchHint.textContent = `检测到额度用尽，已自动切换 Key（标记用尽：${names}）`;
  switchHint.className = "hint";
}

function enableZip(zip) {
  if (!zip?.path) {
    zipDownload.hidden = true;
    return;
  }
  lastZip = zip;
  zipDownload.hidden = false;
  zipDownload.textContent = `一键打包下载（${zip.count} 张）`;
}

async function compressSequential(files, format) {
  const allSwitches = [];
  const successPaths = [];

  for (let i = 0; i < files.length; i++) {
    const file = files[i];
    compressBtn.textContent = `压缩中 ${i + 1}/${files.length}`;

    const data = await invoke("compress_images", {
      paths: [file.path],
      format,
      progressOffset: i,
      progressTotal: files.length,
      skipZip: true,
    });

    if (!data.ok && data.error) {
      if (data.keys) renderKeys(data.keys);
      applyProgressEvent({
        index: i,
        status: "fail",
        name: file.name,
        error: data.error,
      });
      continue;
    }

    if (data.keys) renderKeys(data.keys);
    if (data.switches?.length) allSwitches.push(...data.switches);

    const r = data.results?.[0];
    if (r?.ok && r.outputPath) successPaths.push(r.outputPath);
    // 进度主要由 compress-progress 事件更新；这里兜底一次
    if (r) {
      applyProgressEvent({
        index: i,
        status: r.ok ? "ok" : "fail",
        name: r.name || file.name,
        error: r.error,
        inputSize: r.inputSize,
        outputSize: r.outputSize,
        ratio: r.ratio,
        outputPath: r.outputPath,
      });
    }
  }

  if (successPaths.length) {
    const zip = await invoke("zip_paths", { paths: successPaths });
    enableZip(zip);
  }
  showSwitches(allSwitches);
}

async function compressBatch(files, format) {
  const data = await invoke("compress_images", {
    paths: files.map((f) => f.path),
    format,
  });

  if (!data.ok && data.error) {
    if (data.keys) renderKeys(data.keys);
    throw new Error(data.error);
  }
  if (data.keys) renderKeys(data.keys);

  // 若事件漏掉，用最终结果回填
  (data.results || []).forEach((r, index) => {
    applyProgressEvent({
      index,
      status: r.ok ? "ok" : "fail",
      name: r.name,
      error: r.error,
      inputSize: r.inputSize,
      outputSize: r.outputSize,
      ratio: r.ratio,
      outputPath: r.outputPath,
    });
  });

  enableZip(data.zip);
  showSwitches(data.switches);
}

compressBtn.addEventListener("click", async () => {
  if (!selectedFiles.length) return;
  compressBtn.disabled = true;
  clearBtn.disabled = true;
  lastZip = null;
  initProgress(selectedFiles);

  const format = formatSelect.value;
  const mode = modeSelect.value;

  try {
    if (mode === "sequential") {
      await compressSequential(selectedFiles, format);
    } else {
      compressBtn.textContent = "批量压缩中…";
      await compressBatch(selectedFiles, format);
    }
  } catch (err) {
    results.hidden = false;
    progressText.hidden = false;
    progressText.textContent = err.message || String(err);
    progressText.className = "hint err";
  } finally {
    compressBtn.textContent = "开始压缩";
    compressBtn.disabled = selectedFiles.length === 0;
    clearBtn.disabled = selectedFiles.length === 0;
  }
});

resultList.addEventListener("click", async (e) => {
  const btn = e.target.closest(".reveal-one");
  if (!btn?.dataset.path) return;
  try {
    await invoke("reveal_path", { path: btn.dataset.path });
  } catch (err) {
    alert(String(err));
  }
});

listen("compress-progress", (event) => {
  applyProgressEvent(event.payload);
}).catch(() => {});

loadKeys().catch((err) => {
  keyStatus.textContent = err.message || String(err);
  keyStatus.className = "hint err";
});
