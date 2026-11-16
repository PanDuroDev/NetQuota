import type { PopupState, SiteUsage, VideoStatus } from "./shared/types.js";

const app = document.getElementById("app")!;

function esc(value: string): string {
  return value.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}

function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let i = 0;
  while (value >= 1000 && i < units.length - 1) {
    value /= 1000;
    i += 1;
  }
  return `${value >= 10 || i === 0 ? Math.round(value) : value.toFixed(1)} ${units[i]}`;
}

function send<T>(message: Record<string, unknown>): Promise<T> {
  return chrome.runtime.sendMessage(message) as Promise<T>;
}

function pill(support: VideoStatus["quality"], label: string): string {
  const text = support === "best_effort" ? "best effort" : support;
  return `<span class="pill ${support}">${esc(label)}: ${esc(text)}</span>`;
}

const PROFILES: Array<[string, string]> = [
  ["normal", "Normal"],
  ["data_saver", "Saver"],
  ["strict_data_saver", "Strict"],
  ["gaming", "Gaming"],
];

const QUALITIES: Array<[number | null, string]> = [
  [null, "Off — never force a quality"],
  [144, "Ultra Saver · 144p"],
  [240, "Extreme Saver · 240p"],
  [360, "Balanced Saver · 360p"],
  [480, "Normal Saver · 480p"],
];

function siteLine(site: SiteUsage): string {
  const state = site.blocked ? "blocked" : site.exhausted ? "over quota" : "ok";
  return `<div class="site-line">
    <span class="host">${esc(site.host)}</span>
    <span class="used">${esc(formatBytes(site.down))}</span>
    <span class="used muted">${esc(site.limit === null ? "∞" : formatBytes(site.limit))}</span>
    <span class="pill ${site.blocked ? "unsupported" : "enforced"}">${esc(state)}</span>
  </div>`;
}

function render(state: PopupState): void {
  const limit = state.limit;
  const ratio = limit && limit > 0 ? Math.min(state.today_bytes / limit, 1) : 0;
  const barClass = state.today_bytes >= (limit ?? Infinity) ? "over" : ratio > 0.8 ? "warn" : "";
  const quality = state.video?.quality ?? "unsupported";
  const saving = state.video?.data_saving ?? "unsupported";
  const limitMb = limit === null ? "" : String(Math.round(limit / 1_000_000));
  const initial = (state.host.replace(/^www\./, "")[0] ?? "?").toUpperCase();
  const connected = state.connected;

  app.innerHTML = `
    <div class="top">
      <div class="brand"><img src="icon-48.png" alt="" /><span>NetQuota</span></div>
      <span class="status ${connected ? "on" : ""}">
        <span class="dot ${connected ? "on" : ""}"></span>
        ${connected ? `Service ${esc(state.service_version ?? "")}` : "Service offline"}
      </span>
    </div>

    <section class="site">
      <div class="site-head">
        <div class="avatar">${esc(initial)}</div>
        <div style="min-width:0">
          <div class="host">${esc(state.host || "No active site")}</div>
          <div class="small muted">
            ${
              limit === null
                ? "No daily quota · estimated"
                : `${esc(formatBytes(Math.max(limit - state.today_bytes, 0)))} left of ${esc(formatBytes(limit))}`
            }
          </div>
        </div>
        <div class="amount">${esc(formatBytes(state.today_bytes))}<small>estimated today</small></div>
      </div>
      ${limit === null ? "" : `<div class="bar ${barClass}"><span style="width:${(ratio * 100).toFixed(0)}%"></span></div>`}
      <div class="row">
        <button class="${state.blocked ? "secondary" : "danger"}" id="toggle" ${state.host ? "" : "disabled"}>
          ${state.blocked ? "Unblock" : "Block site"}
        </button>
        <button class="secondary" id="temp" ${state.host ? "" : "disabled"}>Allow 5 min</button>
        <span class="small muted" style="margin-inline-start:auto">MB/day</span>
        <input id="limit" type="number" min="0" step="10" placeholder="none" value="${esc(limitMb)}" ${state.host ? "" : "disabled"} />
        <button class="secondary" id="save" ${state.host ? "" : "disabled"}>Set</button>
      </div>
    </section>

    ${connected ? "" : `<section class="panel"><h2>Service offline</h2>
      <div class="small muted">Site blocking still works in Chrome. Per-application usage and Windows-level blocking need the NetQuota service — open the desktop app and press Install service.</div>
    </section>`}

    <section class="panel">
      <h2>Video data saver</h2>
      <div class="row" style="gap:6px;flex-wrap:wrap">
        ${pill(quality, "quality")} ${pill(saving, "playback")}
      </div>
      <select id="quality">
        ${QUALITIES.map(
          ([value, label]) =>
            `<option value="${value ?? ""}" ${
              (state.quality_height ?? null) === value ? "selected" : ""
            }>${esc(label)}</option>`,
        ).join("")}
      </select>
      <div class="small muted">${esc(state.video?.detail ?? "No adapter report for this site yet.")}</div>
    </section>

    <section class="panel">
      <h2>Profile across all apps</h2>
      <div class="segment">
        ${PROFILES.map(
          ([value, label]) =>
            `<button data-profile="${value}" class="${state.profile === value ? "active" : ""}">${esc(label)}</button>`,
        ).join("")}
      </div>
    </section>

    <section class="panel">
      <h2>Sites with activity</h2>
      <div class="sites">
        ${
          state.sites.length === 0
            ? `<div class="small muted">Nothing tracked yet.</div>`
            : state.sites.map(siteLine).join("")
        }
      </div>
    </section>

    ${state.last_error ? `<div class="small muted">Last error: ${esc(state.last_error)}</div>` : ""}
    <footer>Local only · site bytes are estimated from playback bitrate and resource timing</footer>
  `;

  document.getElementById("toggle")?.addEventListener("click", () => {
    void send({ type: "set_blocked", host: state.host, blocked: !state.blocked }).then(refresh);
  });
  document.getElementById("temp")?.addEventListener("click", () => {
    void send({ type: "temp_access", host: state.host }).then(refresh);
  });
  document.getElementById("save")?.addEventListener("click", () => {
    const raw = (document.getElementById("limit") as HTMLInputElement).value.trim();
    const mb = Number(raw);
    const bytes = raw === "" || Number.isNaN(mb) || mb <= 0 ? null : Math.round(mb * 1_000_000);
    void send({ type: "set_limit", host: state.host, limit: bytes }).then(refresh);
  });
  document.getElementById("quality")?.addEventListener("change", (event) => {
    const raw = (event.target as HTMLSelectElement).value;
    const height = raw === "" ? null : Number(raw);
    void send({ type: "set_quality", height }).then(refresh);
  });
  document.querySelectorAll<HTMLButtonElement>(".segment button").forEach((button) => {
    button.addEventListener("click", () => {
      void send({ type: "set_profile", profile: button.dataset.profile }).then(refresh);
    });
  });
}

async function refresh(): Promise<void> {
  const state = await send<PopupState>({ type: "popup_state" });
  render(state);
}

void refresh();
