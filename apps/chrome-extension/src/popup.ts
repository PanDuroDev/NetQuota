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
  return `<span class="pill ${support}">${esc(label)}: ${esc(support.replace("_", " "))}</span>`;
}

const PROFILES: Array<[string, string]> = [
  ["normal", "Normal"],
  ["data_saver", "Data Saver"],
  ["strict_data_saver", "Strict Data Saver"],
  ["gaming", "Gaming"],
];

const QUALITIES: Array<[number | null, string]> = [
  [null, "Off (no forcing)"],
  [144, "Ultra Saver · 144p"],
  [240, "Extreme Saver · 240p"],
  [360, "Balanced Saver · 360p"],
  [480, "Normal Saver · 480p"],
];

function siteRow(site: SiteUsage): string {
  const used = formatBytes(site.down);
  const limit = site.limit === null ? "unlimited" : formatBytes(site.limit);
  const state = site.blocked ? "blocked" : site.exhausted ? "over quota" : "ok";
  return `<tr>
    <td>${esc(site.host)}</td>
    <td>${esc(used)}</td>
    <td class="muted">${esc(limit)}</td>
    <td class="muted">${esc(state)}</td>
  </tr>`;
}

function render(state: PopupState): void {
  const limit = state.limit;
  const ratio = limit && limit > 0 ? Math.min(state.today_bytes / limit, 1) : 0;
  const barClass = state.today_bytes >= (limit ?? Infinity) ? "over" : ratio > 0.8 ? "warn" : "";
  const quality = state.video?.quality ?? "unsupported";
  const saving = state.video?.data_saving ?? "unsupported";
  const limitMb = limit === null ? "" : String(Math.round(limit / 1_000_000));

  app.innerHTML = `
    <div class="banner">
      <span class="dot ${state.connected ? "on" : ""}"></span>
      <span>${state.connected ? `Service connected · v${esc(state.service_version ?? "?")}` : "Desktop service offline"}</span>
      <span class="pill" style="margin-left:auto">${esc(state.monitoring ?? "unavailable")}</span>
    </div>
    ${state.connected ? "" : `<div class="small muted" style="margin-bottom:8px">Browser rules still apply. Per-app usage and ${
      state.enforcement_ready ? "service" : "system"
    } enforcement need the NetQuota service.</div>`}

    <div class="card">
      <div class="row">
        <div>
          <div class="muted small">This site today (estimated)</div>
          <div class="big">${esc(formatBytes(state.today_bytes))}</div>
        </div>
        <div style="margin-left:auto;text-align:right">
          <div class="muted small">${esc(state.host || "no site")}</div>
          <div>${limit === null ? "no quota" : esc(formatBytes(limit))}</div>
        </div>
      </div>
      ${limit === null ? "" : `<div class="bar ${barClass}"><span style="width:${(ratio * 100).toFixed(0)}%"></span></div>`}
      <div class="row" style="margin-top:10px">
        <button class="${state.blocked ? "secondary" : "danger"}" id="toggle" ${state.host ? "" : "disabled"}>
          ${state.blocked ? "Unblock site" : "Block site"}
        </button>
        <button class="secondary" id="temp" ${state.host ? "" : "disabled"}>Allow 5 min</button>
      </div>
      <div class="row" style="margin-top:8px">
        <span class="muted small">Daily limit (MB)</span>
        <input id="limit" type="number" min="0" step="10" value="${esc(limitMb)}" ${state.host ? "" : "disabled"} />
        <button class="secondary" id="save" ${state.host ? "" : "disabled"}>Save</button>
      </div>
      <div class="small muted" style="margin-top:6px">
        Site bytes are estimated: cross-origin media does not expose transfer sizes,
        so playback is modelled from bitrate. Treat this as a budget, not a meter.
      </div>
    </div>

    <h2>Video data saver</h2>
    <div class="card">
      <div class="row" style="gap:6px;flex-wrap:wrap">
        ${pill(quality, "quality")} ${pill(saving, "data saving")}
      </div>
      <div class="row" style="margin-top:8px">
        <span class="muted small">Preferred quality</span>
        <select id="quality" style="flex:1">
          ${QUALITIES.map(
            ([value, label]) =>
              `<option value="${value ?? ""}" ${
                (state.quality_height ?? null) === value ? "selected" : ""
              }>${esc(label)}</option>`,
          ).join("")}
        </select>
      </div>
      <div class="small muted" style="margin-top:6px">
        ${esc(state.video?.detail ?? "No adapter report yet for this site.")}
      </div>
    </div>

    <h2>Profile</h2>
    <div class="card">
      <select id="profile" style="width:100%">
        ${PROFILES.map(
          ([value, label]) =>
            `<option value="${value}" ${state.profile === value ? "selected" : ""}>${esc(label)}</option>`,
        ).join("")}
      </select>
      <div class="small muted" style="margin-top:6px">
        ${
          state.connected
            ? "Profiles are applied by the Windows service across all apps."
            : "Connect the desktop service to change profiles."
        }
      </div>
    </div>

    <h2>Sites with activity</h2>
    <table>
      <tr><th>Site</th><th>Used</th><th>Limit</th><th>State</th></tr>
      ${state.sites.length === 0 ? `<tr><td colspan="4" class="muted">Nothing tracked yet.</td></tr>` : state.sites.map(siteRow).join("")}
    </table>
    ${state.last_error ? `<p class="small muted">Last error: ${esc(state.last_error)}</p>` : ""}
    <p class="small muted">NetQuota is local-only. No browsing data leaves this machine.</p>
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
  document.getElementById("profile")?.addEventListener("change", (event) => {
    const profile = (event.target as HTMLSelectElement).value;
    void send({ type: "set_profile", profile }).then(refresh);
  });
}

async function refresh(): Promise<void> {
  const state = await send<PopupState>({ type: "popup_state" });
  render(state);
}

void refresh();
