/* Quota Bar — taskbar strip.
   Row 1 answers "what stops me first", row 2 gives the long horizon, the
   spine colours the whole thing, and the crawfish runs at your burn rate. */
(function () {
  "use strict";

  var chip = document.getElementById("chip");
  var rowsEl = document.getElementById("rows");
  var rowTop = document.getElementById("rowTop");
  var rowBottom = document.getElementById("rowBottom");
  var notice = document.getElementById("notice");
  var noticeText = document.getElementById("noticeText");
  var sparkCap = document.getElementById("sparkCap");

  var bugCanvas = document.getElementById("bug");
  var bugCtx = bugCanvas.getContext("2d");
  var sparkCanvas = document.getElementById("spark");
  var sparkCtx = sparkCanvas.getContext("2d");

  var TONES = ["t-ok", "t-warn", "t-hot", "t-crit", "t-none"];

  var spend10 = 0;
  var minutes = [];
  var bugX = 20;
  var bugDir = 1;
  var lastFrame = performance.now();

  /* ── canvas sizing ───────────────────────────────────────────────────
     The taskbar can sit on a 100%, 125% or 150% display. Size the backing
     store to device pixels or the crawfish turns to mush. */
  function fitCanvas(canvas, ctx) {
    var dpr = window.devicePixelRatio || 1;
    var rect = canvas.getBoundingClientRect();
    var w = Math.max(1, Math.round(rect.width));
    var h = Math.max(1, Math.round(rect.height));
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { w: w, h: h };
  }

  function setTone(el, tone) {
    TONES.forEach(function (t) {
      el.classList.remove(t);
    });
    el.classList.add("t-" + (tone || "none"));
  }

  /* ── crawfish ────────────────────────────────────────────────────────
     Speed is deliberately uncapped: a $400/10min burn should look insane. */
  function speedFromSpend(usd10) {
    return 7 + Math.max(0, usd10 || 0) * 1.45;
  }

  function frenzyFromSpend(usd10) {
    return Math.min(1, Math.max(0, usd10 || 0) / 100);
  }

  function dot(ctx, px, py, r, color) {
    ctx.beginPath();
    ctx.fillStyle = color;
    ctx.arc(px, py, r, 0, Math.PI * 2);
    ctx.fill();
  }

  function drawCrawfish(box, t, frenzy) {
    var ctx = bugCtx;
    ctx.clearRect(0, 0, box.w, box.h);

    var wobble = 5 + frenzy * 10;
    var bounce = Math.abs(Math.sin(t * wobble)) * (1.0 + frenzy * 1.6);
    var y = box.h * 0.62 - bounce;
    var squash = 1 + Math.sin(t * wobble) * 0.06;
    var stretch = 1 - Math.sin(t * wobble) * 0.05;

    /* soft contact shadow so it reads as standing on the taskbar */
    ctx.save();
    ctx.globalAlpha = 0.28;
    ctx.beginPath();
    ctx.fillStyle = "#000";
    ctx.ellipse(bugX, box.h * 0.62 + 9, 9, 2.1, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();

    ctx.save();
    ctx.translate(bugX, y);
    ctx.scale(bugDir * stretch, squash);

    /* tail */
    for (var i = -2; i <= 2; i++) {
      var ang = i * 0.42 + Math.sin(t * (4 + frenzy * 8)) * 0.12;
      dot(ctx, 14 + Math.cos(ang) * 5.4, Math.sin(ang) * 4.1, 1.9, "rgba(255,142,72,0.72)");
    }

    /* body */
    dot(ctx, 3.6, 0.9, 8.4, "#f0691c");
    dot(ctx, 6.4, 0.4, 7.6, "#ff8330");
    dot(ctx, 0.9, 0.2, 6.9, "#ff9a48");

    /* blush */
    dot(ctx, -1.0, 2.4, 1.5, "rgba(255,120,140,0.5)");
    dot(ctx, 5.8, 2.6, 1.5, "rgba(255,120,140,0.5)");

    /* claws */
    var pinch = Math.sin(t * (4 + frenzy * 9)) * (1.0 + frenzy);
    dot(ctx, -8.6, -5.6 + pinch * 0.2, 3.7, "#f0691c");
    dot(ctx, -11.2, -6.7 + pinch * 0.35, 2.2, "#ff9a48");
    dot(ctx, -8.4, 4.9 - pinch * 0.15, 3.4, "#f0691c");
    dot(ctx, -10.9, 6.0 - pinch * 0.3, 2.0, "#ff9a48");

    /* legs */
    for (var j = 0; j < 3; j++) {
      var phase = t * (7 + frenzy * 16) + j * 1.05;
      var kick = Math.sin(phase) * (2.2 + frenzy * 2);
      var lx = -0.9 + j * 4.7;
      dot(ctx, lx, 6.7, 1.25, "#d85c18");
      dot(ctx, lx + kick * 0.55, 9.4 + Math.abs(kick) * 0.12, 1.15, "#ff8330");
    }

    /* eyes */
    dot(ctx, -0.5, -3.1, 3.05, "#fffaf4");
    dot(ctx, 5.7, -2.9, 3.05, "#fffaf4");
    var look = bugDir * 0.5;
    dot(ctx, -0.5 + look, -2.9, 1.4, "#2a1208");
    dot(ctx, 5.7 + look, -2.7, 1.4, "#2a1208");
    dot(ctx, -1.1 + look, -3.6, 0.62, "#fff");
    dot(ctx, 5.1 + look, -3.4, 0.62, "#fff");

    /* smile */
    ctx.beginPath();
    ctx.strokeStyle = "#b8501a";
    ctx.lineWidth = 1;
    ctx.lineCap = "round";
    ctx.arc(2.5, 1.0, 2.0, 0.15, Math.PI - 0.15);
    ctx.stroke();

    ctx.restore();
  }

  function tick(now) {
    var box = fitCanvas(bugCanvas, bugCtx);
    var dt = Math.min(0.05, (now - lastFrame) / 1000);
    lastFrame = now;

    var speed = speedFromSpend(spend10);
    var minX = 14;
    var maxX = Math.max(minX + 1, box.w - 14);
    var dist = speed * dt;
    var hops = 0;
    while (dist > 0.0001 && hops++ < 64) {
      var room = bugDir > 0 ? maxX - bugX : bugX - minX;
      if (room <= 0) {
        bugDir *= -1;
        continue;
      }
      if (dist <= room) {
        bugX += bugDir * dist;
        dist = 0;
      } else {
        bugX = bugDir > 0 ? maxX : minX;
        bugDir *= -1;
        dist -= room;
      }
    }
    bugX = Math.min(maxX, Math.max(minX, bugX));

    drawCrawfish(box, now / 1000, frenzyFromSpend(spend10));
    requestAnimationFrame(tick);
  }

  /* ── sparkline ───────────────────────────────────────────────────────
     30 one-minute buckets of spend. Shows whether the number you are
     staring at is climbing or already over. */
  function drawSpark() {
    var box = fitCanvas(sparkCanvas, sparkCtx);
    var ctx = sparkCtx;
    ctx.clearRect(0, 0, box.w, box.h);
    if (!minutes.length) return;

    var peak = 0;
    minutes.forEach(function (m) {
      peak = Math.max(peak, m.cost_usd || 0);
    });

    var n = minutes.length;
    var gap = 1;
    var barW = Math.max(1, (box.w - gap * (n - 1)) / n);

    /* baseline so an all-zero stretch still reads as "nothing happened" */
    ctx.fillStyle = "rgba(255,255,255,0.10)";
    ctx.fillRect(0, box.h - 1, box.w, 1);

    if (peak <= 0) return;
    for (var i = 0; i < n; i++) {
      var v = minutes[i].cost_usd || 0;
      var h = v > 0 ? Math.max(1.5, (v / peak) * (box.h - 2)) : 0;
      if (h <= 0) continue;
      var x = i * (barW + gap);
      /* newest minutes brighter, so the recent edge pops */
      var a = 0.4 + 0.6 * (i / Math.max(1, n - 1));
      ctx.fillStyle = "rgba(255,138,58," + a.toFixed(3) + ")";
      ctx.fillRect(x, box.h - 1 - h, barW, h);
    }
  }

  /* ── meters ──────────────────────────────────────────────────────────*/
  function paintRow(row, limit, opts) {
    opts = opts || {};
    var lbl = row.querySelector(".mrow__lbl");
    var fill = row.querySelector(".meter__fill");
    var pace = row.querySelector(".meter__pace");
    var pctEl = row.querySelector(".mrow__pct");
    var auxEl = row.querySelector(".mrow__aux");

    if (!limit) {
      lbl.textContent = opts.label || "--";
      fill.style.width = "0%";
      pace.hidden = true;
      pctEl.textContent = "--";
      auxEl.textContent = "";
      setTone(row, "none");
      return;
    }

    lbl.textContent = opts.label || limit.shortName;
    fill.style.width = Math.min(100, Math.max(0, limit.usedPct)).toFixed(2) + "%";
    setTone(row, limit.tone);

    if (limit.elapsedPct == null) {
      pace.hidden = true;
    } else {
      pace.hidden = false;
      pace.style.left = Math.min(100, Math.max(0, limit.elapsedPct)).toFixed(2) + "%";
    }

    pctEl.textContent = limit.locked ? "FULL" : QB.pct(limit.usedPct);
    auxEl.textContent = opts.aux == null ? "" : opts.aux;
  }

  /* ── tooltip ─────────────────────────────────────────────────────────*/
  function tooltipFor(limits, burn, payload, err, age) {
    var lines = [];
    if (err) {
      lines.push("⚠ Last poll failed: " + err);
      lines.push(
        age == null
          ? "Showing the last successful read."
          : "Showing the read from " + QB.dur(age) + " ago."
      );
      lines.push("");
    }
    lines.push("Limits — used · remaining · resets in");
    limits.forEach(function (l) {
      lines.push(
        "  " +
          l.name +
          "  " +
          QB.pct(l.usedPct, 1) +
          "  " +
          QB.usd(l.used) +
          " / " +
          QB.usd(l.max) +
          "  " +
          QB.usd(l.left) +
          " left" +
          (l.resetIn == null ? "" : "  resets in " + QB.dur(l.resetIn))
      );
    });
    if (!limits.length) lines.push("  The proxy reported no limits[].");
    lines.push("");
    lines.push(
      "Burn rate  10m " +
        QB.usd(payload.spend_10m) +
        " · 1h " +
        QB.usd(payload.spend_1h) +
        " · 24h " +
        QB.usd(payload.spend_1d)
    );
    if (burn > 0) lines.push("Currently " + QB.usd(burn) + "/h — this is what sets the crawfish speed.");
    lines.push("");
    lines.push("Drag to move · wheel to resize · double-click to reset · click to refresh · right-click for stats");
    return lines.join("\n");
  }

  function showNotice(text, tone) {
    noticeText.textContent = text;
    setTone(notice, tone || "none");
    notice.hidden = false;
    rowsEl.style.visibility = "hidden";
  }

  function hideNotice() {
    notice.hidden = true;
    rowsEl.style.visibility = "";
  }

  /* ── apply ───────────────────────────────────────────────────────────*/
  function apply(payload) {
    if (!payload) return;

    var err = payload.error || null;
    var limits = QB.collectLimits(payload);
    var age = payload.fetched_at ? Math.max(0, Date.now() / 1000 - payload.fetched_at) : null;

    /* Only take the whole strip over when there is genuinely nothing to
       show: no key, or no successful poll has ever landed. */
    if (err && !limits.length) {
      minutes = [];
      spend10 = 0;
      drawSpark();
      sparkCap.textContent = "--";
      var setup = err === "no api key";
      showNotice(setup ? "No API key — tray icon → Settings" : err, setup ? "warn" : "crit");
      setTone(chip, setup ? "warn" : "crit");
      chip.classList.remove("is-alarm");
      chip.classList.remove("is-stale");
      chip.title = setup
        ? "Open the tray icon → Settings and enter the base URL and API key."
        : err;
      return;
    }

    hideNotice();

    /* A dropped request says nothing about the quota, so the figures stay
       and only their age changes. */
    chip.classList.toggle("is-stale", !!err);

    spend10 = payload.spend_10m || 0;
    minutes = payload.minutes || [];
    drawSpark();
    sparkCap.textContent = err
      ? age == null
        ? "stale"
        : QB.dur(age) + " old"
      : QB.usdc(payload.spend_10m) + "/10m";

    /* 1h delta is the steadiest rate we have; fall back to the last 10
       minutes scaled up when the hour is still filling. */
    var burn = payload.spend_1h > 0 ? payload.spend_1h : (payload.spend_10m || 0) * 6;

    if (!limits.length) {
      showNotice("Proxy reported no limits", "warn");
      setTone(chip, "none");
      chip.classList.remove("is-alarm");
      chip.title = tooltipFor(limits, burn, payload, err, age);
      return;
    }

    var binding = QB.pickBinding(limits, burn);
    /* Row 2 is the widest window we know about — the horizon behind the
       thing that is about to bite. */
    var horizon = null;
    limits.forEach(function (l) {
      if (l === binding) return;
      if (!horizon || (l.windowSecs || 0) > (horizon.windowSecs || 0)) horizon = l;
    });

    paintRow(rowTop, binding, {
      label: binding.shortName,
      aux: binding.resetIn == null ? QB.usdc(binding.left) : QB.dur(binding.resetIn)
    });
    paintRow(
      rowBottom,
      horizon,
      horizon
        ? {
            label: horizon.shortName,
            aux: err && age != null ? QB.dur(age) + " old" : QB.usdc(horizon.left)
          }
        : {}
    );

    setTone(chip, binding.tone);
    chip.classList.toggle("is-alarm", binding.locked || binding.usedPct >= 95);
    chip.title = tooltipFor(limits, burn, payload, err, age);
  }

  /* ── width awareness ─────────────────────────────────────────────────
     Below ~430px the sparkline steals room the meters need more. */
  function syncWidth() {
    chip.classList.toggle("is-narrow", chip.clientWidth < 430);
    drawSpark();
  }

  window.addEventListener("DOMContentLoaded", function () {
    var invoke = window.__TAURI__.core.invoke;
    var listen = window.__TAURI__.event.listen;

    requestAnimationFrame(tick);
    syncWidth();
    if (window.ResizeObserver) new ResizeObserver(syncWidth).observe(chip);
    window.addEventListener("resize", syncWidth);

    /* Keep the last payload so countdowns can be re-rendered between polls. */
    var latest = null;
    listen("quota-update", function (ev) {
      latest = ev.payload;
      apply(ev.payload);
    });
    setInterval(function () {
      if (latest) apply(latest);
    }, 30000);

    /* ── pointer: drag to move, click to refresh ───────────────────── */
    var dragging = false;
    var moved = false;
    var lastX = 0;
    var THRESH = 4;

    chip.addEventListener("pointerdown", function (e) {
      if (e.button !== 0) return;
      dragging = true;
      moved = false;
      lastX = e.clientX;
      chip.setPointerCapture(e.pointerId);
      invoke("begin_bar_drag").catch(function () {});
    });

    chip.addEventListener("pointermove", function (e) {
      if (!dragging) return;
      var dxCss = e.clientX - lastX;
      if (!moved && Math.abs(dxCss) < THRESH) return;
      moved = true;
      document.body.classList.add("dragging");
      var dx = Math.round(dxCss * (window.devicePixelRatio || 1));
      lastX = e.clientX;
      if (dx !== 0) invoke("nudge_bar", { dx: dx }).catch(function () {});
    });

    function endDrag(e) {
      if (!dragging) return;
      dragging = false;
      document.body.classList.remove("dragging");
      try {
        chip.releasePointerCapture(e.pointerId);
      } catch (_) {}
      invoke("end_bar_drag").catch(function () {});
      if (!moved) invoke("refresh_now").catch(function () {});
    }
    chip.addEventListener("pointerup", endDrag);
    chip.addEventListener("pointercancel", endDrag);

    chip.addEventListener(
      "wheel",
      function (e) {
        e.preventDefault();
        invoke("nudge_bar_width", { dw: e.deltaY > 0 ? -24 : 24 }).catch(function () {});
      },
      { passive: false }
    );

    chip.addEventListener("dblclick", function (e) {
      e.preventDefault();
      invoke("reset_bar_position").catch(function () {});
    });

    chip.addEventListener("contextmenu", function (e) {
      e.preventDefault();
      invoke("open_stats").catch(function () {});
    });

    invoke("current_quota")
      .then(function (q) {
        latest = q;
        apply(q);
      })
      .catch(function () {
        showNotice("Starting…", "none");
      });
  });
})();
