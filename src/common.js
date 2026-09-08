/* Quota Bar — shared formatting and quota maths.
   Classic script (no bundler); everything hangs off window.QB.

   Wording follows the proxy's own vocabulary: limit windows keep the API's
   spelling ("3h", "daily", "weekly") and the derived concepts keep their
   usual names (binding limit, burn rate, pace marker). */
(function (global) {
  "use strict";

  /* Length of each limit window, so we can work out how far through it we are. */
  var WINDOW_SECS = {
    "1h": 3600,
    "3h": 10800,
    "5h": 18000,
    "12h": 43200,
    daily: 86400,
    weekly: 604800,
    monthly: 2592000
  };

  /* Shortest window first: that is the order in which a limit tends to bite. */
  var WINDOW_ORDER = ["1h", "3h", "5h", "12h", "daily", "weekly", "monthly"];

  /* Abbreviations for the taskbar strip, where a label gets ~30px. */
  var WINDOW_SHORT = {
    "1h": "1h",
    "3h": "3h",
    "5h": "5h",
    "12h": "12h",
    daily: "day",
    weekly: "week",
    monthly: "mo"
  };

  function isNum(v) {
    return typeof v === "number" && isFinite(v);
  }

  /* ── formatting ─────────────────────────────────────────────────────── */

  function usd(n) {
    if (!isNum(n)) return "--";
    var sign = n < 0 ? "-" : "";
    var v = Math.abs(n);
    if (v === 0) return "$0";
    if (v >= 1000) return sign + "$" + v.toLocaleString("en-US", { maximumFractionDigits: 0 });
    if (v >= 0.01) return sign + "$" + v.toFixed(2);
    return sign + "$" + v.toFixed(4);
  }

  /* Tight form for the taskbar, where every pixel is contested. */
  function usdc(n) {
    if (!isNum(n)) return "--";
    var sign = n < 0 ? "-" : "";
    var v = Math.abs(n);
    if (v === 0) return "$0";
    if (v >= 100000) return sign + "$" + Math.round(v / 1000) + "k";
    if (v >= 1000) return sign + "$" + (v / 1000).toFixed(1) + "k";
    if (v >= 100) return sign + "$" + Math.round(v);
    if (v >= 1) return sign + "$" + v.toFixed(1);
    if (v >= 0.01) return sign + "$" + v.toFixed(2);
    return sign + "<$0.01";
  }

  function pct(n, digits) {
    if (!isNum(n)) return "--";
    var v = Math.max(0, n);
    if (digits != null) return v.toFixed(digits) + "%";
    if (v >= 10) return Math.round(v) + "%";
    return v.toFixed(1) + "%";
  }

  function tokens(n) {
    if (!isNum(n)) return "--";
    var v = Math.abs(n);
    if (v >= 1e9) return (n / 1e9).toFixed(2) + "B";
    if (v >= 1e6) return (n / 1e6).toFixed(1) + "M";
    if (v >= 1e3) return (n / 1e3).toFixed(1) + "K";
    return String(Math.round(n));
  }

  function int(n) {
    if (!isNum(n)) return "--";
    return Math.round(n).toLocaleString("en-US");
  }

  /* "2h11m" packs into the bar; "2h 11m" reads better everywhere else. */
  function dur(secs, style) {
    if (!isNum(secs)) return "";
    var s = Math.max(0, Math.round(secs));
    var d = Math.floor(s / 86400);
    var h = Math.floor((s % 86400) / 3600);
    var m = Math.floor((s % 3600) / 60);
    if (style === "long") {
      if (d > 0) return h > 0 ? d + "d " + h + "h" : d + "d";
      if (h > 0) return m > 0 ? h + "h " + m + "m" : h + "h";
      if (m > 0) return m + "m";
      return "under a minute";
    }
    if (d > 0) return d + "d" + h + "h";
    if (h > 0) return h + "h" + String(m).padStart(2, "0") + "m";
    return m + "m";
  }

  function two(n) {
    return String(n).padStart(2, "0");
  }

  /* 24h, fixed format. The OS locale would otherwise render these in a
     different language from the rest of the window. */
  function clockTime(ts) {
    if (!isNum(ts)) return "";
    var d = new Date(ts * 1000);
    return two(d.getHours()) + ":" + two(d.getMinutes());
  }

  /* "9/2 21:46" — same shape the chart uses for hourly buckets. */
  function dateTime(ts) {
    if (!isNum(ts)) return "";
    var d = new Date(ts * 1000);
    return d.getMonth() + 1 + "/" + d.getDate() + " " + two(d.getHours()) + ":" + two(d.getMinutes());
  }

  /* ── limits ─────────────────────────────────────────────────────────── */

  function secsUntil(iso) {
    if (!iso) return null;
    var raw = /Z|[+-]\d{2}:?\d{2}$/.test(iso) ? iso : iso + "Z";
    var t = Date.parse(raw);
    if (isNaN(t)) return null;
    return Math.max(0, Math.round((t - Date.now()) / 1000));
  }

  function isFable(mf) {
    return String(mf || "").toLowerCase().indexOf("fable") !== -1;
  }

  /* 0–59 OK · 60–84 Warn · 85–99 High · 100+ Full.
     Colour and word always move together, so the scale stays readable
     without relying on hue alone. */
  function toneOf(usedPct, remaining) {
    if (!isNum(usedPct)) return { key: "none", word: "—" };
    if (usedPct >= 100 || (isNum(remaining) && remaining <= 0)) return { key: "crit", word: "Full" };
    if (usedPct >= 85) return { key: "hot", word: "High" };
    if (usedPct >= 60) return { key: "warn", word: "Warn" };
    return { key: "ok", word: "OK" };
  }

  /* Accepts both raw limits[] rows (micro-dollars) and the pre-made
     LimitView shape the backend also emits. Returns one flat object. */
  function normalizeLimit(raw) {
    if (!raw) return null;
    var micro = raw.current_usd == null && raw.remaining_usd == null;
    var window = String(raw.limit_window || raw.window || "").toLowerCase();
    var modelFilter = raw.model_filter || null;
    var used = micro ? (raw.current_value || 0) / 1e6 : raw.current_usd || 0;
    var max = micro ? (raw.max_value || 0) / 1e6 : raw.max_usd || 0;
    var left = micro ? (raw.remaining_value || 0) / 1e6 : raw.remaining_usd || 0;
    var usedPct = isNum(raw.used_percent) ? raw.used_percent : max > 0 ? (used / max) * 100 : 0;
    var fable = isFable(modelFilter);
    var windowSecs = WINDOW_SECS[window] || null;
    var resetIn = secsUntil(raw.reset_at);

    /* How far through the window we are. A limit at 40% with 80% of the
       window elapsed is fine; at 40% with 10% elapsed is not. */
    var elapsedPct = null;
    if (windowSecs && resetIn != null) {
      elapsedPct = Math.min(100, Math.max(0, ((windowSecs - resetIn) / windowSecs) * 100));
    }

    var tone = toneOf(usedPct, left);
    return {
      key: (fable ? "fable:" : "all:") + window,
      window: window,
      fable: fable,
      modelFilter: modelFilter,
      name: (fable ? "Fable" : "All models") + " · " + window,
      shortName: (fable ? "F/" : "") + (WINDOW_SHORT[window] || window),
      used: used,
      max: max,
      left: left,
      usedPct: usedPct,
      windowSecs: windowSecs,
      resetIn: resetIn,
      resetAt: raw.reset_at || null,
      elapsedPct: elapsedPct,
      paceDelta: elapsedPct == null ? null : usedPct - elapsedPct,
      locked: usedPct >= 100 || left <= 0,
      tone: tone.key,
      status: tone.word
    };
  }

  /* Prefer the raw limits[] array: it renders whatever the proxy sends,
     including windows this build has never heard of. Fall back to the five
     named views for older payloads. */
  function collectLimits(payload) {
    if (!payload) return [];
    var out = [];
    var seen = {};
    var push = function (raw) {
      var l = normalizeLimit(raw);
      if (!l || seen[l.key]) return;
      seen[l.key] = true;
      out.push(l);
    };
    if (Array.isArray(payload.limits)) payload.limits.forEach(push);
    ["three_h", "daily", "weekly", "fable_daily", "fable_weekly"].forEach(function (k) {
      if (payload[k]) push(payload[k]);
    });
    out.sort(function (a, b) {
      if (a.fable !== b.fable) return a.fable ? 1 : -1;
      var ai = WINDOW_ORDER.indexOf(a.window);
      var bi = WINDOW_ORDER.indexOf(b.window);
      return (ai < 0 ? 99 : ai) - (bi < 0 ? 99 : bi);
    });
    return out;
  }

  /* Hours of headroom left at the current burn rate. */
  function hoursToEmpty(limit, burnPerHour) {
    if (!limit || !isNum(burnPerHour) || burnPerHour <= 0) return null;
    if (limit.left <= 0) return 0;
    return limit.left / burnPerHour;
  }

  /* The binding limit: whichever runs dry soonest at the current burn rate,
     falling back to raw percentage when there is no rate to go on. */
  function pickBinding(limits, burnPerHour) {
    if (!limits || !limits.length) return null;
    var scored = limits.map(function (l) {
      var h = hoursToEmpty(l, burnPerHour);
      var untilReset = l.resetIn == null ? Infinity : l.resetIn / 3600;
      /* Only counts as a threat if it empties before it resets. */
      var threat = h != null && h < untilReset ? h : Infinity;
      return { limit: l, threat: threat };
    });
    scored.sort(function (a, b) {
      if (a.limit.locked !== b.limit.locked) return a.limit.locked ? -1 : 1;
      if (a.threat !== b.threat) return a.threat - b.threat;
      return b.limit.usedPct - a.limit.usedPct;
    });
    return scored[0].limit;
  }

  /* One plain sentence about where a limit is heading. */
  function forecast(limit, burnPerHour) {
    if (!limit) return null;
    if (limit.locked) {
      return {
        tone: "crit",
        text:
          limit.resetIn != null
            ? "Blocked until it resets in " + dur(limit.resetIn, "long") + "."
            : "Limit fully used."
      };
    }
    var h = hoursToEmpty(limit, burnPerHour);
    if (h == null) {
      return { tone: "none", text: "No recent spend, so there is nothing to project." };
    }
    var emptySecs = h * 3600;
    if (limit.resetIn != null && emptySecs >= limit.resetIn) {
      return { tone: "ok", text: "Holds until reset at the current rate." };
    }
    var gap = limit.resetIn == null ? null : limit.resetIn - emptySecs;
    var tail = gap == null ? "" : ", " + dur(gap, "long") + " before reset";
    return {
      tone: emptySecs < 3600 ? "crit" : "hot",
      text: "Empties in " + dur(emptySecs, "long") + " at the current rate" + tail + "."
    };
  }

  /* Where usage sits against the pace marker. */
  function paceNote(limit) {
    if (!limit || limit.paceDelta == null) return null;
    var d = limit.paceDelta;
    if (d > 12) return { tone: "hot", text: Math.round(d) + "%p ahead of an even pace." };
    if (d < -12) return { tone: "ok", text: Math.round(-d) + "%p behind an even pace." };
    return { tone: "ok", text: "Tracking an even pace." };
  }

  /* Compact form of the same idea, for dense rows. */
  function paceDeltaLabel(limit) {
    if (!limit || limit.paceDelta == null) return "—";
    var d = Math.round(limit.paceDelta);
    return d > 0 ? "+" + d + "%p" : d + "%p";
  }

  /* ── series ─────────────────────────────────────────────────────────── */

  /* The backend only emits buckets that had a snapshot in them. Pad the gaps
     with zeroes so the time axis does not lie about how long ago things were. */
  function densify(rows, bucketSecs, count) {
    var by = {};
    (rows || []).forEach(function (r) {
      by[r.start_ts] = r;
    });
    var last = rows && rows.length ? rows[rows.length - 1].start_ts : null;
    var nowBucket = Math.floor(Date.now() / 1000 / bucketSecs) * bucketSecs;
    var end = Math.max(nowBucket, last == null ? nowBucket : last);
    var start = end - (count - 1) * bucketSecs;
    var out = [];
    for (var t = start; t <= end; t += bucketSecs) {
      out.push(by[t] || { start_ts: t, tokens: 0, cached: 0, cost_usd: 0, requests: 0 });
    }
    return out;
  }

  function sum(rows, key) {
    return (rows || []).reduce(function (a, r) {
      return a + (r[key] || 0);
    }, 0);
  }

  global.QB = {
    WINDOW_SECS: WINDOW_SECS,
    usd: usd,
    usdc: usdc,
    pct: pct,
    tokens: tokens,
    int: int,
    dur: dur,
    clockTime: clockTime,
    dateTime: dateTime,
    secsUntil: secsUntil,
    isFable: isFable,
    toneOf: toneOf,
    normalizeLimit: normalizeLimit,
    collectLimits: collectLimits,
    hoursToEmpty: hoursToEmpty,
    pickBinding: pickBinding,
    forecast: forecast,
    paceNote: paceNote,
    paceDeltaLabel: paceDeltaLabel,
    densify: densify,
    sum: sum
  };
})(window);
