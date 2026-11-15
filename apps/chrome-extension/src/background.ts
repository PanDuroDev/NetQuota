//! Extension service worker: native bridge, site usage ledger, DNR enforcement.
//!
//! The extension never performs privileged work itself. Every blocking action
//! is either a declarativeNetRequest rule scoped to one hostname or a request
//! forwarded to the NetQuota Windows service through the native host.

import type {
  PopupState,
  Request,
  Response,
  SiteUsage,
  Snapshot,
  VideoSettings,
  VideoStatus,
} from "./shared/types.js";

const NATIVE_HOST = "com.netquota.host";
const K_SETTINGS = "settings";
const K_BLOCKED = "blocked";
const K_SITES = "sites";
const K_USAGE = "usage";
const K_VIDEO = "video";

const DEFAULT_SETTINGS: VideoSettings = {
  quality_height: null,
  pause_background_video: true,
  prevent_autoplay: false,
};

type BlockedEntry = { until_ms: number | null };
type BlockedMap = Record<string, BlockedEntry>;
type UsageLedger = { day: string; hosts: Record<string, number> };

let port: chrome.runtime.Port | null = null;
let connected = false;
let lastError: string | null = null;
let snapshot: Snapshot | null = null;
let reconnectDelay = 2_000;
let reconnectTimer: number | null = null;

// A single pending response: the native host answers each request in order.
let pending: { resolve: (r: Response) => void; reject: (e: Error) => void } | null = null;
let callChain: Promise<unknown> = Promise.resolve();

