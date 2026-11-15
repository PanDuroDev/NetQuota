# Video adapters

## Contract

Every adapter implements the same two operations and must report one of three
states per operation:

| State | Meaning |
| --- | --- |
| `enforced` | Applied **and verified** (read back from the player, or the action is deterministic such as `pause()`) |
| `best_effort` | Attempted; the site may override it |
| `unsupported` | The site exposes no reliable control |

```ts
interface VideoAdapter {
  name: string;
  matches(): boolean;
  applyQuality(height: number | null): Support;
  reduceData(settings: VideoSettings): Support;
}
```

The registry in `apps/chrome-extension/src/content.ts` is ordered
specific → generic. Adding a platform means adding one object; the worker, popup,
and quota code do not change.

Adapters report their last result to the worker, which stores it per host and
shows it in the popup. A status is never upgraded to `enforced` after the fact.

## Profiles

| Profile | Target height |
| --- | --- |
| Ultra Saver | 144p |
| Extreme Saver | 240p |
| Balanced Saver | 360p |
| Normal Saver | 480p |
| Custom | user-selected |

`apply_profile` in the service maps a `ProfileKind` to a preferred height and
returns it in the snapshot; the extension writes it into its settings, which the
content script reads.

## YouTube

The adapter uses the on-page player element (`#movie_player`):

- `setPlaybackQualityRange(level, level)` with a level derived from the target
  height (`tiny`, `small`, `medium`, `large`, `hd720`, `hd1080`, …).
- Immediately reads `getPlaybackQuality()`. If the reported level is at or below
  the target, the result is `enforced`; otherwise `best_effort`.
- Re-runs on `yt-navigate-finish` so in-app navigation to another video does not
  leave the previous choice in place.

Why not the IFrame API: `setPlaybackQuality` is deprecated and no longer sets
quality. Why not `ytplayer.config`: it is not present on modern watch pages and
would be a fragile internal detail. The player methods are also unofficial, which
is exactly why the adapter verifies and downgrades to `best_effort` when the
player ignores the request.

Data-saving actions that are deterministic: `preload = "none"`, pause on hidden
tab (when enabled), pause off-screen embeds.

## TikTok

No quality selector is exposed to page scripts in a stable way; forcing a
rendition would require intercepting media manifests, which this project does not
do. Therefore:

- `applyQuality` → `unsupported`.
- `reduceData` → pauses off-screen reels, pauses all video when the tab is
  hidden, sets `preload = "none"`.

The popup shows `quality: unsupported` for TikTok. That is the honest result, not
a bug.

## Generic HTML5

- Quality → `unsupported`: an `HTMLVideoElement` has no API to choose a rendition
  of HLS, DASH, or Media Source content.
- Data saving → pauses hidden-tab video, trims `preload` from `auto` to
  `metadata`, and optionally pauses autoplaying video when "prevent autoplay" is
  enabled.

## What this project will not do

- No HTTPS interception, no custom root certificate, no `webRequest` body
  inspection.
- No DRM circumvention.
- No manifest rewriting for adaptive streaming. If a site does not expose a
  control, the adapter says so.

## Future platforms

Twitch, Instagram Reels, Facebook Video, Reddit Video, and X Video all expose
either a quality menu or a player object. Each gets its own adapter with its own
verification step; none of them inherits `enforced` from another site.
