const m10El = document.getElementById("m10");
const h1El = document.getElementById("h1");
const d1El = document.getElementById("d1");
const qfill = document.getElementById("qfill");
const qpct = document.getElementById("qpct");
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

function quotaTone(pct) {
  if (pct >= 90) return "hot";
  if (pct >= 70) return "warn";
  return "";
}

function speedFromSpend(usd10) {
  return 7 + heat(usd10) * 145;
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
  x += dir * spd * dt;
  const minX = 16;
  const maxX = canvas.width - 22;
  if (x > maxX) {
    x = maxX;
    dir = -1;
  } else if (x < minX) {
    x = minX;
    dir = 1;
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
    d1El.textContent = "";
    qpct.textContent = "";
    qfill.style.width = "0%";
    qfill.className = "fill";
    qpct.className = "pct";
    if (q.error === "no api key") chip.classList.add("setup");
    chip.title = q.error;
    spend10 = 0;
    return;
  }
  spend10 = q.spend_10m || 0;
  const pct = q.daily_pct || 0;
  const cap = q.daily_quota_usd || 6400;
  const tone = quotaTone(pct);
  m10El.textContent = fmtUsd(q.spend_10m);
  h1El.textContent = fmtUsd(q.spend_1h);
  d1El.textContent = fmtUsd(q.spend_1d);
  qpct.textContent = fmtPct(pct);
  qfill.style.width = `${Math.min(100, Math.max(0, pct))}%`;
  qfill.className = `fill ${tone}`.trim();
  qpct.className = `pct ${tone}`.trim();
  chip.title = [
    `daily ${fmtPct(pct)}  ${fmtUsd(q.spend_1d)} / ${fmtUsd(cap)}`,
    `10m ${fmtUsd(q.spend_10m)} · 1h ${fmtUsd(q.spend_1h)}`,
    "Pro 하루 한도 $6400 · 가재 최고속은 10분에 $100부터",
    "드래그해서 이동 · 더블클릭 위치 리셋 · 우클릭 통계",
  ].join("\n");
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
