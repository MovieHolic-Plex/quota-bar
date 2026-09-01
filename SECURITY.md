# Security

Quota Bar talks to whatever Anthropic-compatible base URL you configure. Treat that endpoint as trusted.

## API keys

- Do **not** commit API keys, `.env` files, or Credential Manager exports.
- The app stores the key in Windows Credential Manager (`dev.quotabar.desktop` / `api-key`).
- Settings UI never echoes a saved key back to the renderer; a blank field means "keep the existing key".
- Logs and events redact the secret if it appears in an error string.

## Polling

Each refresh sends a 1-token `POST /v1/messages` probe. That consumes a tiny amount of quota. Raise `poll_interval_secs` if you want fewer probes.
