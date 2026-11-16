//! NetQuota desktop UI. Vanilla TypeScript on purpose: no framework bundle, no
//! remote assets, and nothing that needs a build server.

export {};

type Period = "daily" | "weekly" | "monthly";
type Enforcement = "block" | "notify_only" | "block_background";

interface Usage {
  down: number;
  up: number;
}

interface AppUsage {
  identity: { key: string; exe_name: string; path: string | null; publisher: string | null };
  pid: number;
  category: string;
  down: number;
  up: number;
  speed_down_bps: number;
  speed_up_bps: number;
  daily: Usage;
  monthly: Usage;
  period_used: Usage;
  limit: { total: number | null; download: number | null; upload: number | null };
  remaining_total: number | null;
  exhausted: boolean;
  paused: boolean;
  blocked: boolean;
  background: boolean;
  confidence: string;
  protection_note: string | null;
  last_seen_ms: number;
}

interface SiteUsage {
  host: string;
  down: number;
  up: number;
  estimated: boolean;
  active_secs: number;
  tabs: number;
  limit: number | null;
  remaining_total: number | null;
  exhausted: boolean;
  blocked: boolean;
}

interface EventRecord {
  id: number;
  ts_ms: number;
  kind: string;
  subject: string;
  message: string;
  bytes: number | null;
}

interface Snapshot {
  generated_at_ms: number;
  service_version: string;
  monitoring: string;
  enforcement_ready: boolean;
  global: { down: number; up: number; today_down: number; today_up: number; down_bps: number; up_bps: number };
  apps: AppUsage[];
  sites: SiteUsage[];
  profile: string;
  profile_video_height: number | null;
  metered: boolean;
  interface: string | null;
  events: EventRecord[];
  unattributed_bytes: number;
}

interface Health {
  connected: boolean;
  service_version: string | null;
  monitoring: string | null;
  enforcement_ready: boolean;
  protocol_version: number;
  pipe: string;
}

interface FallbackUsage {
  down: number;
  up: number;
  primary_alias: string | null;
  primary_kind: string | null;
}

declare global {
  interface Window {
    __TAURI__: { core: { invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> } };
  }
}

const invoke = <T>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  window.__TAURI__.core.invoke<T>(cmd, args);

