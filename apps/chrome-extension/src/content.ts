//! Content script: site measurement plus the video adapter registry.
//!
//! Measurement is honest about its limits. `PerformanceResourceTiming` only
//! exposes transferred bytes for same-origin or Timing-Allow-Origin responses,
//! so media bytes are a documented bitrate model and every number produced here
//! is reported as estimated, never as a measurement.

export {};

interface VideoSettings {
  quality_height: number | null;
  pause_background_video: boolean;
  prevent_autoplay: boolean;
}

type Support = "enforced" | "best_effort" | "unsupported";

interface AdapterResult {
  quality: Support;
  data_saving: Support;
  detail: string;
}

interface VideoAdapter {
  name: string;
  matches(): boolean;
  /** Applies the preferred height; returns what was actually verified. */
  applyQuality(height: number | null): Support;
  /** Pauses off-screen/hidden video and trims preload. */
  reduceData(settings: VideoSettings): Support;
}

const host = location.hostname;
const settings: VideoSettings = {
  quality_height: null,
  pause_background_video: true,
  prevent_autoplay: false,
};

// ---------------------------------------------------------------------------
// YouTube
// ---------------------------------------------------------------------------

const YT_LEVELS: Record<string, number> = {
  tiny: 144,
  small: 240,
  medium: 360,
  large: 480,
  hd720: 720,
  hd1080: 1080,
  hd1440: 1440,
  hd2160: 2160,
};

function levelForHeight(height: number): string {
  const entries = Object.entries(YT_LEVELS).sort((a, b) => a[1] - b[1]);
  for (const [name, h] of entries) {
    if (h >= height) return name;
  }
  return entries[entries.length - 1][0];
}

interface YouTubePlayer {
  getPlaybackQuality?: () => string;
  getAvailableQualityLevels?: () => string[];
  setPlaybackQualityRange?: (start: string, end?: string) => void;
  pauseVideo?: () => void;
}

function ytPlayer(): YouTubePlayer | null {
  const el = document.getElementById("movie_player");
  return el as unknown as YouTubePlayer | null;
}

const youtube: VideoAdapter = {
  name: "youtube",
  matches: () => /(^|\.)youtube\.com$/.test(host) || /(^|\.)youtube-nocookie\.com$/.test(host),
  applyQuality(height) {
    const player = ytPlayer();
    // The IFrame API's setPlaybackQuality is deprecated; the on-page player
    // still exposes these methods, but it may re-select a level on its own.
    if (!player?.setPlaybackQualityRange) return "unsupported";
    if (height === null) return "unsupported";
    const target = levelForHeight(height);
    try {
      player.setPlaybackQualityRange(target, target);
    } catch {
      return "unsupported";
    }
    const actual = player.getPlaybackQuality?.();
    if (!actual) return "best_effort";
    const actualHeight = YT_LEVELS[actual] ?? 0;
    if (actualHeight > 0 && actualHeight <= height) return "enforced";
    return "best_effort";
  },
  reduceData(s) {
    const video = document.querySelector("video");
    if (!video) return "unsupported";
    video.preload = "none";
    if (!s.pause_background_video) return "best_effort";
    if (document.hidden && !video.paused) {
      video.pause();
    }
    return "enforced";
  },
};

// ---------------------------------------------------------------------------
// TikTok
// ---------------------------------------------------------------------------

const tiktok: VideoAdapter = {
  name: "tiktok",
  matches: () => /(^|\.)tiktok\.com$/.test(host),
  applyQuality() {
    // TikTok exposes no quality control that survives its player re-creation.
    return "unsupported";
  },
  reduceData(s) {
    const videos = [...document.querySelectorAll<HTMLVideoElement>("video")];
    if (videos.length === 0) return "unsupported";
    for (const video of videos) {
      video.preload = "none";
      // Off-screen reels keep buffering unless explicitly paused.
      if (video.getBoundingClientRect().bottom < 0) video.pause();
    }
    if (document.hidden && s.pause_background_video) {
      for (const video of videos) video.pause();
    }
    return "enforced";
  },
};

// ---------------------------------------------------------------------------
// Generic HTML5
// ---------------------------------------------------------------------------

