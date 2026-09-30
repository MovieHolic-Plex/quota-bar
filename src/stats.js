/* Quota Bar — stats window.
   Layout order: the binding limit in one strip, the chart at full size,
   then limits, burn rate, windows and lifetime totals as reference. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;

  var el = function (id) {
    return document.getElementById(id);
  };

  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  var state = { stats: null, burn: 0, limits: [], keys: null, keyId: null };
  var chart = { range: "48h", metric: "cost_usd", rows: [], scaleMax: 1, cfg: null };

  /* ── band helpers ────────────────────────────────────────────────────
     A band labelled "7d" only really covers 7 days if we have 7 days of
     snapshots. Everything downstream needs to know when it does not. */
  function latestTs(s) {
    return (s.latest && s.latest.fetched_at) || Math.floor(Date.now() / 1000);
  }

  function coverage(band, s) {
    var nominal = band.seconds || 0;
    var span = s.first_ts ? Math.max(0, latestTs(s) - s.first_ts) : 0;
    if (nominal <= 0) return { secs: span, partial: false };
    var secs = Math.min(nominal, span || nominal);
    return { secs: secs, partial: span > 0 && span < nominal * 0.98 };
  }

  function ratePerHour(band, s) {
    if (!band) return 0;
    var c = coverage(band, s);
    if (!c.secs) return 0;
    return band.cost_usd / (c.secs / 3600);
  }

  function findBand(s, label) {
    return (s.bands || []).filter(function (b) {
      return b.label === label;
    })[0];
  }

  /* ── binding limit strip ─────────────────────────────────────────────*/
  function renderTokens() {
    var L = (state.stats && state.stats.latest) || {};
    var mix = QB.tokenMix(L);
    if (!mix) {
      el("secTokens").hidden = true;
      return;
    }
    el("secTokens").hidden = false;
    var tiles = [
      { k: "Total", v: QB.tokens(mix.total), sub: "lifetime, input plus output" },
      { k: "Cached input", v: QB.tokens(mix.cachedInput), sub: QB.pct(mix.cacheShare) + " of all tokens" },
      { k: "Everything else", v: QB.tokens(mix.other), sub: "uncached input + output" },
      {
        k: "Per request",
        v: mix.perRequest == null ? "—" : QB.tokens(mix.perRequest),
        sub: mix.perRequest == null ? "no requests yet" : QB.int(L.request_count) + " requests"
      },
      {
        k: "List price / 1M tok",
        v: mix.usdPerMillion == null ? "—" : QB.usd(mix.usdPerMillion),
        sub: "lifetime cost ÷ lifetime tokens"
      }
    ];
    el("tokenTiles").innerHTML = tiles
      .map(function (t) {
        return (
          '<div class="tile"><div class="tile__k">' +
          esc(t.k) +
          '</div><div class="tile__v num">' +
          esc(t.v) +
          '</div><div class="tile__sub num">' +
          esc(t.sub) +
          "</div></div>"
        );
      })
      .join("");
    el("tokenMix").hidden = mix.total <= 0;
    el("tokenMixCached").style.width =
      mix.total > 0 ? Math.min(100, mix.cacheShare).toFixed(2) + "%" : "0%";
  }

  function renderBind() {
    var host = el("bind");
    var limits = state.limits;

    if (!limits.length) {
      /* A failed fetch and a proxy with no limits look identical from here,
         so say which one it actually is. */
      var err = state.stats && state.stats.latest && state.stats.latest.error;
      var failed = err && err !== "no samples yet";
      host.className = "bind " + (failed ? "t-crit" : "t-none");
      host.innerHTML =
        '<div class="bind__main"><span class="bind__name">' +
        (failed ? "Could not read limits" : "No limits reported") +
        "</span></div>" +
        '<div class="bind__note"><span class="bind__note-icon">' +
        (failed ? "⚠" : "·") +
        "</span>" +
        (failed
          ? "The last poll failed with <b>" +
            esc(err) +
            "</b>. Check the base URL and API key in Settings."
          : "<b>GET /v1/usage/self</b> returned no limits[]. The proxy may be an older build, or the base URL may be wrong.") +
        "</div>";
      return;
    }

    var l = QB.pickBinding(limits, state.burn);
    var fc = QB.forecast(l, state.burn);
    var pace = QB.paceNote(l);
    host.className = "bind t-" + l.tone;

    var paceMark =
      l.elapsedPct == null
        ? ""
        : '<div class="meter__pace" style="left:' +
          Math.min(100, Math.max(0, l.elapsedPct)).toFixed(2) +
          '%"></div>';

    host.innerHTML =
      '<div class="bind__main">' +
      '<span class="chip"><span class="chip__dot"></span>' +
      esc(l.status) +
      "</span>" +
      '<span class="bind__name">' +
      esc(l.name) +
      "</span>" +
      '<span class="bind__pct num">' +
      (l.locked ? "100%" : QB.pct(l.usedPct, 1)) +
      "</span>" +
      '<div class="meter"><div class="meter__fill" style="width:' +
      Math.min(100, Math.max(0, l.usedPct)).toFixed(2) +
      '%"></div>' +
      paceMark +
      "</div>" +
      '<span class="bind__fig num"><b>' +
      QB.usd(l.left) +
      "</b> left of " +
      QB.usd(l.max) +
      "</span>" +
      '<span class="bind__fig num">resets in <b data-reset-at="' +
      esc(l.resetAt || "") +
      '">' +
      (l.resetIn == null ? "—" : QB.dur(l.resetIn, "long")) +
      "</b></span>" +
      "</div>" +
      '<div class="bind__note"><span class="bind__note-icon">' +
      (fc && (fc.tone === "crit" || fc.tone === "hot") ? "⚠" : "✓") +
      "</span>" +
      "<b>" +
      esc(fc ? fc.text : "") +
      "</b>" +
      (state.burn > 0 ? " Burning <b>" + QB.usd(state.burn) + "/h</b> over the last hour." : "") +
      (pace ? " " + esc(pace.text) : "") +
      "</div>";
  }

  /* ── limit rows ──────────────────────────────────────────────────────*/
  function renderLimitRows() {
    var host = el("limitRows");
    var old = host.querySelectorAll(".lrow");
    for (var i = 0; i < old.length; i++) old[i].remove();

    if (!state.limits.length) {
      el("secLimits").hidden = true;
      return;
    }
    el("secLimits").hidden = false;

    host.insertAdjacentHTML(
      "beforeend",
      state.limits
        .map(function (l) {
          var paceMark =
            l.elapsedPct == null
              ? ""
              : '<div class="meter__pace" style="left:' +
                Math.min(100, Math.max(0, l.elapsedPct)).toFixed(2) +
                '%"></div>';
          var pd = l.paceDelta;
          var paceCls = pd == null ? "" : pd > 0 ? " over" : " under";
          return (
            '<div class="lrow t-' +
            l.tone +
            '">' +
            '<span class="lrow__name"><i class="lrow__dot"></i>' +
            esc(l.name) +
            "</span>" +
            '<span class="lrow__pct num">' +
            (l.locked ? "100%" : QB.pct(l.usedPct)) +
            "</span>" +
            '<div class="meter"><div class="meter__fill" style="width:' +
            Math.min(100, Math.max(0, l.usedPct)).toFixed(2) +
            '%"></div>' +
            paceMark +
            "</div>" +
            '<span class="lrow__cell num"><b>' +
            QB.usd(l.left) +
            "</b> / " +
            QB.usd(l.max) +
            "</span>" +
            '<span class="lrow__cell num" data-reset-at="' +
            esc(l.resetAt || "") +
            '">' +
            (l.resetIn == null ? "—" : QB.dur(l.resetIn, "long")) +
            "</span>" +
            '<span class="lrow__pace num' +
            paceCls +
            '">' +
            esc(QB.paceDeltaLabel(l)) +
            "</span>" +
            "</div>"
          );
        })
        .join("")
    );
  }

  /* ── burn rate ───────────────────────────────────────────────────────*/
  function renderBurn() {
    var s = state.stats;
    var tiles = [];

    [
      ["10m", "Last 10m"],
      ["1h", "Last 1h"],
      ["1d", "Last 24h"]
    ].forEach(function (d) {
      var b = findBand(s, d[0]);
      if (!b) return;
      var rate = ratePerHour(b, s);
      tiles.push({
        k: d[1],
        v: QB.usd(b.cost_usd),
        sub: rate > 0 ? QB.usd(rate) + " / hour" : "no spend"
      });
    });

    if (s.since_reset) {
      var cap = QB.dailyCapUsd(state.limits, s.daily_quota_usd);
      var used = s.since_reset.cost_usd || 0;
      var share = cap != null ? (used / cap) * 100 : null;
      tiles.push({
        k: "Since reset",
        v: QB.usd(used),
        sub:
          (s.last_reset_ts ? "from " + QB.clockTime(s.last_reset_ts) : "") +
          (share == null ? "" : " · " + QB.pct(share) + " of daily quota")
      });
    }

    if (!tiles.length) {
      el("secBurn").hidden = true;
      return;
    }
    el("secBurn").hidden = false;
    el("burnTiles").innerHTML = tiles
      .map(function (t) {
        return (
          '<div class="tile"><div class="tile__k">' +
          esc(t.k) +
          '</div><div class="tile__v num">' +
          esc(t.v) +
          '</div><div class="tile__sub num">' +
          esc(t.sub) +
          "</div></div>"
        );
      })
      .join("");
  }

  /* ── chart ───────────────────────────────────────────────────────────*/
  var RANGES = {
    "30m": {
      title: "Last 30m",
      rows: function (s) {
        return QB.densify(s.minutes, 60, 30);
      },
      every: 5,
      tick: function (ts) {
        var d = new Date(ts * 1000);
        return String(d.getHours()).padStart(2, "0") + ":" + String(d.getMinutes()).padStart(2, "0");
      },
      when: function (ts) {
        var f = function (x) {
          return String(x.getHours()).padStart(2, "0") + ":" + String(x.getMinutes()).padStart(2, "0");
        };
        return f(new Date(ts * 1000)) + "–" + f(new Date((ts + 60) * 1000));
      },
      avgLabel: "per minute"
    },
    "48h": {
      title: "Last 48h",
      rows: function (s) {
        return QB.densify(s.hourly, 3600, 48);
      },
      every: 6,
      tick: function (ts) {
        return String(new Date(ts * 1000).getHours()).padStart(2, "0");
      },
      when: function (ts) {
        var d = new Date(ts * 1000);
        return d.getMonth() + 1 + "/" + d.getDate() + " " + String(d.getHours()).padStart(2, "0") + ":00";
      },
      avgLabel: "per hour"
    },
    "30d": {
      title: "Last 30d",
      rows: function (s) {
        return QB.densify(s.daily, 86400, 30);
      },
      every: 5,
      tick: function (ts) {
        var d = new Date(ts * 1000);
        return d.getMonth() + 1 + "/" + d.getDate();
      },
      when: function (ts) {
        var d = new Date(ts * 1000);
        return d.getMonth() + 1 + "/" + d.getDate();
      },
      avgLabel: "per day"
    }
  };

  var METRICS = {
    cost_usd: { fmt: QB.usd, axis: QB.usd },
    tokens: { fmt: QB.tokens, axis: QB.tokens },
    requests: { fmt: QB.int, axis: QB.tokens }
  };

  /* Round the top of the axis up to something a person can divide by eye. */
  function niceMax(v) {
    if (!(v > 0)) return 1;
    var exp = Math.pow(10, Math.floor(Math.log10(v)));
    var steps = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];
    for (var i = 0; i < steps.length; i++) {
      if (steps[i] * exp >= v) return steps[i] * exp;
    }
    return 10 * exp;
  }

  function renderChart() {
    var s = state.stats;
    var cfg = RANGES[chart.range];
    var m = METRICS[chart.metric];
    var rows = cfg.rows(s);
    chart.rows = rows;
    chart.cfg = cfg;

    var vals = rows.map(function (r) {
      return r[chart.metric] || 0;
    });
    var peak = Math.max.apply(null, vals.concat([0]));
    var scaleMax = niceMax(peak);
    chart.scaleMax = scaleMax;

    el("chartGrid").innerHTML = [0, 0.25, 0.5, 0.75, 1]
      .map(function (f) {
        return '<i style="bottom:' + (f * 100).toFixed(2) + '%"></i>';
      })
      .join("");

    el("chartY").innerHTML = [1, 0.75, 0.5, 0.25, 0]
      .map(function (f) {
        return '<i style="bottom:' + (f * 100).toFixed(2) + '%">' + esc(m.axis(scaleMax * f)) + "</i>";
      })
      .join("");

    el("chartBars").innerHTML = rows
      .map(function (r, i) {
        var v = vals[i];
        if (v <= 0) return '<div class="chart__bar is-zero"></div>';
        return (
          '<div class="chart__bar" style="height:' +
          Math.max(2, (v / scaleMax) * 100).toFixed(2) +
          '%"></div>'
        );
      })
      .join("");

    /* mean line, so one spike does not read as "normal" */
    var mean = vals.length
      ? vals.reduce(function (a, b) {
          return a + b;
        }, 0) / vals.length
      : 0;
    var avgEl = el("chartAvg");
    if (mean > 0) {
      avgEl.hidden = false;
      avgEl.style.bottom = Math.min(100, (mean / scaleMax) * 100).toFixed(2) + "%";
      avgEl.title = "mean " + m.fmt(mean);
    } else {
      avgEl.hidden = true;
    }

    el("chartX").innerHTML = rows
      .map(function (r, i) {
        if (i % cfg.every !== 0 && i !== rows.length - 1) return "";
        return (
          '<i style="left:' +
          (((i + 0.5) / rows.length) * 100).toFixed(2) +
          '%">' +
          esc(cfg.tick(r.start_ts)) +
          "</i>"
        );
      })
      .join("");

    setReadout(null);
  }

  function setReadout(idx) {
    var host = el("chartReadout");
    var cfg = chart.cfg;
    var m = METRICS[chart.metric];
    var rows = chart.rows;
    if (!cfg || !rows.length) {
      host.innerHTML = "";
      return;
    }

    if (idx == null) {
      var total = QB.sum(rows, chart.metric);
      var peak = rows.reduce(function (a, r) {
        return Math.max(a, r[chart.metric] || 0);
      }, 0);
      host.innerHTML =
        '<span class="when">' +
        esc(cfg.title) +
        "</span>" +
        '<span>total <span class="big num">' +
        esc(m.fmt(total)) +
        "</span></span>" +
        "<span>" +
        esc(cfg.avgLabel) +
        ' <b class="num">' +
        esc(m.fmt(total / rows.length)) +
        "</b></span>" +
        '<span>peak <b class="num">' +
        esc(m.fmt(peak)) +
        "</b></span>";
      return;
    }

    var r = rows[idx];
    host.innerHTML =
      '<span class="when">' +
      esc(cfg.when(r.start_ts)) +
      "</span>" +
      '<span class="big num">' +
      esc(QB.usd(r.cost_usd || 0)) +
      "</span>" +
      '<span class="num"><b>' +
      esc(QB.tokens(r.tokens || 0)) +
      "</b> tokens</span>" +
      '<span class="num"><b>' +
      esc(QB.int(r.requests || 0)) +
      "</b> requests</span>" +
      '<span class="num">cache <b>' +
      esc(QB.tokens(r.cached || 0)) +
      "</b></span>";
  }

  function wireChart() {
    var plot = el("chartPlot");
    var cursor = el("chartCursor");

    plot.addEventListener("mousemove", function (e) {
      var rows = chart.rows;
      if (!rows.length) return;
      var rect = plot.getBoundingClientRect();
      var f = (e.clientX - rect.left) / Math.max(1, rect.width);
      var idx = Math.min(rows.length - 1, Math.max(0, Math.floor(f * rows.length)));
      plot.classList.add("is-hover");
      cursor.style.display = "block";
      cursor.style.left = (((idx + 0.5) / rows.length) * 100).toFixed(3) + "%";
      var bars = plot.querySelectorAll(".chart__bar");
      for (var i = 0; i < bars.length; i++) bars[i].classList.toggle("is-on", i === idx);
      setReadout(idx);
    });

    plot.addEventListener("mouseleave", function () {
      plot.classList.remove("is-hover");
      cursor.style.display = "none";
      var bars = plot.querySelectorAll(".chart__bar.is-on");
      for (var i = 0; i < bars.length; i++) bars[i].classList.remove("is-on");
      setReadout(null);
    });

    function tabs(id, attr, onPick) {
      el(id).addEventListener("click", function (e) {
        var btn = e.target.closest("button");
        if (!btn) return;
        var group = el(id).querySelectorAll("button");
        for (var i = 0; i < group.length; i++) group[i].classList.remove("is-on");
        btn.classList.add("is-on");
        onPick(btn.getAttribute(attr));
      });
    }
    tabs("rangeTabs", "data-range", function (v) {
      chart.range = v;
      if (state.stats) renderChart();
    });
    tabs("metricTabs", "data-metric", function (v) {
      chart.metric = v;
      if (state.stats) renderChart();
    });
  }

  /* ── windows table ───────────────────────────────────────────────────*/
  var BAND_LABELS = [
    ["10m", "Last 10m"],
    ["1h", "Last 1h"],
    ["5h", "Last 5h"],
    ["1d", "Last 24h"],
    ["3d", "Last 3d"],
    ["7d", "Last 7d"],
    ["30d", "Last 30d"],
    ["all", "All time"]
  ];

  function bandRow(name, band, s, anchor) {
    var cov = coverage(band, s);
    var rate = cov.secs > 0 ? band.cost_usd / (cov.secs / 3600) : 0;
    var cacheRate = band.tokens > 0 ? (band.cached / band.tokens) * 100 : null;
    return (
      '<tr class="' +
      (anchor ? "is-anchor" : "") +
      '"><td class="name">' +
      esc(name) +
      (cov.partial ? '<span class="tagmark">partial</span>' : "") +
      "</td><td>" +
      esc(QB.usd(band.cost_usd)) +
      "</td><td>" +
      esc(rate > 0 ? QB.usd(rate) : "—") +
      "</td><td>" +
      esc(QB.int(band.requests)) +
      "</td><td>" +
      esc(QB.tokens(band.tokens)) +
      "</td><td>" +
      esc(cacheRate == null ? "—" : QB.pct(cacheRate)) +
      "</td></tr>"
    );
  }

  function renderBands() {
    var s = state.stats;
    var out = [];
    if (s.since_reset) {
      var t = s.last_reset_ts ? "Since reset " + QB.clockTime(s.last_reset_ts) : "Since reset";
      out.push(bandRow(t, s.since_reset, s, true));
    }
    BAND_LABELS.forEach(function (d) {
      var b = findBand(s, d[0]);
      if (b) out.push(bandRow(d[1], b, s, false));
    });
    el("bands").innerHTML = out.join("");
    el("secBands").hidden = !out.length;
  }

  /* ── lifetime ────────────────────────────────────────────────────────*/
  function renderTotals() {
    var L = state.stats.latest || {};
    var saved = L.savings_usd;
    var tiles = [
      { k: "List price", v: QB.usd(L.total_cost_usd), sub: "same usage billed at API rates" },
      { k: "Paid", v: QB.usd(L.pro_usd), sub: "subscription, editable in Settings" },
      {
        k: "Savings",
        v: QB.usd(saved),
        sub: saved >= 0 ? "list price minus subscription" : "subscription not yet earned back",
        cls: saved >= 0 ? "tile--good" : "tile--bad"
      },
      { k: "Requests", v: QB.int(L.request_count), sub: "counted by the proxy" },
      { k: "Tokens", v: QB.tokens(L.total_tokens), sub: "input plus output" },
      { k: "Cached input / all tokens", v: QB.pct(L.cache_pct), sub: "not an input-only cache hit rate" }
    ];
    el("totalTiles").innerHTML = tiles
      .map(function (t) {
        return (
          '<div class="tile ' +
          (t.cls || "") +
          '"><div class="tile__k">' +
          esc(t.k) +
          '</div><div class="tile__v num">' +
          esc(t.v) +
          '</div><div class="tile__sub">' +
          esc(t.sub) +
          "</div></div>"
        );
      })
      .join("");
  }

  /* Reset countdowns drift between the 15s reloads; keep them honest. */
  function tickCountdowns() {
    var nodes = document.querySelectorAll("[data-reset-at]");
    for (var i = 0; i < nodes.length; i++) {
      var iso = nodes[i].getAttribute("data-reset-at");
      if (!iso) continue;
      var secs = QB.secsUntil(iso);
      if (secs == null) continue;
      nodes[i].textContent = QB.dur(secs, "long");
    }
  }

  /* ── keys ────────────────────────────────────────────────────────────
     Tabs pick whose history the page draws; the table shows all of them. */
  function keyCell(limit) {
    if (!limit) return '<span class="kcell__none">—</span>';
    var l = QB.normalizeLimit(limit);
    return (
      '<div class="kcell t-' +
      l.tone +
      '"><span class="kcell__pct num">' +
      esc(l.locked ? "FULL" : QB.pct(l.usedPct)) +
      '</span><div class="meter"><div class="meter__fill" style="width:' +
      Math.min(100, Math.max(0, l.usedPct)).toFixed(1) +
      '%"></div></div></div>'
    );
  }

  function healthChip(k) {
    if (!k.enabled) return '<span class="chip t-none">Disabled</span>';
    if (k.health === "exhausted") return '<span class="chip t-crit">' + esc(k.health_reason || "Exhausted") + "</span>";
    if (k.health === "ok") return '<span class="chip t-ok">Healthy</span>';
    return '<span class="chip t-warn" title="' + esc(k.health_reason || "") + '">Unknown</span>';
  }

  function renderKeys() {
    var v = state.keys;
    var list = (v && v.keys) || [];
    var shown = state.stats && state.stats.key_id;
    el("keyTabs").hidden = list.length < 2;
    el("secKeys").hidden = list.length < 2;
    if (list.length < 2) return;

    el("keyTabs").innerHTML = list
      .map(function (k) {
        return (
          '<button data-key="' +
          esc(k.id) +
          '"' +
          (k.id === shown ? ' class="is-on"' : "") +
          ">" +
          esc(k.label) +
          (k.active ? " ●" : "") +
          "</button>"
        );
      })
      .join("");

    el("keyRows").innerHTML =
      '<div class="krow is-head"><span>#</span><span>Key</span><span>3h</span><span>daily</span><span>weekly</span><span>Status</span><span>Claude Code</span></div>' +
      list
        .map(function (k, i) {
          return (
            '<div class="krow' +
            (k.active ? " is-live" : "") +
            (k.id === shown ? " is-shown" : "") +
            '" data-key="' +
            esc(k.id) +
            '"><span class="krow__rank num">' +
            (i + 1) +
            '</span><span class="krow__name"><b>' +
            esc(k.label) +
            (k.active ? " · live" : "") +
            "</b><span>" +
            esc(k.preview || "") +
            "</span></span>" +
            keyCell(k.three_h) +
            keyCell(k.daily) +
            keyCell(k.weekly) +
            "<span>" +
            healthChip(k) +
            '</span><span class="krow__used">' +
            esc((k.used_in || []).join(", ") || "—") +
            "</span></div>"
          );
        })
        .join("");
  }

  function pickKey(e) {
    var t = e.target.closest("[data-key]");
    if (!t) return;
    state.keyId = t.getAttribute("data-key");
    load();
  }
  el("keyTabs").addEventListener("click", pickKey);
  el("keyRows").addEventListener("click", pickKey);

  el("openSettings").addEventListener("click", function () {
    invoke("open_settings").catch(function () {});
  });

  /* ── load ────────────────────────────────────────────────────────────*/
  var loading = false;

  async function load() {
    if (loading) return;
    loading = true;
    lastLoad = performance.now();
    try {
      var got = await Promise.all([
        invoke("get_stats", { keyId: state.keyId }),
        invoke("list_keys").catch(function () {
          return null;
        })
      ]);
      var s = got[0];
      state.stats = s;
      state.keys = got[1];
      renderKeys();
      state.limits = QB.collectLimits(s.latest || {});

      state.burn = ratePerHour(findBand(s, "1h"), s) || ratePerHour(findBand(s, "10m"), s) || 0;

      var err = s.latest && s.latest.error;
      var showErr = err && err !== "no samples yet";
      var noData = !s.snapshot_count;
      var stale = !!(s.latest && s.latest.stale);
      el("banner").hidden = !showErr;
      el("banner").className = "banner" + (stale ? " banner--warn" : "");
      if (showErr) {
        el("bannerText").textContent = stale
          ? "Showing the last successful read from " +
            QB.clockTime(s.latest.fetched_at) +
            ". The latest poll failed: " +
            err
          : err;
      }

      renderBind();
      renderLimitRows();

      el("empty").hidden = !noData;
      ["secTokens", "secBurn", "secChart", "secBands", "secTotal"].forEach(function (id) {
        el(id).hidden = noData;
      });

      if (!noData) {
        renderTokens();
        renderBurn();
        renderChart();
        renderBands();
        renderTotals();
      }

      var fetched = s.latest && s.latest.fetched_at;
      el("stamp").textContent = fetched ? "Updated " + QB.clockTime(fetched) : "";

      var first = s.first_ts ? QB.dateTime(s.first_ts) : "—";
      el("foot").innerHTML =
        "<span>Recording since " +
        esc(first) +
        "</span><span>" +
        esc(QB.int(s.snapshot_count || 0)) +
        " snapshots</span>" +
        (s.next_reset_ts
          ? "<span>Next daily reset " + esc(QB.dateTime(s.next_reset_ts)) + "</span>"
          : "<span>No daily reset time set — add one in Settings to anchor the daily figures</span>");
    } catch (e) {
      el("banner").hidden = false;
      el("bannerText").textContent = String(e);
    } finally {
      loading = false;
    }
  }

  el("refresh").addEventListener("click", async function () {
    var btn = el("refresh");
    btn.classList.add("is-busy");
    btn.textContent = "Fetching…";
    try {
      await invoke("refresh_now");
    } catch (_) {}
    setTimeout(async function () {
      await load();
      btn.classList.remove("is-busy");
      btn.textContent = "Refresh";
    }, 1400);
  });

  /* ── visibility ──────────────────────────────────────────────────────
     This window is only ever hidden, never closed, so a bare interval
     keeps running a dozen SQL queries every 15 seconds for the rest of the
     session with nobody looking at the result.

     document.hidden alone is not enough to trust across webview versions,
     so rAF doubles as the probe: it does not fire for a hidden window, and
     an empty callback costs nothing on one that is already compositing. */
  var lastBeat = performance.now();
  (function beat() {
    lastBeat = performance.now();
    requestAnimationFrame(beat);
  })();

  function onScreen() {
    return !document.hidden && performance.now() - lastBeat < 2000;
  }

  var lastLoad = 0;
  var wasOnScreen = true;

  wireChart();
  load();

  setInterval(function () {
    var now = performance.now();
    var here = onScreen();
    if (!here) {
      wasOnScreen = false;
      return;
    }
    /* Reload on the way back in, then settle into the 15s cadence. The edge
       is detected here rather than off visibilitychange so it still works
       if the webview never fires that event. */
    if (!wasOnScreen || now - lastLoad >= 15000) load();
    wasOnScreen = true;
    tickCountdowns();
  }, 1000);
})();
