const m10El = document.getElementById("m10");
const h1El = document.getElementById("h1");
const h3El = document.getElementById("h3");
const fabEl = document.getElementById("fab");
const dayFill = document.getElementById("dayFill");
const dayPct = document.getElementById("dayPct");
const weekFill = document.getElementById("weekFill");
const weekPct = document.getElementById("weekPct");
const chip = document.getElementById("chip");
const canvas = document.getElementById("bug");
const ctx = canvas.getContext("2d");

let spend10 = 0;
let x = 22;
let dir = 1;
let last = performance.now();

function fmtUsd(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const v = Math.abs(n);
  if (v >= 1000) return `$${v.toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
  if (v >= 10) return `$${v.toFixed(2)}`;
  return `$${v.toFixed(3)}`;
}

function heat(usd10) {
  return Math.min(1, Math.max(0, usd10 || 0) / 100);
}

function fmtPct(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const v = Math.max(0, n);
  if (v >= 100) return `${v.toFixed(0)}%`;
  if (v >= 10) return `${v.toFixed(0)}%`;
  return `${v.toFixed(1)}%`;
}

function fmtCountdown(secs) {
  if (secs == null) return "";
  const s = Math.max(0, secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}h${String(m).padStart(2, "0")}m` : `${m}m`;
}

function quotaTone(pct) {
  if (pct >= 90) return "hot";
  if (pct >= 70) return "warn";
  return "";
}

function microUsd(n) {
  return (n || 0) / 1e6;
}

function pickLimit(limits, window, model) {
  const want = model || null;
  return (
    (limits || []).find((l) => {
      const mf = l.model_filter || null;
      return l.limit_window === window && mf === want;
    }) || null
  );
}

function resetSecs(iso) {
  if (!iso) return null;
  const raw = /Z|[+-]\d{2}:?\d{2}$/.test(iso) ? iso : `${iso}Z`;
  const t = Date.parse(raw);
  if (Number.isNaN(t)) return null;
  return Math.max(0, Math.round((t - Date.now()) / 1000));
}

function paintBar(fillEl, pctEl, pct) {
  const p = pct == null || Number.isNaN(pct) ? 0 : pct;
  const tone = quotaTone(p);
  fillEl.style.width = `${Math.min(100, Math.max(0, p))}%`;
  fillEl.className = `fill ${tone}`.trim();
  pctEl.textContent = pct == null || Number.isNaN(pct) ? "--" : fmtPct(p);
  pctEl.className = `pct ${tone}`.trim();
}

function limitLine(l, label) {
  if (!l) return `${label} --`;
  const remain = microUsd(l.remaining_value);
  const inSecs = resetSecs(l.reset_at);
  const reset = inSecs == null ? "" : ` · ${fmtCountdown(inSecs)}`;
  const model = l.model_filter ? ` ${l.model_filter}` : "";
  return `${label}${model} ${fmtPct(l.used_percent)}  ${fmtUsd(microUsd(l.current_value))} / ${fmtUsd(microUsd(l.max_value))}  left ${fmtUsd(remain)}${reset}`;
}

function speedFromSpend(usd10) {
  return 7 + Math.max(0, usd10 || 0) * 1.45;
}

function circle(px, py, r, color) {
  ctx.beginPath();
  ctx.fillStyle = color;
  ctx.arc(px, py, r, 0, Math.PI * 2);
  ctx.fill();
}

function drawCrawfish(t, frenzy) {
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);

  const s = dir;
  const bounce = Math.abs(Math.sin(t * (5 + frenzy * 10))) * (1.2 + frenzy * 1.8);
  const y = h * 0.62 - bounce;
  const squash = 1 + Math.sin(t * (5 + frenzy * 10)) * 0.06;
  const stretch = 1 - Math.sin(t * (5 + frenzy * 10)) * 0.05;

  ctx.save();
  ctx.translate(x, y);
  ctx.scale(s * stretch, squash);

  // tail
  for (let i = -2; i <= 2; i++) {
    const ang = i * 0.42 + Math.sin(t * (4 + frenzy * 8)) * 0.12;
    circle(16 + Math.cos(ang) * 6, Math.sin(ang) * 4.5, 2.1, "rgba(255,140,70,0.75)");
  }

  // body
  circle(4, 1, 9.2, "#ff7a2a");
  circle(7, 0.4, 8.4, "#ff8a3a");
  circle(1, 0.2, 7.6, "#ff9a4a");

  // blush
  circle(-1.2, 2.6, 1.7, "rgba(255,120,140,0.55)");
  circle(6.4, 2.8, 1.7, "rgba(255,120,140,0.55)");

  // claws — chubby mittens
  const pinch = Math.sin(t * (4 + frenzy * 9)) * (1.1 + frenzy);
  circle(-9.5, -6.2 + pinch * 0.2, 4.1, "#ff7a2a");
  circle(-12.4, -7.4 + pinch * 0.35, 2.4, "#ff9a48");
  circle(-9.2, 5.4 - pinch * 0.15, 3.8, "#ff7a2a");
  circle(-12.0, 6.6 - pinch * 0.3, 2.2, "#ff9a48");

  // legs
  for (let i = 0; i < 3; i++) {
    const phase = t * (7 + frenzy * 16) + i * 1.05;
    const kick = Math.sin(phase) * (2.4 + frenzy * 2);
    const lx = -1 + i * 5.2;
    circle(lx, 7.4, 1.35, "#e86a20");
    circle(lx + kick * 0.55, 10.4 + Math.abs(kick) * 0.12, 1.25, "#ff8a3a");
  }

  // eyes — big and shiny
  circle(-0.6, -3.4, 3.35, "#fffaf4");
  circle(6.2, -3.2, 3.35, "#fffaf4");
  const look = s * 0.55;
  circle(-0.6 + look, -3.2, 1.55, "#2a1208");
  circle(6.2 + look, -3.0, 1.55, "#2a1208");
  circle(-1.3 + look, -4.0, 0.7, "#fff");
  circle(5.5 + look, -3.8, 0.7, "#fff");

  // tiny smile
  ctx.beginPath();
  ctx.strokeStyle = "#c45a20";
  ctx.lineWidth = 1.1;
  ctx.lineCap = "round";
  ctx.arc(2.8, 1.1, 2.2, 0.15, Math.PI - 0.15);
  ctx.stroke();

  ctx.restore();
}