const generic: VideoAdapter = {
  name: "generic",
  matches: () => true,
  applyQuality() {
    // An HTMLVideoElement cannot select a rendition of HLS/DASH/MSE content.
    return "unsupported";
  },
  reduceData(s) {
    const videos = [...document.querySelectorAll<HTMLVideoElement>("video")];
    if (videos.length === 0) return "unsupported";
    for (const video of videos) {
      if (!video.dataset.netquotaPreload) {
        video.dataset.netquotaPreload = "1";
        if (video.preload === "auto") video.preload = "metadata";
      }
      if (s.pause_background_video && document.hidden && !video.paused) video.pause();
    }
    if (s.prevent_autoplay) {
      for (const video of videos) {
        if (!video.paused) video.pause();
      }
    }
    return "enforced";
  },
};

const adapters: VideoAdapter[] = [youtube, tiktok, generic];

function activeAdapter(): VideoAdapter | null {
  return adapters.find((a) => a.matches()) ?? null;
}

let lastResult: AdapterResult | null = null;

function runAdapters(): void {
  const adapter = activeAdapter();
  if (!adapter) return;
  const quality = adapter.applyQuality(settings.quality_height);
  const dataSaving = adapter.reduceData(settings);
  const result: AdapterResult = {
    quality,
    data_saving: dataSaving,
    detail: `${adapter.name}: quality ${quality}, data saving ${dataSaving}`,
  };
  if (
    lastResult &&
    lastResult.quality === result.quality &&
    lastResult.data_saving === result.data_saving
  ) {
    return;
  }
  lastResult = result;
  void chrome.runtime.sendMessage({
    type: "video_status",
    status: {
      host,
      quality: result.quality,
      data_saving: result.data_saving,
      detail: result.detail,
      ts_ms: Date.now(),
    },
  });
}

// ---------------------------------------------------------------------------
// Usage measurement
// ---------------------------------------------------------------------------

const BITRATE_KBPS: Array<[number, number]> = [
  [144, 96],
  [240, 160],
  [360, 320],
  [480, 560],
  [720, 1_200],
  [1080, 2_400],
  [2_160, 6_000],
];

function bitrateKbps(): number {
  const video = document.querySelector<HTMLVideoElement>("video");
  const height = video?.videoHeight ?? settings.quality_height ?? 480;
  for (const [h, kbps] of BITRATE_KBPS) {
    if (height <= h) return kbps;
  }
  return BITRATE_KBPS[BITRATE_KBPS.length - 1][1];
}

let measuredBytes = 0;
let mediaSecs = 0;
let lastTick = Date.now();

function collectEntries(entries: PerformanceEntryList): void {
  for (const entry of entries) {
    if (entry.entryType !== "resource") continue;
    const r = entry as PerformanceResourceTiming;
    if (r.transferSize > 0) measuredBytes += r.transferSize;
  }
}

const observer = new PerformanceObserver((list) => collectEntries(list.getEntries()));
observer.observe({ type: "resource", buffered: true });

function tick(): void {
  const now = Date.now();
  const elapsed = Math.min(now - lastTick, 15_000) / 1000;
  lastTick = now;

  const video = document.querySelector<HTMLVideoElement>("video");
  const playing = video ? !video.paused && !video.ended && video.readyState > 2 : false;
  if (playing && !document.hidden) {
    mediaSecs += elapsed;
  }

  const mediaBytes = (mediaSecs * bitrateKbps() * 1000) / 8;
  const delta = Math.round(measuredBytes + mediaBytes);
  if (delta <= 0) return;
  // Deltas: the background accumulates them into a per-day ledger and forwards
  // the ledger total to the service. `estimated` is always true here.
  void chrome.runtime.sendMessage({
    type: "content_usage",
    host,
    bytes: delta,
    estimated: true,
  });
  measuredBytes = 0;
  mediaSecs = 0;
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

chrome.runtime.sendMessage({ type: "content_settings" }, (reply: unknown) => {
  const got = (reply as { settings?: VideoSettings } | undefined)?.settings;
  if (got) Object.assign(settings, got);
  runAdapters();
});

window.setInterval(tick, 10_000);
document.addEventListener("visibilitychange", runAdapters);
window.addEventListener("yt-navigate-finish", () => {
  lastResult = null;
  window.setTimeout(runAdapters, 1_500);
});
window.addEventListener("popstate", () => {
  lastResult = null;
  window.setTimeout(runAdapters, 1_500);
});

// Re-run after late player creation without polling the whole page constantly.
let runs = 0;
const bootPoll = window.setInterval(() => {
  runs += 1;
  runAdapters();
  if (runs >= 10) window.clearInterval(bootPoll);
}, 1_000);