function dayKey(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate(),
  ).padStart(2, "0")}`;
}

async function readSettings(): Promise<VideoSettings> {
  const stored = await chrome.storage.local.get(K_SETTINGS);
  return { ...DEFAULT_SETTINGS, ...(stored[K_SETTINGS] as Partial<VideoSettings> | undefined) };
}

async function readBlocked(): Promise<BlockedMap> {
  const stored = await chrome.storage.local.get(K_BLOCKED);
  return (stored[K_BLOCKED] as BlockedMap | undefined) ?? {};
}

async function readSites(): Promise<SiteUsage[]> {
  const stored = await chrome.storage.local.get(K_SITES);
  return (stored[K_SITES] as SiteUsage[] | undefined) ?? [];
}

async function readUsage(): Promise<UsageLedger> {
  const stored = await chrome.storage.local.get(K_USAGE);
  const ledger = stored[K_USAGE] as UsageLedger | undefined;
  const today = dayKey();
  if (!ledger || ledger.day !== today) {
    return { day: today, hosts: {} };
  }
  return ledger;
}

// ---------------------------------------------------------------------------
// Native messaging
// ---------------------------------------------------------------------------

function setConnected(value: boolean): void {
  connected = value;
  void chrome.action.setBadgeText({ text: value ? "" : "!" });
  void chrome.action.setBadgeBackgroundColor({ color: "#b45309" });
}

function scheduleReconnect(): void {
  if (reconnectTimer !== null) return;
  const delay = reconnectDelay;
  reconnectTimer = self.setTimeout(() => {
    reconnectTimer = null;
    reconnectDelay = Math.min(reconnectDelay * 2, 60_000);
    void connectNative();
  }, delay) as unknown as number;
}

function connectNative(): Promise<void> {
  return new Promise((resolve) => {
    if (connected && port) {
      resolve();
      return;
    }
    try {
      port = chrome.runtime.connectNative(NATIVE_HOST);
    } catch (err) {
      lastError = `native host unavailable: ${String(err)}`;
      setConnected(false);
      scheduleReconnect();
      resolve();
      return;
    }
    const p = port;
    p.onMessage.addListener((message: unknown) => {
      const resolver = pending;
      pending = null;
      if (!resolver) return;
      const response = message as Response;
      if (response.type === "error") {
        lastError = response.message;
        resolver.reject(new Error(`${response.code}: ${response.message}`));
      } else {
        resolver.resolve(response);
      }
      if (response.type === "snapshot") snapshot = response.snapshot;
    });
    p.onDisconnect.addListener(() => {
      const reason = chrome.runtime.lastError?.message ?? "disconnected";
      lastError = reason;
      port = null;
      setConnected(false);
      if (pending) {
        pending.reject(new Error(reason));
        pending = null;
      }
      scheduleReconnect();
    });
    p.postMessage({
      type: "hello",
      version: 1,
      client: "chrome_extension",
      extension_id: chrome.runtime.id,
    } satisfies Request);
    pending = {
      resolve: (r) => {
        if (r.type === "hello") {
          setConnected(true);
          lastError = null;
          reconnectDelay = 2_000;
          void refreshFromService();
        }
        resolve();
      },
      reject: (e) => {
        lastError = e.message;
        setConnected(false);
        scheduleReconnect();
        resolve();
      },
    };
  });
}

function nativeCall(request: Request): Promise<Response> {
  const run = callChain.then(
    () =>
      new Promise<Response>((resolve, reject) => {
        if (!port || !connected) {
          reject(new Error("service disconnected"));
          return;
        }
        const timer = self.setTimeout(() => {
          pending = null;
          reject(new Error("service response timeout"));
        }, 10_000);
        pending = {
          resolve: (r) => {
            self.clearTimeout(timer);
            resolve(r);
          },
          reject: (e) => {
            self.clearTimeout(timer);
            reject(e);
          },
        };
        try {
          port.postMessage(request);
        } catch (err) {
          self.clearTimeout(timer);
          pending = null;
          reject(err instanceof Error ? err : new Error(String(err)));
        }
      }),
  );
  callChain = run.catch(() => undefined);
  return run;
}

async function refreshFromService(): Promise<void> {
  try {
    const sites = await nativeCall({ type: "list_sites" });
    if (sites.type === "sites") {
      await chrome.storage.local.set({ [K_SITES]: sites.sites });
      await syncRules(sites.sites);
    }
    const snap = await nativeCall({ type: "snapshot" });
    if (snap.type === "snapshot") {
      snapshot = snap.snapshot;
      const settings = await readSettings();
      const height = snap.snapshot.profile_video_height;
      if (height !== settings.quality_height) {
        await chrome.storage.local.set({
          [K_SETTINGS]: { ...settings, quality_height: height },
        });
      }
    }
  } catch (err) {
    lastError = String(err);
  }
}

// ---------------------------------------------------------------------------
// Site usage and quota enforcement
// ---------------------------------------------------------------------------

async function recordUsage(host: string, bytes: number): Promise<void> {
  if (!host || bytes <= 0) return;
  const usage = await readUsage();
  usage.hosts[host] = (usage.hosts[host] ?? 0) + bytes;
  await chrome.storage.local.set({ [K_USAGE]: usage });
  void flushSite(host, usage.hosts[host]);
}

async function flushSite(host: string, total: number): Promise<void> {
  const sites = await readSites();
  const site = sites.find((s) => s.host === host);
  const limit = site?.limit ?? null;

  if (connected) {
    try {
      // Counters are cumulative per day; the service treats repeated totals as
      // the authoritative value for the current period.
      await nativeCall({
        type: "site_usage",
        site: host,
        down: total,
        up: 0,
        active_secs: 0,
      });
    } catch {
      // Offline: the local ledger still drives enforcement.
    }
  }

  if (limit !== null && total >= limit) {
    const blocked = await readBlocked();
    if (!blocked[host]) {
      blocked[host] = { until_ms: null };
      await chrome.storage.local.set({ [K_BLOCKED]: blocked });
      await applyRules(blocked);
      void notify(
        "NetQuota: site quota reached",
        `${host} reached its limit (${formatBytes(total)} of ${formatBytes(limit)}).`,
      );
      void chrome.storage.local.set({
        [`event:${host}`]: { used: total, limit, at: new Date().toISOString() },
      });
    }
  }
}

function notify(title: string, message: string): void {
  chrome.notifications.create("", {
    type: "basic",
    iconUrl: chrome.runtime.getURL("icon-128.png"),
    title,
    message,
  });
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

// ---------------------------------------------------------------------------
// declarativeNetRequest rule management
// ---------------------------------------------------------------------------

function ruleBase(host: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < host.length; i += 1) {
    hash ^= host.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return (hash % 9_000) * 4 + 1;
}

async function applyRules(blocked: BlockedMap): Promise<void> {
  const existing = await chrome.declarativeNetRequest.getDynamicRules();
  const removeRuleIds = existing.map((r) => r.id);
  const addRules: chrome.declarativeNetRequest.Rule[] = [];
  const now = Date.now();
  for (const [host, entry] of Object.entries(blocked)) {
    if (entry.until_ms !== null && entry.until_ms <= now) continue;
    const base = ruleBase(host);
    const redirect = chrome.runtime.getURL(`dist/blocked.html?host=${encodeURIComponent(host)}`);
    addRules.push(
      {
        id: base,
        priority: 2,
        action: {
          type: "redirect" as chrome.declarativeNetRequest.RuleActionType,
          redirect: { url: redirect },
        },
        condition: {
          requestDomains: [host],
          resourceTypes: ["main_frame" as chrome.declarativeNetRequest.ResourceType],
        },
      },
      {
        id: base + 1,
        priority: 1,
        action: { type: "block" as chrome.declarativeNetRequest.RuleActionType },
        condition: {
          requestDomains: [host],
          resourceTypes: [
            "sub_frame",
            "stylesheet",
            "script",
            "image",
            "font",
            "object",
            "xmlhttprequest",
            "ping",
            "media",
            "websocket",
            "other",
          ] as chrome.declarativeNetRequest.ResourceType[],
        },
      },
    );
  }
  await chrome.declarativeNetRequest.updateDynamicRules({ removeRuleIds, addRules });
}

async function syncRules(sites: SiteUsage[]): Promise<void> {
  const blocked = await readBlocked();
  for (const site of sites) {
    if (site.blocked && !blocked[site.host]) {
      blocked[site.host] = { until_ms: null };
    }
  }
  await chrome.storage.local.set({ [K_BLOCKED]: blocked });
  await applyRules(blocked);
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

async function popupState(): Promise<PopupState> {
  const usage = await readUsage();
  const sites = await readSites();
  const settings = await readSettings();
  const blocked = await readBlocked();
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  let host = "";
  try {
    host = tab?.url ? new URL(tab.url).hostname : "";
  } catch {
    host = "";
  }
  const site = sites.find((s) => s.host === host);
  const videoMap = (await chrome.storage.local.get(K_VIDEO))[K_VIDEO] as
    | Record<string, VideoStatus>
    | undefined;
  const total = usage.hosts[host] ?? 0;
  return {
    connected,
    service_version: snapshot?.service_version ?? null,
    monitoring: snapshot?.monitoring ?? null,
    enforcement_ready: snapshot?.enforcement_ready ?? false,
    host,
    today_bytes: total,
    estimated: true,
    limit: site?.limit ?? null,
    blocked: Boolean(blocked[host]),
    exhausted: site?.exhausted ?? false,
    profile: snapshot?.profile ?? "normal",
    quality_height: settings.quality_height,
    video: videoMap?.[host] ?? null,
    sites: sites.filter((s) => s.down > 0 || s.limit !== null),
    global: snapshot?.global ?? null,
    last_error: lastError,
  };
}

chrome.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
  const msg = message as Record<string, unknown>;
  void (async () => {
    switch (msg.type) {
      case "content_usage":
        await recordUsage(String(msg.host), Number(msg.bytes) || 0);
        sendResponse({ ok: true });
        break;
      case "content_settings":
        sendResponse({ settings: await readSettings() });
        break;
      case "video_status": {
        const status = msg.status as VideoStatus;
        const stored = (await chrome.storage.local.get(K_VIDEO))[K_VIDEO] as
          | Record<string, VideoStatus>
          | undefined;
        const map = stored ?? {};
        map[status.host] = status;
        await chrome.storage.local.set({ [K_VIDEO]: map });
        sendResponse({ ok: true });
        break;
      }
      case "popup_state":
        sendResponse(await popupState());
        break;
      case "set_blocked": {
        const host = String(msg.host);
        const blocked = await readBlocked();
        if (msg.blocked) {
          blocked[host] = { until_ms: null };
        } else {
          delete blocked[host];
        }
        await chrome.storage.local.set({ [K_BLOCKED]: blocked });
        await applyRules(blocked);
        const site = (await readSites()).find((s) => s.host === host);
        if (connected) {
          try {
            await nativeCall({
              type: "set_site_policy",
              site: host,
              limit_bytes: site?.limit ?? null,
              blocked: Boolean(msg.blocked),
            });
          } catch {
            lastError = "change saved locally; service unreachable";
          }
        }
        sendResponse({ ok: true });
        break;
      }
      case "temp_access": {
        const host = String(msg.host);
        const blocked = await readBlocked();
        blocked[host] = { until_ms: Date.now() + 5 * 60_000 };
        await chrome.storage.local.set({ [K_BLOCKED]: blocked });
        await applyRules(blocked);
        await chrome.alarms.create(`unblock:${host}`, { when: Date.now() + 5 * 60_000 });
        sendResponse({ ok: true });
        break;
      }
      case "set_limit": {
        const host = String(msg.host);
        const limit = msg.limit === null ? null : Number(msg.limit);
        const sites = await readSites();
        const existing = sites.find((s) => s.host === host);
        if (existing) existing.limit = limit;
        else {
          sites.push({
            host,
            down: 0,
            up: 0,
            estimated: true,
            active_secs: 0,
            tabs: 1,
            limit,
            remaining_total: limit,
            exhausted: false,
            blocked: false,
          });
        }
        await chrome.storage.local.set({ [K_SITES]: sites });
        if (connected) {
          try {
            await nativeCall({
              type: "set_site_policy",
              site: host,
              limit_bytes: limit,
              blocked: false,
            });
          } catch {
            lastError = "limit saved locally; service unreachable";
          }
        }
        sendResponse({ ok: true });
        break;
      }
      case "set_quality": {
        const settings = await readSettings();
        settings.quality_height = msg.height === null ? null : Number(msg.height);
        await chrome.storage.local.set({ [K_SETTINGS]: settings });
        sendResponse({ ok: true });
        break;
      }
      case "set_profile": {
        if (connected) {
          try {
            await nativeCall({ type: "apply_profile", profile: String(msg.profile) });
            await refreshFromService();
          } catch (err) {
            lastError = String(err);
          }
        }
        sendResponse({ ok: true, connected });
        break;
      }
      case "reconnect":
        await connectNative();
        sendResponse({ ok: true });
        break;
      default:
        sendResponse({ ok: false, error: "unknown message" });
    }
  })();
  return true;
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name.startsWith("unblock:")) {
    const host = alarm.name.slice("unblock:".length);
    void (async () => {
      const blocked = await readBlocked();
      delete blocked[host];
      await chrome.storage.local.set({ [K_BLOCKED]: blocked });
      await applyRules(blocked);
    })();
  }
});

chrome.alarms.create("maintenance", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name !== "maintenance") return;
  void (async () => {
    if (!connected) await connectNative();
    const usage = await readUsage();
    for (const [host, bytes] of Object.entries(usage.hosts)) {
      await flushSite(host, bytes);
    }
  })();
});

chrome.runtime.onInstalled.addListener(() => {
  void (async () => {
    await applyRules(await readBlocked());
    await connectNative();
  })();
});

chrome.runtime.onStartup.addListener(() => {
  void connectNative();
});

void connectNative();
