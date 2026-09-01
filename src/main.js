const tok = document.getElementById("tok");
const cache = document.getElementById("cache");
const api = document.getElementById("api");
const gain = document.getElementById("gain");
const chip = document.getElementById("chip");

function fmtB(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const b = n / 1e9;
  if (b >= 100) return `${b.toFixed(1)}B`;
  if (b >= 10) return `${b.toFixed(2)}B`;
  if (b >= 1) return `${b.toFixed(3)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(Math.round(n));
}

function fmtUsd(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const sign = n < 0 ? "-" : "";
  const v = Math.abs(n);
  if (v >= 100000) return `${sign}$${(v / 1000).toFixed(0)}k`;
  if (v >= 10000) return `${sign}$${(v / 1000).toFixed(1)}k`;
  if (v >= 1000) return `${sign}$${v.toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
  return `${sign}$${v.toFixed(2)}`;
}

function apply(q) {
  chip.classList.remove("setup");
  if (!q) return;
  if (q.error) {
    tok.textContent = q.error;
    cache.textContent = "";
    api.textContent = "";
    gain.textContent = "";
    if (q.error === "no api key") chip.classList.add("setup");
    chip.title = q.error;
    return;
  }
  tok.textContent = fmtB(q.total_tokens);
  cache.textContent = fmtB(q.cached_input_tokens);
  api.textContent = fmtUsd(q.total_cost_usd);
  const sav = q.savings_usd || 0;
  gain.textContent = `${sav >= 0 ? "+" : "-"}${fmtUsd(Math.abs(sav))}`;
  const cachePct =
    q.total_tokens > 0 ? ((q.cached_input_tokens / q.total_tokens) * 100).toFixed(1) : "0";
  chip.title = [
    `tokens ${fmtB(q.total_tokens)}  cache ${fmtB(q.cached_input_tokens)} (${cachePct}%)`,
    `API equivalent ${fmtUsd(q.total_cost_usd)}  paid ${fmtUsd(q.paid_usd)}  savings ${fmtUsd(q.savings_usd)}`,
    `${q.request_count} requests`,
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
    tok.textContent = "starting…";
  }
});
