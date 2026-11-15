//! Shared message shapes. The extension mirrors the Rust protocol instead of
//! importing it; these are type-only, so nothing is duplicated at runtime.

export type ClientKind = "desktop" | "chrome_extension" | "cli";

export type Request =
  | { type: "hello"; version: number; client: ClientKind; extension_id?: string | null }
  | { type: "ping" }
  | { type: "snapshot" }
  | { type: "list_apps" }
  | { type: "site_usage"; site: string; down: number; up: number; active_secs: number }
  | { type: "list_sites" }
  | { type: "set_site_policy"; site: string; limit_bytes: number | null; blocked: boolean }
  | { type: "list_events"; limit: number }
  | { type: "apply_profile"; profile: string }
  | { type: "emergency_restore" };

export interface Usage {
  down: number;
  up: number;
}

export interface GlobalUsage {
  down: number;
  up: number;
  today_down: number;
  today_up: number;
  down_bps: number;
  up_bps: number;
}

export interface SiteUsage {
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

export interface EventRecord {
  id: number;
  ts_ms: number;
  kind: string;
  subject: string;
  message: string;
  bytes: number | null;
}

export interface AppUsage {
  identity: { key: string; exe_name: string; path: string | null };
  pid: number;
  category: string;
  down: number;
  up: number;
  speed_down_bps: number;
  speed_up_bps: number;
  period_used: Usage;
  limit: { total: number | null; download: number | null; upload: number | null };
  exhausted: boolean;
  blocked: boolean;
  background: boolean;
  confidence: string;
}

export interface Snapshot {
  generated_at_ms: number;
  service_version: string;
  monitoring: "running" | "degraded" | "unavailable";
  enforcement_ready: boolean;
  global: GlobalUsage;
  apps: AppUsage[];
  sites: SiteUsage[];
  profile: string;
  profile_video_height: number | null;
  metered: boolean;
  interface: string | null;
  events: EventRecord[];
  unattributed_bytes: number;
}

export type Response =
  | { type: "hello"; version: number; service_version: string; enforcement_ready: boolean; monitoring: string }
  | { type: "pong" }
  | { type: "snapshot"; snapshot: Snapshot }
  | { type: "apps"; apps: AppUsage[] }
  | { type: "policy_ok" }
  | { type: "sites"; sites: SiteUsage[] }
  | { type: "events"; events: EventRecord[] }
  | { type: "restored"; removed_rules: number }
  | { type: "error"; code: string; message: string };

export interface VideoSettings {
  quality_height: number | null;
  pause_background_video: boolean;
  prevent_autoplay: boolean;
}

export interface VideoStatus {
  host: string;
  quality: "enforced" | "best_effort" | "unsupported";
  data_saving: "enforced" | "best_effort" | "unsupported";
  detail: string;
  ts_ms: number;
}

export interface PopupState {
  connected: boolean;
  service_version: string | null;
  monitoring: string | null;
  enforcement_ready: boolean;
  host: string;
  today_bytes: number;
  estimated: boolean;
  limit: number | null;
  blocked: boolean;
  exhausted: boolean;
  profile: string;
  quality_height: number | null;
  video: VideoStatus | null;
  sites: SiteUsage[];
  global: GlobalUsage | null;
  last_error: string | null;
}
