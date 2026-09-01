# Quota Bar

A tiny Windows taskbar overlay for Anthropic-compatible Claude proxies (for example [claude-lb](https://claude.nekos.me)).

It sits on the taskbar next to the clock cluster and shows **cumulative usage from `GET /v1/usage/self`**, not the 5-hour rate-limit window (that window hops between load-balanced accounts).

## What it shows

| Field | Source |
| --- | --- |
| **orange crayfish** | crawls faster when the last 10 minutes cost more |
| **10m** | API-equivalent dollars in the last 10 minutes |
| **1h** | last hour |
| **1d** | last 24 hours |
| **3d** | last 3 days |

Click the bar to refresh. Right-click opens **Stats**.

## Logging and stats

Every successful poll is appended to SQLite:

`%APPDATA%\quotabar\quota-bar\usage.db`

Time-band tables (1h / 5h / 24h / 7d / 30d / all) and hourly/daily charts are **deltas between snapshots** of the cumulative counters. Leave it running; the history is what makes the stats useful.

Tray menu: Show bar · Stats · Refresh · Settings · Quit.

## Privacy

- API keys are not in this repository.
- On Windows the key is stored in **Credential Manager** (`dev.quotabar.desktop` / `api-key`).
- Settings live in `%APPDATA%\quotabar\quota-bar\config.json`.
- The key is never written to SQLite, logs, or UI events.

## Setup

### 1. Build tools (Windows)

- [Rust](https://rustup.rs/) (MSVC toolchain)
- [Visual Studio 2022 Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with **Desktop development with C++**
- WebView2
- Node.js 18+

### 2. Run from source

```powershell
git clone https://github.com/MovieHolic-Plex/quota-bar.git
cd quota-bar
npm install
npm run dev
```

```powershell
npm run build
```

Installer: `src-tauri/target/release/bundle/nsis/`.

### 3. API key

Tray → **Settings**, or user environment variables:

```text
ANTHROPIC_BASE_URL=https://claude.nekos.me
ANTHROPIC_API_KEY=sk-clb-...
```

Savings are **API list-price equivalent minus Claude Pro ($20/mo)**. Change the Pro amount in Settings if you compare against Max.

## Polling

`GET /v1/usage/self` every 60 seconds (configurable). No `POST /v1/messages` probes.

## Taskbar placement

Windows 11 draws the XAML taskbar over explorer child windows, so this stays a top-level popup flush on the taskbar, left of the clock / TrafficMonitor cluster. It only moves when that cluster changes.

## Config

`%APPDATA%\quotabar\quota-bar\config.json`

```json
{
  "base_url": "https://claude.nekos.me",
  "poll_interval_secs": 60,
  "bar_width": 420,
  "paid_usd": 0
}
```

## License

MIT
