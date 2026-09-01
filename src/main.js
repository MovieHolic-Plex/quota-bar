const fill5 = document.getElementById("fill5");
const fill7 = document.getElementById("fill7");
const pct5 = document.getElementById("pct5");
const pct7 = document.getElementById("pct7");
const meta = document.getElementById("meta");
const chip = document.getElementById("chip");

function level(remaining) {
  if (remaining == null || Number.isNaN(remaining)) return "";
  if (remaining < 0.15) return "bad";
  if (remaining < 0.4) return "warn";
  return "";
}

function fmtPct(remaining) {
  if (remaining == null) return "--";
  return `${Math.round(remaining * 100)}%`;
}

function fmtReset(unix) {
  if (!unix) return "";
  const ms = unix * 1000 - Date.now();
  if (ms <= 0) return "reset now";
  const m = Math.round(ms / 60000);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  if (h < 48) return rm ? `${h}h ${rm}m` : `${h}h`;
  return `${Math.round(h / 24)}d`;
}

function apply(q) {
  chip.classList.remove("pulse", "setup");
  if (!q) return;

  if (q.error) {
    meta.textContent = q.error;
    if (q.error === "no api key") chip.classList.add("setup");
    return;
  }

  const r5 = q.remaining_5h;
  const r7 = q.remaining_7d;
  fill5.style.width = r5 == null ? "0%" : `${Math.max(0, Math.min(1, r5)) * 100}%`;
  fill7.style.width = r7 == null ? "0%" : `${Math.max(0, Math.min(1, r7)) * 100}%`;
  fill5.className = `fill ${level(r5)}`.trim();
  fill7.className = `fill ${level(r7)}`.trim();
  pct5.textContent = fmtPct(r5);
  pct7.textContent = fmtPct(r7);

  const a = fmtReset(q.reset_5h);
  const b = fmtReset(q.reset_7d);
  meta.textContent = a ? `5h ${a}` : b || "ok";

  if (q.status_5h && /reject|exceed|limit/i.test(q.status_5h)) {
    chip.classList.add("pulse");
    meta.textContent = "5h exhausted";
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  await listen("quota-update", (ev) => apply(ev.payload));

  chip.addEventListener("click", () => {
    invoke("refresh_now").catch(() => {});
  });

  chip.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("open_settings").catch(() => {});
  });

  try {
    const current = await invoke("current_quota");
    apply(current);
  } catch {
    meta.textContent = "starting…";
  }
});
