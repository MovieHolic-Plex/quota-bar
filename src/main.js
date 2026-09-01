const fill5 = document.getElementById("fill5");
const fill7 = document.getElementById("fill7");
const pct5 = document.getElementById("pct5");
const pct7 = document.getElementById("pct7");
const when5 = document.getElementById("when5");
const when7 = document.getElementById("when7");
const chip = document.getElementById("chip");

let latest = null;

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

function fmtRemain(unix) {
  if (!unix) return "";
  const ms = unix * 1000 - Date.now();
  if (ms <= 0) return "now";
  const m = Math.max(0, Math.round(ms / 60000));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  if (h < 48) return rm ? `${h}h ${rm}m` : `${h}h`;
  const d = Math.floor(h / 24);
  const rh = h % 24;
  return rh ? `${d}d ${rh}h` : `${d}d`;
}

function fmtClock(unix) {
  if (!unix) return "";
  const d = new Date(unix * 1000);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
}

function fmtWhen(unix, withClock) {
  if (!unix) return "--";
  const remain = fmtRemain(unix);
  if (!withClock) return remain;
  return `${remain} · ${fmtClock(unix)}`;
}

function paintTimes() {
  if (!latest || latest.error) return;
  when5.textContent = fmtWhen(latest.reset_5h, true);
  when7.textContent = latest.reset_7d ? fmtRemain(latest.reset_7d) : "";
  const tip = latest.reset_5h
    ? `5h ${fmtPct(latest.remaining_5h)} left · resets ${fmtClock(latest.reset_5h)} (${fmtRemain(latest.reset_5h)})`
    : "Quota Bar";
  chip.title = `${tip}\nClick to refresh · Right-click for settings`;
}

function apply(q) {
  latest = q;
  chip.classList.remove("pulse", "setup");
  if (!q) return;

  if (q.error) {
    when5.textContent = q.error;
    when7.textContent = "";
    if (q.error === "no api key") chip.classList.add("setup");
    chip.title = q.error;
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
  paintTimes();

  if (q.status_5h && /reject|exceed|limit/i.test(q.status_5h)) {
    chip.classList.add("pulse");
    when5.textContent = `exhausted · ${fmtClock(q.reset_5h)}`;
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  await listen("quota-update", (ev) => apply(ev.payload));
  setInterval(paintTimes, 1000);

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
    when5.textContent = "starting…";
  }
});