const T: Record<string, Record<string, string>> = {
  en: {
    "nav.dashboard": "Dashboard",
    "nav.apps": "Applications",
    "nav.sites": "Websites",
    "nav.saver": "Data Saver",
    "nav.events": "Notifications",
    "nav.settings": "Settings",
    "btn.refresh": "Refresh",
    "btn.pause": "Pause",
    "btn.resume": "Resume",
    "btn.block": "Block",
    "btn.unblock": "Unblock",
    "btn.edit": "Policy",
    "btn.save": "Save",
    "btn.cancel": "Cancel",
    "card.today": "Used today",
    "card.speed": "Current speed",
    "card.active": "Active apps",
    "card.restricted": "Restricted",
    "card.interface": "Interface",
    "card.profile": "Profile",
    "card.month": "This month",
    "table.app": "Application",
    "table.used": "Downloaded",
    "table.up": "Uploaded",
    "table.quota": "Quota",
    "table.speed": "Speed",
    "table.state": "State",
    "table.site": "Site",
    "table.actions": "Actions",
    "state.ok": "Allowed",
    "state.paused": "Paused",
    "state.blocked": "Blocked",
    "state.over": "Over quota",
    "state.bg": "Background",
    "dash.top": "Top applications today",
    "dash.empty": "No per-application data yet. Install and start the service, or wait for the first polling cycle.",
    "apps.search": "Search applications",
    "apps.empty": "No applications detected yet.",
    "site.estimated": "estimated",
    "sites.empty": "No websites reported by the extension yet.",
    "saver.title": "Profiles change several policies at once",
    "saver.quality": "Preferred video quality",
    "saver.note": "Video quality is requested by the extension and verified against the player. Sites that expose no control are reported as unsupported instead of pretending to be limited.",
    "events.empty": "No events yet.",
    "settings.service": "Service",
    "settings.pipe": "Pipe",
    "settings.recover": "Remove all NetQuota restrictions",
    "settings.recoverHelp": "Removes every firewall rule NetQuota owns. Use it if a crash left an app blocked.",
    "settings.install": "Service installation",
    "settings.installHelp": "Press Install service and confirm the Windows prompt. It registers a LocalSystem service; this window keeps running without administrator rights.",
    "settings.offline": "Service offline — showing interface totals only.",
    "svc.install": "Install service (asks for admin)",
    "svc.retry": "Retry",
    "svc.installing": "Installing…",
    "svc.hint": "Installing registers a LocalSystem service so per-application usage and blocking work. This window stays unelevated.",
    "trust.recover": "Removed {n} NetQuota rules.",
    "trust.offline": "The service is not running. Per-application data and enforcement are unavailable.",
  },
  ar: {
    "nav.dashboard": "لوحة التحكم",
    "nav.apps": "التطبيقات",
    "nav.sites": "المواقع",
    "nav.saver": "موفر البيانات",
    "nav.events": "الإشعارات",
    "nav.settings": "الإعدادات",
    "btn.refresh": "تحديث",
    "btn.pause": "إيقاف مؤقت",
    "btn.resume": "استئناف",
    "btn.block": "حظر",
    "btn.unblock": "إلغاء الحظر",
    "btn.edit": "السياسة",
    "btn.save": "حفظ",
    "btn.cancel": "إلغاء",
    "card.today": "المستخدم اليوم",
    "card.speed": "السرعة الحالية",
    "card.active": "تطبيقات نشطة",
    "card.restricted": "مقيّدة",
    "card.interface": "واجهة الشبكة",
    "card.profile": "الملف",
    "card.month": "هذا الشهر",
    "table.app": "التطبيق",
    "table.used": "التنزيل",
    "table.up": "الرفع",
    "table.quota": "الحصة",
    "table.speed": "السرعة",
    "table.state": "الحالة",
    "table.site": "الموقع",
    "table.actions": "إجراءات",
    "state.ok": "مسموح",
    "state.paused": "متوقف",
    "state.blocked": "محظور",
    "state.over": "تجاوز الحصة",
    "state.bg": "في الخلفية",
    "dash.top": "أكثر التطبيقات استهلاكاً اليوم",
    "dash.empty": "لا توجد بيانات لكل تطبيق بعد. ثبّت الخدمة وشغّلها أو انتظر أول دورة جمع.",
    "apps.search": "بحث في التطبيقات",
    "apps.empty": "لم يتم اكتشاف تطبيقات بعد.",
    "site.estimated": "تقديري",
    "sites.empty": "لم تُرسل الإضافة أي مواقع بعد.",
    "saver.title": "الملفات تغيّر عدة سياسات معاً",
    "saver.quality": "جودة الفيديو المفضلة",
    "saver.note": "تطلب الإضافة جودة الفيديو وتتحقق منها من المشغّل. المواقع التي لا توفر تحكماً تُصنّف غير مدعومة بدل الادعاء بتقييدها.",
    "events.empty": "لا توجد أحداث بعد.",
    "settings.service": "الخدمة",
    "settings.pipe": "الأنبوب",
    "settings.recover": "إزالة كل قيود NetQuota",
    "settings.recoverHelp": "يحذف كل قواعد جدار الحماية التي تملكها NetQuota. استخدمه إذا تسبب عطل في حظر تطبيق.",
    "settings.install": "تثبيت الخدمة",
    "settings.installHelp": "اضغط تثبيت الخدمة ثم أكّد رسالة ويندوز. سيُنشئ خدمة نظام، وتبقى هذه النافذة بدون صلاحيات إدارية.",
    "settings.offline": "الخدمة غير متصلة — تُعرض إحصاءات الواجهة فقط.",
    "svc.install": "تثبيت الخدمة (يطلب صلاحية المدير)",
    "svc.retry": "إعادة المحاولة",
    "svc.installing": "جارٍ التثبيت…",
    "svc.hint": "التثبيت ينشئ خدمة نظام لتعمل إحصاءات كل تطبيق والحجب. هذه النافذة تبقى بدون صلاحيات إدارية.",
    "trust.recover": "أُزيلت {n} قاعدة NetQuota.",
    "trust.offline": "الخدمة لا تعمل. بيانات التطبيقات والتنفيذ غير متاحة.",
  },
};

