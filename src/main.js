const cacheEl = document.getElementById("cache");
const gainEl = document.getElementById("gain");
const spark = document.getElementById("spark");
const chip = document.getElementById("chip");

function fmtUsd(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const sign = n < 0 ? "-" : "";
  const v = Math.abs(n);
  if (v >= 100000) return `${sign}$${(v / 1000).toFixed(0)}k`;
  if (v >= 10000) return `${sign}$${(v / 1000).toFixed(1)}k`;
  if (v >= 1000) return `${sign}$${v.toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
  return `${sign}$${v.toFixed(2)}`;
}

function fmtB(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const b = n / 1e9;
  if (Math.abs(b) >= 1) return `${b.toFixed(2)}B`;
  if (Math.abs(n) >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (Math.abs(n) >= 1e3) return `${(n / 1e3).toFixed(0)}K`;
  return String(Math.round(n));
}

function paintSpark(minutes) {
  spark.innerHTML = "";
  const rows = minutes || [];
  const max = Math.max(0.0001, ...rows.map((r) => r.cost_usd || 0));
  rows.forEach((r) => {
    const b = document.createElement("b");
    const h = Math.max(1, Math.round(((r.cost_usd || 0) / max) * 32));
    b.style.height = `${h}px`;
    if (!r.cost_usd) b.classList.add("dim");
    const t = new Date((r.start_ts || 0) * 1000);
    const hh = String(t.getHours()).padStart(2, "0");
    const mm = String(t.getMinutes()).padStart(2, "0");
    b.title = `${hh}:${mm}  ${fmtUsd(r.cost_usd)}  ${fmtB(r.tokens)} tok`;
    spark.appendChild(b);
  });
}

function apply(q) {
  chip.classList.remove("setup");
  if (!q) return;
  if (q.error) {
    cacheEl.textContent = q.error;
    gainEl.textContent = "";
    spark.innerHTML = "";
    if (q.error === "no api key") chip.classList.add("setup");
    chip.title = q.error;
    return;
  }
  const pct = q.cache_pct != null ? q.cache_pct : 0;
  cacheEl.textContent = `${pct.toFixed(1)}%`;
  const sav = q.savings_usd || 0;
  gainEl.textContent = `${sav >= 0 ? "+" : "-"}${fmtUsd(Math.abs(sav))}`;
  paintSpark(q.minutes || []);
  chip.title = [
    `cache ${pct.toFixed(1)}%  (${fmtB(q.cached_input_tokens)} / ${fmtB(q.total_tokens)})`,
    `API ${fmtUsd(q.total_cost_usd)} − Pro ${fmtUsd(q.pro_usd)} = ${fmtUsd(q.savings_usd)}`,
    `${Number(q.request_count || 0).toLocaleString()} requests · 오른쪽은 1분 사용량`,
    "Click refresh · Right-click stats",
  ].join("\n");
}

window.addEventListener("DOMContentLoaded", async () => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  await listen("quota-update", (ev) => apply(ev.payload));
  chip.addEventListener("click", () => invoke("refresh_now").catch(() => {}));
  chip.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("open_stats").catch(() => {});
  });
  try {
    apply(await invoke("current_quota"));
  } catch {
    cacheEl.textContent = "starting…";
  }
});
