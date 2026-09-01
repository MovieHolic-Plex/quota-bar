const m10El = document.getElementById("m10");
const h1El = document.getElementById("h1");
const chip = document.getElementById("chip");
const canvas = document.getElementById("bug");
const ctx = canvas.getContext("2d");

let spend10 = 0;
let x = 18;
let dir = 1;
let last = performance.now();

function fmtUsd(n) {
  if (n == null || Number.isNaN(n)) return "--";
  const v = Math.abs(n);
  if (v >= 1000) return `$${v.toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
  if (v >= 10) return `$${v.toFixed(2)}`;
  return `$${v.toFixed(3)}`;
}

function speedFromSpend(usd10) {
  const v = Math.max(0, usd10 || 0);
  return 14 + Math.min(110, v * 18);
}

function frenzyFromSpend(usd10) {
  return Math.min(1, Math.max(0, usd10 || 0) / 8);
}

function dot(px, py, r, a) {
  ctx.beginPath();
  ctx.fillStyle = `rgba(255, ${Math.round(90 + a * 40)}, 20, ${0.75 + a * 0.25})`;
  ctx.arc(px, py, r, 0, Math.PI * 2);
  ctx.fill();
}

function drawCrawfish(t, frenzy) {
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  const y = h * 0.58;
  const s = dir;
  const wiggle = Math.sin(t * (8 + frenzy * 18));
  const bob = Math.sin(t * (5 + frenzy * 10)) * (1 + frenzy);

  const body = [
    [0, 0, 2.4],
    [4, 0.4, 2.6],
    [8, 0.2, 2.7],
    [12, 0, 2.4],
    [16, -0.3, 2.1],
    [20, -0.6, 1.7],
  ];
  for (const [bx, by, r] of body) {
    dot(x + s * bx, y + by + bob * 0.3, r, 0.7);
  }

  // tail fan
  for (let i = -2; i <= 2; i++) {
    const ang = i * 0.35 + wiggle * 0.12;
    dot(x + s * (24 + Math.cos(ang) * 5), y - 0.8 + Math.sin(ang) * 5 + bob, 1.3, 0.45);
  }

  // claws
  const pinch = Math.sin(t * (6 + frenzy * 14)) * (2 + frenzy * 2);
  dot(x + s * (-7), y - 5 + pinch * 0.15, 2.3, 1);
  dot(x + s * (-11), y - 7 + pinch * 0.3, 1.7, 0.9);
  dot(x + s * (-6), y + 3 - pinch * 0.1, 2.1, 1);
  dot(x + s * (-10), y + 6 - pinch * 0.25, 1.6, 0.9);

  // legs 뽈뽈뽈
  for (let i = 0; i < 4; i++) {
    const phase = t * (10 + frenzy * 22) + i * 0.9;
    const kick = Math.sin(phase) * (3.2 + frenzy * 2.4);
    const lx = x + s * (3 + i * 4.2);
    dot(lx, y + 4.8 + Math.abs(kick) * 0.15, 1.15, 0.55);
    dot(lx + s * kick * 0.35, y + 8.2 + kick * 0.2, 1.05, 0.4);
  }

  // eyes
  dot(x + s * (-2.2), y - 3.2, 1.15, 1);
  ctx.beginPath();
  ctx.fillStyle = "#1a0a00";
  ctx.arc(x + s * (-2.4), y - 3.2, 0.45, 0, Math.PI * 2);
  ctx.fill();
}

function tick(now) {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  const spd = speedFromSpend(spend10);
  const frenzy = frenzyFromSpend(spend10);
  x += dir * spd * dt;
  const minX = 14;
  const maxX = canvas.width - 28;
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
    if (q.error === "no api key") chip.classList.add("setup");
    chip.title = q.error;
    spend10 = 0;
    return;
  }
  spend10 = q.spend_10m || 0;
  m10El.textContent = fmtUsd(q.spend_10m);
  h1El.textContent = fmtUsd(q.spend_1h);
  chip.title = [
    `지난 10분 ${fmtUsd(q.spend_10m)}`,
    `지난 1시간 ${fmtUsd(q.spend_1h)}`,
    "가재는 최근 10분 소비가 클수록 빨리 기어다님",
    "Click refresh · Right-click stats",
  ].join("\n");
}

window.addEventListener("DOMContentLoaded", async () => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  requestAnimationFrame(tick);
  await listen("quota-update", (ev) => apply(ev.payload));
  chip.addEventListener("click", () => invoke("refresh_now").catch(() => {}));
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