let lang: "en" | "ar" = "en";
let theme: "dark" | "light" = "dark";
let page = "dashboard";
let snapshot: Snapshot | null = null;
let health: Health | null = null;
let installBusy = false;
let fallback: FallbackUsage | null = null;
let editorKey: string | null = null;
let lastError = "";
const icons = new Map<string, string>();
const iconPending = new Set<string>();

const t = (key: string): string => T[lang][key] ?? T.en[key] ?? key;

function esc(value: string): string {
  return value.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function bytes(value: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = Math.abs(value);
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i += 1;
  }
  const text = `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  return value < 0 ? `-${text}` : text;
}

function speed(value: number): string {
  return `${bytes(value)}/s`;
}

function dateTime(ms: number): string {
  return new Date(ms).toLocaleString(lang === "ar" ? "ar" : undefined);
}

function barClass(ratio: number): string {
  return ratio >= 1 ? "over" : ratio > 0.8 ? "warn" : "";
}

function appState(app: AppUsage): [string, string] {
  if (app.blocked) return [t("state.blocked"), "bad"];
  if (app.paused) return [t("state.paused"), "warn"];
  if (app.exhausted) return [t("state.over"), "bad"];
  if (app.background) return [t("state.bg"), ""];
  return [t("state.ok"), "ok"];
}

function renderNav(): void {
  const items: Array<[string, string]> = [
    ["dashboard", t("nav.dashboard")],
    ["apps", t("nav.apps")],
    ["sites", t("nav.sites")],
    ["saver", t("nav.saver")],
    ["events", t("nav.events")],
    ["settings", t("nav.settings")],
  ];
  document.getElementById("nav")!.innerHTML = items
    .map(
      ([id, label]) =>
        `<button data-page="${id}" class="${page === id ? "active" : ""}">${esc(label)}</button>`,
    )
    .join("");
  document.querySelectorAll<HTMLButtonElement>("nav button").forEach((btn) => {
    btn.addEventListener("click", () => {
      page = btn.dataset.page!;
      editorKey = null;
      renderNav();
      render();
    });
  });
  document.getElementById("page-title")!.textContent = t(`nav.${page}`);
}

function banner(): string {
  if (health?.connected) return "";
  if (!health) return "";
  const busy = installBusy ? ` ${esc(t("svc.installing"))}` : "";
  return `<div class="banner"><div>${esc(t("settings.offline"))} ${esc(t("trust.offline"))}
    <p class="hint">${esc(t("svc.hint"))}</p></div>
    <div class="banner-actions">
      <button class="primary" data-svc="install" ${installBusy ? "disabled" : ""}>${esc(t("svc.install"))}${busy}</button>
      <button class="ghost" data-svc="retry">${esc(t("svc.retry"))}</button>
    </div></div>`;
}

function cards(): string {
  const g = snapshot?.global;
  const apps = snapshot?.apps ?? [];
  const restricted = apps.filter((a) => a.blocked || a.paused).length;
  const active = apps.filter((a) => Date.now() - a.last_seen_ms < 120_000).length;
  const todayDown = g ? g.today_down || g.down : fallback?.down ?? 0;
  const todayUp = g ? g.today_up || g.up : fallback?.up ?? 0;
  const downBps = g?.down_bps ?? 0;
  const upBps = g?.up_bps ?? 0;
  return `<div class="cards">
    <div class="card"><div class="label">${esc(t("card.today"))}</div>
      <div class="value">${bytes(todayDown)}</div>
      <div class="sub">↓ ${bytes(todayUp)} up</div></div>
    <div class="card"><div class="label">${esc(t("card.speed"))}</div>
      <div class="value">${speed(downBps)}</div>
      <div class="sub">↑ ${speed(upBps)}</div></div>
    <div class="card"><div class="label">${esc(t("card.active"))}</div>
      <div class="value">${active}</div>
      <div class="sub">${esc(t("card.restricted"))}: ${restricted}</div></div>
    <div class="card"><div class="label">${esc(t("card.interface"))}</div>
      <div class="value small" style="font-size:15px">${esc(snapshot?.interface ?? fallback?.primary_alias ?? "—")}</div>
      <div class="sub">${snapshot?.metered ? "metered" : "unmetered"}</div></div>
    <div class="card"><div class="label">${esc(t("card.profile"))}</div>
      <div class="value small" style="font-size:15px">${esc(snapshot?.profile ?? "—")}</div>
      <div class="sub">${snapshot?.profile_video_height ? `${snapshot.profile_video_height}p` : "video off"}</div></div>
  </div>`;
}

function iconCell(app: AppUsage): string {
  const key = app.identity.key;
  const src = icons.get(key);
  return `<span class="app-name">
    ${src ? `<img alt="" src="${src}">` : `<span class="logo" style="width:22px;height:22px;box-shadow:inset 0 0 0 5px var(--panel)"></span>`}
    <span class="meta"><div>${esc(app.identity.exe_name)}</div>
    <div class="small muted">${esc(app.identity.publisher ?? app.category)} · PID ${app.pid}</div></span>
  </span>`;
}

function ensureIcon(app: AppUsage): void {
  const key = app.identity.key;
  if (icons.has(key) || iconPending.has(key) || !app.identity.path) return;
  iconPending.add(key);
  void invoke<number[] | null>("app_icon", { path: app.identity.path })
    .then((png) => {
      if (png && png.length > 0) {
        const url = URL.createObjectURL(new Blob([new Uint8Array(png)], { type: "image/png" }));
        icons.set(key, url);
        const img = document.querySelector<HTMLImageElement>(`img[data-icon="${CSS.escape(key)}"]`);
        if (img) img.src = url;
      }
    })
    .catch(() => undefined)
    .finally(() => iconPending.delete(key));
}

function appsTable(apps: AppUsage[]): string {
  if (apps.length === 0) return `<div class="empty">${esc(t("apps.empty"))}</div>`;
  return `<table><thead><tr>
    <th>${esc(t("table.app"))}</th><th>${esc(t("table.used"))}</th><th>${esc(t("table.up"))}</th>
    <th>${esc(t("table.quota"))}</th><th>${esc(t("table.speed"))}</th><th>${esc(t("table.state"))}</th><th></th>
  </tr></thead><tbody>${apps
    .map((app) => {
      const limit = app.limit.total;
      const ratio = limit && limit > 0 ? Math.min(app.period_used.down / limit, 1) : 0;
      const [state, cls] = appState(app);
      return `<tr data-key="${esc(app.identity.key)}">
        <td>${iconCell(app).replace("<img", `<img data-icon="${esc(app.identity.key)}"`)}</td>
        <td>${bytes(app.daily.down)}</td>
        <td>${bytes(app.daily.up)}</td>
        <td style="min-width:130px">${
          limit ? `${bytes(app.period_used.down)} / ${bytes(limit)}<div class="bar ${barClass(ratio)}"><span style="width:${(ratio * 100).toFixed(0)}%"></span></div>` : "unlimited"
        }</td>
        <td class="small">↓${speed(app.speed_down_bps)}<br>↑${speed(app.speed_up_bps)}</td>
        <td><span class="pill ${cls}">${esc(state)}</span></td>
        <td class="row" style="gap:4px">
          <button class="ghost" data-act="pause" data-key="${esc(app.identity.key)}">${esc(app.paused ? t("btn.resume") : t("btn.pause"))}</button>
          <button class="ghost" data-act="block" data-key="${esc(app.identity.key)}">${esc(app.blocked ? t("btn.unblock") : t("btn.block"))}</button>
          <button class="ghost" data-act="edit" data-key="${esc(app.identity.key)}">${esc(t("btn.edit"))}</button>
        </td>
      </tr>`;
    })
    .join("")}</tbody></table>`;
}

function editor(app: AppUsage): string {
  const limitMb = app.limit.total ? Math.round(app.limit.total / 1_000_000) : "";
  return `<div class="editor" data-editor="${esc(app.identity.key)}">
    <div class="field" style="grid-column:1/-1">
      <strong>${esc(app.identity.exe_name)}</strong>
      <span class="small muted">${esc(app.identity.path ?? app.identity.key)}</span>
      ${app.protection_note ? `<span class="small" style="color:var(--warn)">${esc(app.protection_note)}</span>` : ""}
    </div>
    <div class="field"><label>Period</label>
      <select id="ed-period">
        <option value="daily" selected>Daily</option>
        <option value="weekly">Weekly</option>
        <option value="monthly">Monthly</option>
      </select></div>
    <div class="field"><label>Limit (MB, empty = unlimited)</label>
      <input id="ed-limit" type="number" min="0" step="10" value="${limitMb}"></div>
    <div class="field"><label>When exhausted</label>
      <select id="ed-enforce">
        <option value="block">Block network access</option>
        <option value="notify_only">Notify only</option>
        <option value="block_background">Block background only</option>
      </select></div>
    <div class="field"><label>Background</label>
      <label class="check"><input type="checkbox" id="ed-bg"> Block when not focused</label>
      <label class="check"><input type="checkbox" id="ed-fg"> Foreground only</label>
      <label class="check"><input type="checkbox" id="ed-metered"> Block on metered networks</label>
      <label class="check"><input type="checkbox" id="ed-wifi"> Wi-Fi only</label></div>
    <div class="field"><label>${esc(t("table.state"))}</label>
      <span class="small muted">Attribution: ${esc(app.confidence)}${app.exhausted ? " · quota exhausted" : ""}</span>
      <div class="row" style="margin-top:6px">
        <button class="ghost" data-act="credit" data-bytes="50000000" data-key="${esc(app.identity.key)}">+50 MB</button>
        <button class="ghost" data-act="credit" data-bytes="100000000" data-key="${esc(app.identity.key)}">+100 MB</button>
        <button class="ghost" data-act="credit" data-bytes="500000000" data-key="${esc(app.identity.key)}">+500 MB</button>
      </div>
    </div>
    <div class="field" style="grid-column:1/-1" >
      <div class="row">
        <button id="ed-save">${esc(t("btn.save"))}</button>
        <button class="ghost" id="ed-cancel">${esc(t("btn.cancel"))}</button>
        <span class="small muted">Temporary access adds a credit without changing the limit.</span>
      </div>
    </div>
  </div>`;
}

async function saveEditor(key: string): Promise<void> {
  const period = (document.getElementById("ed-period") as HTMLSelectElement).value as Period;
  const limitRaw = (document.getElementById("ed-limit") as HTMLInputElement).value.trim();
  const mb = Number(limitRaw);
  const limitBytes = limitRaw === "" || Number.isNaN(mb) || mb <= 0 ? null : Math.round(mb * 1_000_000);
  await invoke("save_policy", {
    appKey: key,
    limitBytes,
    period,
    enforcement: (document.getElementById("ed-enforce") as HTMLSelectElement).value as Enforcement,
    blockBackground: (document.getElementById("ed-bg") as HTMLInputElement).checked,
    foregroundOnly: (document.getElementById("ed-fg") as HTMLInputElement).checked,
    blockOnMetered: (document.getElementById("ed-metered") as HTMLInputElement).checked,
    wifiOnly: (document.getElementById("ed-wifi") as HTMLInputElement).checked,
    paused: false,
  });
  editorKey = null;
  await refresh(true);
}

function wireEditor(apps: AppUsage[]): void {
  document.querySelectorAll<HTMLButtonElement>("[data-act]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const key = btn.dataset.key!;
      const act = btn.dataset.act!;
      const app = apps.find((a) => a.identity.key === key);
      if (!app) return;
      if (act === "edit") {
        editorKey = editorKey === key ? null : key;
        render();
        return;
      }
      void (async () => {
        try {
          if (act === "pause") await invoke("set_paused", { appKey: key, paused: !app.paused });
          if (act === "block") await invoke("set_blocked", { appKey: key, blocked: !app.blocked });
          if (act === "credit") await invoke("grant_credit", { appKey: key, extraBytes: Number(btn.dataset.bytes), durationSecs: null });
          await refresh(true);
        } catch (err) {
          lastError = String(err);
          render();
        }
      })();
    });
  });
  document.getElementById("ed-save")?.addEventListener("click", () => {
    void saveEditor(editorKey!).catch((err) => {
      lastError = String(err);
      render();
    });
  });
  document.getElementById("ed-cancel")?.addEventListener("click", () => {
    editorKey = null;
    render();
  });
}

function viewDashboard(): string {
  const apps = [...(snapshot?.apps ?? [])].sort((a, b) => b.daily.down - a.daily.down).slice(0, 6);
  const content =
    apps.length === 0
      ? `<div class="empty">${esc(t("dash.empty"))}</div>`
      : `<table><thead><tr><th>${esc(t("table.app"))}</th><th>${esc(t("table.used"))}</th><th>${esc(t("table.quota"))}</th><th>${esc(t("table.speed"))}</th></tr></thead><tbody>
        ${apps
          .map((app) => {
            const limit = app.limit.total;
            const ratio = limit ? Math.min(app.period_used.down / limit, 1) : 0;
            return `<tr><td>${iconCell(app)}</td><td>${bytes(app.daily.down)}</td>
              <td style="min-width:120px">${limit ? `${bytes(app.period_used.down)} / ${bytes(limit)}<div class="bar ${barClass(ratio)}"><span style="width:${(ratio * 100).toFixed(0)}%"></span></div>` : "unlimited"}</td>
              <td class="small">↓${speed(app.speed_down_bps)}</td></tr>`;
          })
          .join("")}</tbody></table>`;
  return `${banner()}${cards()}<div class="section"><h2>${esc(t("dash.top"))}</h2>${content}</div>`;
}

function viewApps(): string {
  const query = (document.getElementById("app-search") as HTMLInputElement | null)?.value ?? "";
  const all = snapshot?.apps ?? [];
  const apps = all.filter((a) =>
    `${a.identity.exe_name} ${a.identity.path ?? ""} ${a.category}`.toLowerCase().includes(query.toLowerCase()),
  );
  const selected = apps.find((a) => a.identity.key === editorKey);
  return `${banner()}
    <div class="row" style="margin-bottom:10px">
      <input id="app-search" placeholder="${esc(t("apps.search"))}" value="${esc(query)}" style="flex:1;max-width:340px">
      <span class="small muted">${apps.length} / ${all.length}</span>
    </div>
    ${appsTable(apps)}
    ${selected ? editor(selected) : ""}`;
}

function viewSites(): string {
  const sites = snapshot?.sites ?? [];
  if (sites.length === 0) return `${banner()}<div class="empty">${esc(t("sites.empty"))}</div>`;
  return `${banner()}<table><thead><tr>
    <th>${esc(t("table.site"))}</th><th>${esc(t("table.used"))}</th><th>${esc(t("table.quota"))}</th><th>${esc(t("table.state"))}</th><th>${esc(t("table.actions"))}</th>
  </tr></thead><tbody>${sites
    .map((site) => {
      const ratio = site.limit ? Math.min(site.down / site.limit, 1) : 0;
      const state = site.blocked ? t("state.blocked") : site.exhausted ? t("state.over") : t("state.ok");
      const cls = site.blocked || site.exhausted ? "bad" : "ok";
      return `<tr>
        <td>${esc(site.host)} <span class="small muted">${site.estimated ? `(${esc(t("site.estimated"))})` : ""}</span></td>
        <td>${bytes(site.down)}<div class="small muted">↑ ${bytes(site.up)}</div></td>
        <td style="min-width:170px">${site.limit ? `${bytes(site.down)} / ${bytes(site.limit)}<div class="bar ${barClass(ratio)}"><span style="width:${(ratio * 100).toFixed(0)}%"></span></div>` : "unlimited"}</td>
        <td><span class="pill ${cls}">${esc(state)}</span></td>
        <td class="row" style="gap:6px">
          <input type="number" min="0" step="10" data-site-limit="${esc(site.host)}" value="${site.limit ? Math.round(site.limit / 1_000_000) : ""}" style="width:90px">
          <button class="ghost" data-act="site-limit" data-host="${esc(site.host)}">${esc(t("btn.save"))}</button>
          <button class="ghost" data-act="site-block" data-host="${esc(site.host)}" data-blocked="${site.blocked ? "0" : "1"}">${esc(site.blocked ? t("btn.unblock") : t("btn.block"))}</button>
        </td></tr>`;
    })
    .join("")}</tbody></table>`;
}

function viewSaver(): string {
  const profiles: Array<[string, string, string]> = [
    ["normal", "Normal", "No restrictions. Video quality untouched."],
    ["data_saver", "Data Saver", "360p video, blocks background traffic on metered networks."],
    ["strict_data_saver", "Strict Data Saver", "240p video, background blocked."],
    ["gaming", "Gaming", "Prioritises latency: no video forcing, background updates paused."],
  ];
  return `${banner()}<div class="profiles">${profiles
    .map(
      ([id, name, desc]) => `<div class="profile ${snapshot?.profile === id ? "active" : ""}" data-profile="${id}">
        <strong>${esc(name)}</strong><span class="small muted">${esc(desc)}</span></div>`,
    )
    .join("")}</div>
    <div class="section"><h2>${esc(t("saver.quality"))}</h2>
      <div class="card"><div class="value small" style="font-size:16px">${
        snapshot?.profile_video_height ? `${snapshot.profile_video_height}p` : "Off"
      }</div>
      <div class="small muted">${esc(t("saver.note"))}</div></div></div>`;
}

function viewEvents(): string {
  const events = snapshot?.events ?? [];
  if (events.length === 0) return `${banner()}<div class="empty">${esc(t("events.empty"))}</div>`;
  return `${banner()}<div class="events">${events
    .map(
      (event) => `<div class="event"><time>${esc(dateTime(event.ts_ms))}</time>
        <span class="pill">${esc(event.kind)}</span>
        <span>${esc(event.message)}</span>
        <span class="small muted" style="margin-inline-start:auto">${esc(event.subject)}</span></div>`,
    )
    .join("")}</div>`;
}

function viewSettings(): string {
  const h = health;
  return `<div class="section" style="margin-top:0"><h2>${esc(t("settings.service"))}</h2>
    <div class="card">
      <div class="row"><span class="dot ${h?.connected ? "on" : ""}"></span>
        <strong>${h?.connected ? "Connected" : "Not connected"}</strong>
        <span class="small muted">${esc(h?.service_version ?? "")}</span></div>
      <div class="small muted" style="margin-top:6px">Monitoring: ${esc(h?.monitoring ?? "unavailable")} · Enforcement: ${
        h?.enforcement_ready ? "ready" : "unavailable"
      } · Protocol v${h?.protocol_version ?? "?"}</div>
      <div class="small muted">${esc(t("settings.pipe"))}: <code>${esc(h?.pipe ?? "")}</code></div>
    </div></div>
    <div class="section"><h2>${esc(t("settings.recover"))}</h2>
      <div class="card">
        <div class="small muted" style="margin-bottom:8px">${esc(t("settings.recoverHelp"))}</div>
        <button class="danger" id="recover">${esc(t("settings.recover"))}</button>
        <span id="recover-result" class="small muted"></span>
      </div></div>
    <div class="section"><h2>${esc(t("settings.install"))}</h2>
      <div class="card"><div class="small muted">${esc(t("settings.installHelp"))}</div>
      <div class="banner-actions" style="margin-top:10px">
        <button data-svc="install" ${installBusy ? "disabled" : ""}>${esc(t("svc.install"))}</button>
        <button class="ghost" data-svc="retry">${esc(t("svc.retry"))}</button>
      </div>
      <div class="small muted" style="margin-top:8px">Extension: load <code>apps/chrome-extension</code> unpacked, then run
      <code>scripts/register-native-host.ps1 -ExtensionId &lt;id&gt;</code> so the bridge can authenticate it.</div></div></div>
    ${lastError ? `<div class="banner" style="margin-top:14px">${esc(lastError)}</div>` : ""}`;
}

function render(): void {
  document.documentElement.dir = lang === "ar" ? "rtl" : "ltr";
  document.documentElement.dataset.theme = theme;
  document.getElementById("conn-text")!.textContent = health?.connected
    ? `service ${health.service_version ?? ""}`
    : "service offline";
  document.querySelector("#conn .dot")!.className = `dot ${health?.connected ? "on" : ""}`;
  const view = document.getElementById("view")!;
  switch (page) {
    case "apps":
      view.innerHTML = viewApps();
      wireEditor(snapshot?.apps ?? []);
      document.getElementById("app-search")?.addEventListener("input", () => {
        view.innerHTML = viewApps();
        wireEditor(snapshot?.apps ?? []);
      });
      (snapshot?.apps ?? []).forEach(ensureIcon);
      break;
    case "sites":
      view.innerHTML = viewSites();
      document.querySelectorAll<HTMLButtonElement>("[data-act^='site-']").forEach((btn) => {
        btn.addEventListener("click", () => {
          const host = btn.dataset.host!;
          void (async () => {
            try {
              if (btn.dataset.act === "site-block") {
                await invoke("set_site_policy", { site: host, limitBytes: null, blocked: btn.dataset.blocked === "1" });
                // A block without a stored limit must still reach the extension.
                const state = await invoke<Snapshot>("snapshot");
                const site = state.sites.find((s) => s.host === host);
                if (site) await invoke("set_site_policy", { site: host, limitBytes: site.limit, blocked: btn.dataset.blocked === "1" });
              } else {
                const input = document.querySelector<HTMLInputElement>(`[data-site-limit="${CSS.escape(host)}"]`)!;
                const mb = Number(input.value);
                const limit = input.value.trim() === "" || mb <= 0 ? null : Math.round(mb * 1_000_000);
                await invoke("set_site_policy", { site: host, limitBytes: limit, blocked: false });
              }
              await refresh(true);
            } catch (err) {
              lastError = String(err);
              render();
            }
          })();
        });
      });
      break;
    case "saver":
      view.innerHTML = viewSaver();
      document.querySelectorAll<HTMLElement>("[data-profile]").forEach((el) => {
        el.addEventListener("click", () => {
          void invoke("apply_profile", { profile: el.dataset.profile }).then(() => refresh(true));
        });
      });
      break;
    case "events":
      view.innerHTML = viewEvents();
      break;
    case "settings":
      view.innerHTML = viewSettings();
      document.getElementById("recover")?.addEventListener("click", () => {
        void invoke<number>("restore_all")
          .then((removed) => {
            const el = document.getElementById("recover-result");
            if (el) el.textContent = t("trust.recover").replace("{n}", String(removed));
          })
          .catch((err) => {
            lastError = String(err);
            render();
          });
      });
      break;
    default:
      view.innerHTML = viewDashboard();
      (snapshot?.apps ?? []).forEach(ensureIcon);
  }
  document.getElementById("page-title")!.textContent = t(`nav.${page}`);
}

async function refresh(force: boolean): Promise<void> {
  try {
    health = await invoke<Health>("health");
    if (health.connected) {
      snapshot = await invoke<Snapshot>("snapshot");
      fallback = null;
    } else {
      fallback = await invoke<FallbackUsage>("fallback_usage").catch(() => null);
      snapshot = null;
    }
  } catch (err) {
    lastError = String(err);
    health = { connected: false, service_version: null, monitoring: null, enforcement_ready: false, protocol_version: 1, pipe: "" };
  }
  if (force || page === "dashboard") render();
}

function boot(): void {
  lang = (localStorage.getItem("nq-lang") as "en" | "ar") ?? "en";
  theme = (localStorage.getItem("nq-theme") as "dark" | "light") ?? "dark";
  const langSelect = document.getElementById("lang") as HTMLSelectElement;
  const themeSelect = document.getElementById("theme") as HTMLSelectElement;
  langSelect.value = lang;
  themeSelect.value = theme;
  langSelect.addEventListener("change", () => {
    lang = langSelect.value as "en" | "ar";
    localStorage.setItem("nq-lang", lang);
    renderNav();
    render();
  });
  themeSelect.addEventListener("change", () => {
    theme = themeSelect.value as "dark" | "light";
    localStorage.setItem("nq-theme", theme);
    render();
  });
  document.getElementById("refresh")!.addEventListener("click", () => void refresh(true));
  document.getElementById("view")!.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLElement>("[data-svc]");
    if (!button) return;
    if (button.dataset.svc === "retry") {
      void refresh(true);
      return;
    }
    installBusy = true;
    render();
    void invoke<string>("install_service")
      .then(async () => {
        installBusy = false;
        // The service needs a moment before the pipe accepts clients.
        await new Promise((resolve) => setTimeout(resolve, 1500));
        await refresh(true);
      })
      .catch((err) => {
        installBusy = false;
        lastError = String(err);
        render();
      });
  });
  renderNav();
  void refresh(true);
  // Live dashboard only: editors keep focus, other pages refresh on navigation.
  window.setInterval(() => {
    if (page === "dashboard" && !editorKey) void refresh(false);
  }, 2_000);
}

boot();
