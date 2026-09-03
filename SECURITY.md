# Security

Quota Bar talks to whatever Anthropic-compatible base URL you configure in **Settings**. Treat that endpoint as trusted.

## Secrets stay off this repository

- Do **not** commit API keys, `.env` files, Credential Manager exports, or `usage.db`.
- There is no `.env.example` and no sample key in this repo on purpose.
- On Windows the key is stored only in **Credential Manager** (`dev.quotabar.desktop` / `api-key`).
- Settings never echo a saved key back to the renderer. A blank field means “keep the existing key”.
- Logs and events redact the secret if it appears in an error string.
- SQLite snapshots store usage counters only — never the key.

## Polling

Each refresh is `GET /v1/usage/self`. Raise `poll_interval_secs` in Settings if you want fewer requests.
