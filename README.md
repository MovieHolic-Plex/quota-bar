# Quota Bar

A tiny Windows taskbar overlay that shows remaining **5-hour** and **7-day** rate-limit quota for Anthropic-compatible Claude proxies (for example [claude-lb](https://claude.nekos.me)).

It sits **on the Windows taskbar** as a compact bar — the same strip as Start / the clock — not a normal desktop window.

## What it shows

| Segment | Meaning |
| --- | --- |
| **5h** | Remaining unified 5-hour window |
| **7d** | Remaining unified 7-day window |

Color: green while healthy, amber under 40% remaining, red under 15%.

Quota is read from Anthropic-style response headers:

- `anthropic-ratelimit-unified-5h-utilization`
- `anthropic-ratelimit-unified-7d-utilization`
- matching `*-reset` unix timestamps

`utilization` is **used** fraction. The bar displays **remaining** = `1 - utilization`.

## Privacy

- Your API key is **not** stored in this repository.
- On Windows the key is saved in **Credential Manager** (`dev.quotabar.desktop` / `api-key`).
- Non-secret settings live in `%APPDATA%\quotabar\quota-bar\config.json`.
- The key is never written to logs, events, or screenshots of config.

## Setup

### 1. Build tools (Windows)

- [Rust](https://rustup.rs/) (MSVC toolchain)
- [Visual Studio 2022 Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with **Desktop development with C++**
- WebView2 (already on current Windows 10/11)
- Node.js 18+

### 2. Run from source

```powershell
git clone https://github.com/MovieHolic-Plex/quota-bar.git
cd quota-bar
npm install
npm run dev
```

Release installer:

```powershell
npm run build
```

The NSIS installer is written to `src-tauri/target/release/bundle/nsis/`.

### 3. API key

Right-click the bar (or the tray icon) → **Settings**.

Or set user environment variables before the first launch:

```text
ANTHROPIC_BASE_URL=https://claude.nekos.me
ANTHROPIC_API_KEY=sk-clb-...
```

`ANTHROPIC_AUTH_TOKEN` is accepted as an alias for the key.

## How polling works

The proxy only reports live remaining quota on **`POST /v1/messages`** responses. A 1-token Haiku probe is used by default, every 3 minutes (configurable). Click the bar to refresh immediately.

`GET /v1/models` does **not** return rate-limit headers, so it cannot be used as a free poll.

## Taskbar placement

Windows 11's taskbar is a XAML island, so child windows parented to `Shell_TrayWnd` vanish. Quota Bar uses a top-level popup **flush on the taskbar, immediately left of the right-hand cluster** (clock, system tray, TrafficMonitor, and similar overlays) so it does not sit on top of them. The window is only moved when that cluster actually changes, which avoids flicker. The 5-hour row shows remaining percent plus **countdown and reset clock** (`3h 36m · 13:20`).

## Config

`%APPDATA%\quotabar\quota-bar\config.json`

```json
{
  "base_url": "https://claude.nekos.me",
  "poll_interval_secs": 180,
  "probe_model": "claude-haiku-4-5-20251001",
  "bar_width": 420
}
```

## License

MIT