function tick(now) {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  const spd = speedFromSpend(spend10);
  const frenzy = heat(spend10);
  const minX = 16;
  const maxX = canvas.width - 22;
  let dist = spd * dt;
  let hops = 0;
  while (dist > 0.0001 && hops++ < 48) {
    const room = dir > 0 ? maxX - x : x - minX;
    if (room <= 0) {
      dir *= -1;
      continue;
    }
    if (dist <= room) {
      x += dir * dist;
      dist = 0;
    } else {
      x = dir > 0 ? maxX : minX;
      dir *= -1;
      dist -= room;
    }
  }
  drawCrawfish(now / 1000, frenzy);
  requestAnimationFrame(tick);
}

function apply(q) {
  chip.classList.remove("setup");
  if (!q) return;
  if (q.error) {
    m10El.textContent = q.error;
    h1El.textContent = "";
    h3El.textContent = "";
    fabEl.textContent = "";
    paintBar(dayFill, dayPct, null);
    paintBar(weekFill, weekPct, null);
    dayPct.textContent = "";
    weekPct.textContent = "";
    if (q.error === "no api key") chip.classList.add("setup");
    chip.title = q.error;
    spend10 = 0;
    return;
  }
  spend10 = q.spend_10m || 0;
  m10El.textContent = fmtUsd(q.spend_10m);
  h1El.textContent = fmtUsd(q.spend_1h);

  const day = pickLimit(q.limits, "daily");
  const week = pickLimit(q.limits, "weekly");
  const threeH = pickLimit(q.limits, "3h");
  const fabDay = pickLimit(q.limits, "daily", "claude-fable-5");
  paintBar(dayFill, dayPct, day ? day.used_percent : q.daily_pct);
  paintBar(weekFill, weekPct, week ? week.used_percent : null);
  paintRemain(h3El, threeH);
  paintRemain(fabEl, fabDay);

  chip.title = [
    limitLine(day, "day"),
    limitLine(week, "week"),
    limitLine(threeH, "3h"),
    limitLine(fabDay, "fable daily"),
    `10m ${fmtUsd(q.spend_10m)} · 1h ${fmtUsd(q.spend_1h)}`,
    "가재 속도 = 10분 달러, 상한 없음",
    "드래그해서 이동 · 더블클릭 위치 리셋 · 우클릭 통계",
  ]
    .filter(Boolean)
    .join("\n");
}

function paintRemain(el, limit) {
  if (!limit) {
    el.textContent = "--";
    el.classList.remove("hot");
    return;
  }
  el.textContent = fmtUsd(microUsd(limit.remaining_value));
  el.classList.toggle("hot", (limit.used_percent || 0) >= 90 || (limit.remaining_value || 0) <= 0);
}

window.addEventListener("DOMContentLoaded", async () => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  requestAnimationFrame(tick);
  await listen("quota-update", (ev) => apply(ev.payload));

  let dragging = false;
  let moved = false;
  let lastX = 0;
  const THRESH = 4;

  chip.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    dragging = true;
    moved = false;
    lastX = e.clientX;
    chip.setPointerCapture(e.pointerId);
    invoke("begin_bar_drag").catch(() => {});
  });
  chip.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    const dxCss = e.clientX - lastX;
    if (!moved && Math.abs(dxCss) < THRESH) return;
    moved = true;
    document.body.classList.add("dragging");
    const dx = Math.round(dxCss * (window.devicePixelRatio || 1));
    lastX = e.clientX;
    if (dx !== 0) invoke("nudge_bar", { dx }).catch(() => {});
  });
  function endDrag(e) {
    if (!dragging) return;
    dragging = false;
    document.body.classList.remove("dragging");
    try {
      chip.releasePointerCapture(e.pointerId);
    } catch (_) {}
    invoke("end_bar_drag").catch(() => {});
    if (!moved) invoke("refresh_now").catch(() => {});
  }
  chip.addEventListener("pointerup", endDrag);
  chip.addEventListener("pointercancel", endDrag);
  chip.addEventListener("dblclick", (e) => {
    e.preventDefault();
    invoke("reset_bar_position").catch(() => {});
  });
  chip.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("open_stats").catch(() => {});
  });
  try {
    apply(await invoke("current_quota"));
  } catch {
    m10El.textContent = "starting…";
  }
});
