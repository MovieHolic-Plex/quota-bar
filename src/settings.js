/* Quota Bar — settings window.
   Keys and Claude Code act on click; General is the only form with Save. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;
  var listen = window.__TAURI__.event.listen;
  var getCurrentWindow = window.__TAURI__.window.getCurrentWindow;
  var $ = function (id) {
    return document.getElementById(id);
  };

  var MODEL_LABELS = {
    ANTHROPIC_DEFAULT_OPUS_MODEL: "opus →",
    ANTHROPIC_DEFAULT_SONNET_MODEL: "sonnet →",
    ANTHROPIC_DEFAULT_FABLE_MODEL: "fable →",
    ANTHROPIC_DEFAULT_HAIKU_MODEL: "haiku →"
  };

  var state = {
    pane: "keys",
    keys: null,
    targets: null,
    revealed: {},
    editing: null,
    armed: null
  };

  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  var sayTimer = 0;
  function say(text, kind) {
    var m = $("msg");
    m.textContent = text;
    m.className = "msg" + (kind ? " " + kind : "");
    clearTimeout(sayTimer);
    if (kind === "ok") {
      sayTimer = setTimeout(function () {
        m.textContent = "";
      }, 6000);
    }
  }

  function errText(e) {
    return String(e && e.message ? e.message : e);
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (_) {
      var ta = document.createElement("textarea");
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
  }

  /* ── panes ──────────────────────────────────────────────────────────*/
  function showPane(name) {
    state.pane = name;
    document.querySelectorAll(".nav").forEach(function (b) {
      b.classList.toggle("is-on", b.getAttribute("data-pane") === name);
    });
    ["keys", "claude", "general"].forEach(function (p) {
      $("pane-" + p).hidden = p !== name;
    });
    $("save").hidden = name !== "general";
    say("");
    if (name === "claude" && !state.targets) loadTargets();
  }

  document.querySelectorAll(".nav").forEach(function (b) {
    b.addEventListener("click", function () {
      showPane(b.getAttribute("data-pane"));
    });
  });

  /* ── keys ───────────────────────────────────────────────────────────*/
  function keyById(id) {
    var list = (state.keys && state.keys.keys) || [];
    for (var i = 0; i < list.length; i++) if (list[i].id === id) return list[i];
    return null;
  }

  function statusChip(k) {
    if (!k.enabled) return '<span class="chip t-none"><span class="chip__dot"></span>Disabled</span>';
    if (k.health === "exhausted")
      return (
        '<span class="chip t-crit"><span class="chip__dot"></span>Exhausted · ' +
        esc(k.health_reason || "") +
        "</span>"
      );
    if (k.health === "ok") {
      var tone = QB.toneOf(k.peak_pct || 0).key;
      return (
        '<span class="chip t-' +
        tone +
        '"><span class="chip__dot"></span>Healthy · peak ' +
        esc(QB.pct(k.peak_pct || 0)) +
        "</span>"
      );
    }
    var st = k.status;
    var why = st && st.error ? st.error : "Not read yet";
    return (
      '<span class="chip t-warn" title="' +
      esc(why) +
      '"><span class="chip__dot"></span>' +
      esc(why.length > 42 ? why.slice(0, 40) + "…" : why) +
      "</span>"
    );
  }

  function limitsHtml(k) {
    var limits = QB.collectLimits(k);
    if (!limits.length) {
      return k.status && k.status.fetched_at
        ? '<span class="help">The proxy reported no limits for this key.</span>'
        : "";
    }
    return (
      '<div class="lims">' +
      limits
        .map(function (l) {
          var sub = QB.usdc(l.left) + " left";
          if (l.resetIn != null) sub += " · " + QB.dur(l.resetIn);
          return (
            '<div class="lim t-' +
            l.tone +
            '"><div class="lim__top"><span class="lim__name">' +
            esc((l.fable ? "Fable " : "") + l.window) +
            '</span><span class="lim__pct num">' +
            esc(l.locked ? "FULL" : QB.pct(l.usedPct)) +
            '</span></div><div class="meter"><div class="meter__fill" style="width:' +
            Math.min(100, Math.max(0, l.usedPct)).toFixed(1) +
            '%"></div></div><span class="lim__sub num">' +
            esc(sub) +
            "</span></div>"
          );
        })
        .join("") +
      "</div>"
    );
  }

  function keyCard(k, idx, total, auto) {
    var revealed = state.revealed[k.id];
    var used = (k.used_in || [])
      .map(function (t) {
        return '<span class="chip t-ok" title="Claude Code on ' + esc(t) + ' uses this key">' + esc(t) + "</span>";
      })
      .join("");
    var live = k.active
      ? '<span class="chip" style="--tone:var(--brand);--tone-soft:var(--brand-soft)"><span class="chip__dot"></span>LIVE</span>'
      : "";
    var useBtn = k.active || !k.enabled
      ? ""
      : '<button data-act="use">' + (auto ? "Make primary" : "Use") + "</button>";
    var editing = state.editing === k.id;
    var edit = !editing
      ? ""
      : '<div class="key__edit">' +
        '<div class="grid3">' +
        '<label><span class="lab">Label</span><input data-f="label" value="' +
        esc(k.label) +
        '" /></label>' +
        '<label><span class="lab">Replace key</span><input data-f="secret" class="mono" type="password" autocomplete="off" spellcheck="false" placeholder="leave blank to keep" /></label>' +
        '<label><span class="lab">Base URL</span><input data-f="base" spellcheck="false" placeholder="global URL" value="' +
        esc(k.base_url || "") +
        '" /></label>' +
        "</div>" +
        '<div class="row"><span class="grow"></span><button data-act="edit-cancel">Cancel</button><button class="primary" data-act="edit-save">Save</button></div>' +
        "</div>";

    return (
      '<article class="key' +
      (k.active ? " is-live" : "") +
      (k.enabled ? "" : " is-off") +
      '" data-id="' +
      esc(k.id) +
      '">' +
      '<div class="key__head"><span class="key__rank num">' +
      (idx + 1) +
      '</span><span class="key__label">' +
      esc(k.label) +
      "</span>" +
      live +
      statusChip(k) +
      '<span class="spacer"></span><span class="used">' +
      used +
      "</span></div>" +
      '<div class="key__secret' +
      (revealed ? " is-revealed" : "") +
      '"><span class="mono">' +
      esc(revealed || k.preview || "(no secret stored)") +
      '</span><button class="ghost icon" data-act="reveal" title="Show or hide the full key">' +
      (revealed ? "Hide" : "Show") +
      '</button><button class="ghost icon" data-act="copy" title="Copy the full key">Copy</button></div>' +
      '<div class="key__url">' +
      esc(k.effective_base_url) +
      (k.base_url ? "" : " · global") +
      "</div>" +
      limitsHtml(k) +
      (k.env
        ? '<span class="help">Read from <code>ANTHROPIC_API_KEY</code> in this process\'s environment. Add it below to label, reorder or sync it.</span>'
        : '<div class="key__actions">' +
          useBtn +
          '<button class="ghost icon" data-act="up" title="Higher priority"' +
          (idx === 0 ? " disabled" : "") +
          ">↑</button>" +
          '<button class="ghost icon" data-act="down" title="Lower priority"' +
          (idx === total - 1 ? " disabled" : "") +
          ">↓</button>" +
          '<button class="ghost" data-act="test">Test</button>' +
          '<button class="ghost" data-act="edit">Edit</button>' +
          '<button class="ghost" data-act="toggle">' +
          (k.enabled ? "Disable" : "Enable") +
          "</button>" +
          '<span class="spacer"></span><span class="probe" data-probe></span>' +
          '<button class="ghost danger" data-act="remove">' +
          (state.armed === k.id ? "Click again to remove" : "Remove") +
          "</button></div>") +
      edit +
      "</article>"
    );
  }

  function renderKeys() {
    var v = state.keys;
    if (!v) return;
    var list = v.keys || [];
    var auto = v.failover && v.failover.enabled;
    $("keyList").innerHTML = list.length
      ? list
          .map(function (k, i) {
            return keyCard(k, i, list.length, auto);
          })
          .join("")
      : '<div class="empty"><b>No keys yet</b>Add one below. The first key you add becomes the live key.</div>';

    $("navKeyCount").textContent = list.length ? String(list.length) : "";

    var fo = v.failover || {};
    if (document.activeElement !== $("foThreshold")) $("foThreshold").value = fo.threshold_pct;
    $("foEnabled").checked = !!fo.enabled;
    $("foFailBack").checked = !!fo.fail_back;
    $("foFailBack").disabled = !fo.enabled;
    $("foThreshold").disabled = !fo.enabled;

    var sw = v.last_switch;
    $("switchLog").hidden = !sw;
    if (sw) {
      var synced = sw.synced && sw.synced.length ? " Pointed " + sw.synced.join(", ") + " at it." : "";
      var failed = sw.sync_errors && sw.sync_errors.length ? " Sync failed: " + sw.sync_errors.join(" · ") : "";
      $("switchLog").innerHTML =
        "<span>↪</span><span>" +
        esc(QB.clockTime(sw.at)) +
        " — switched " +
        (sw.from_label ? "from <b>" + esc(sw.from_label) + "</b> " : "") +
        "to <b>" +
        esc(sw.to_label) +
        "</b> (" +
        esc(sw.reason) +
        ")." +
        esc(synced) +
        (failed ? '<span style="color:#ff8f86">' + esc(failed) + "</span>" : "") +
        "</span>";
    }

    var live = list.filter(function (k) {
      return k.active;
    })[0];
    $("liveLabel").textContent = live ? live.label : "none";
    var tone = !live ? "warn" : live.health === "exhausted" ? "crit" : live.health === "ok" ? "ok" : "none";
    $("liveChip").className = "chip t-" + tone;
  }

  async function loadKeys() {
    try {
      state.keys = await invoke("list_keys");
      renderKeys();
    } catch (e) {
      say(errText(e), "err");
    }
  }

  /* Poll results land every interval; skip the redraw while something in
     the list is being typed into, or it would eat the edit. */
  function keysBusy() {
    var a = document.activeElement;
    return state.editing != null || (a && a.closest && a.closest("#keyList") && a.tagName === "INPUT");
  }

  function probeText(p) {
    if (!p.ok) return { text: p.error || "failed", kind: "err" };
    var limits = QB.collectLimits(p);
    var parts = limits
      .filter(function (l) {
        return !l.fable;
      })
      .map(function (l) {
        return l.window + " " + QB.pct(l.usedPct);
      });
    return { text: "Works" + (parts.length ? " — " + parts.join(" · ") : ""), kind: "ok" };
  }

  $("keyList").addEventListener("click", async function (e) {
    var btn = e.target.closest("button[data-act]");
    if (!btn) return;
    var card = btn.closest(".key");
    var id = card.getAttribute("data-id");
    var k = keyById(id);
    var act = btn.getAttribute("data-act");
    var probe = card.querySelector("[data-probe]");
    try {
      if (act === "reveal") {
        if (state.revealed[id]) delete state.revealed[id];
        else {
          state.revealed[id] = await invoke("reveal_key", { id: id });
          /* Do not leave a secret on screen indefinitely. */
          setTimeout(function () {
            if (state.revealed[id]) {
              delete state.revealed[id];
              renderKeys();
            }
          }, 30000);
        }
        renderKeys();
      } else if (act === "copy") {
        await copyText(await invoke("reveal_key", { id: id }));
        say("Copied " + (k ? k.label : "key") + " to the clipboard.", "ok");
      } else if (act === "use") {
        say("Switching…");
        await invoke("set_active_key", { id: id });
        say("Live key is now " + k.label + ".", "ok");
        await loadKeys();
      } else if (act === "up" || act === "down") {
        await invoke("move_key", { id: id, delta: act === "up" ? -1 : 1 });
        await loadKeys();
      } else if (act === "test") {
        if (probe) {
          probe.className = "probe";
          probe.textContent = "Reading…";
        }
        var p = await invoke("test_key", { id: id, baseUrl: k.effective_base_url });
        var t = probeText(p);
        if (probe) {
          probe.className = "probe " + t.kind;
          probe.textContent = t.text;
        }
      } else if (act === "edit") {
        state.editing = id;
        renderKeys();
        var first = card.parentNode.querySelector('.key[data-id="' + id + '"] input[data-f="label"]');
        if (first) first.focus();
      } else if (act === "edit-cancel") {
        state.editing = null;
        renderKeys();
      } else if (act === "edit-save") {
        var val = function (f) {
          var el = card.querySelector('[data-f="' + f + '"]');
          return el ? el.value : "";
        };
        await invoke("update_key", {
          id: id,
          label: val("label"),
          baseUrl: val("base"),
          secret: val("secret") || null,
          enabled: null
        });
        state.editing = null;
        delete state.revealed[id];
        say("Saved " + val("label") + ".", "ok");
        await loadKeys();
      } else if (act === "toggle") {
        await invoke("update_key", { id: id, label: null, baseUrl: null, secret: null, enabled: !k.enabled });
        await loadKeys();
      } else if (act === "remove") {
        if (state.armed !== id) {
          state.armed = id;
          renderKeys();
          setTimeout(function () {
            if (state.armed === id) {
              state.armed = null;
              renderKeys();
            }
          }, 3500);
          return;
        }
        state.armed = null;
        await invoke("remove_key", { id: id });
        delete state.revealed[id];
        say("Removed " + (k ? k.label : "key") + " and its secret.", "ok");
        await loadKeys();
      }
    } catch (err) {
      say(errText(err), "err");
    }
  });

  $("keyList").addEventListener("keydown", function (e) {
    if (e.key !== "Enter" || e.target.tagName !== "INPUT") return;
    var save = e.target.closest(".key").querySelector('[data-act="edit-save"]');
    if (save) save.click();
  });

  async function saveFailover() {
    try {
      await invoke("set_failover", {
        failover: {
          enabled: $("foEnabled").checked,
          threshold_pct: Number($("foThreshold").value) || 98,
          fail_back: $("foFailBack").checked
        }
      });
      say("Failover updated.", "ok");
      await loadKeys();
    } catch (e) {
      say(errText(e), "err");
    }
  }
  $("foEnabled").addEventListener("change", saveFailover);
  $("foFailBack").addEventListener("change", saveFailover);
  $("foThreshold").addEventListener("change", saveFailover);

  function setProbe(el, text, kind) {
    el.className = "probe grow" + (kind ? " " + kind : "");
    el.textContent = text;
  }

  $("newTest").addEventListener("click", async function () {
    setProbe($("newProbe"), "Reading…");
    try {
      var p = await invoke("test_key", {
        secret: $("newSecret").value,
        id: null,
        baseUrl: $("newBase").value || null
      });
      var t = probeText(p);
      setProbe($("newProbe"), t.text, t.kind);
    } catch (e) {
      setProbe($("newProbe"), errText(e), "err");
    }
  });

  $("newAdd").addEventListener("click", async function () {
    try {
      await invoke("add_key", {
        label: $("newLabel").value,
        secret: $("newSecret").value,
        baseUrl: $("newBase").value || null
      });
      $("newLabel").value = "";
      $("newSecret").value = "";
      $("newBase").value = "";
      setProbe($("newProbe"), "Added. Reading it now…", "ok");
      await loadKeys();
      setTimeout(loadKeys, 2500);
    } catch (e) {
      setProbe($("newProbe"), errText(e), "err");
    }
  });

  $("addCard").addEventListener("keydown", function (e) {
    if (e.key === "Enter" && e.target.tagName === "INPUT") $("newAdd").click();
  });

  /* ── Claude Code ────────────────────────────────────────────────────*/
  function keyOptions(t) {
    var list = ((state.keys && state.keys.keys) || []).filter(function (k) {
      return !k.env || t.key_id === k.id;
    });
    var current = t.key_id ? keyById(t.key_id) : null;
    var keep = current
      ? "Keep — " + current.label
      : t.key_preview
        ? "Keep — unknown key " + t.key_preview
        : "Keep — no key set";
    return (
      '<option value="">' +
      esc(keep) +
      "</option>" +
      list
        .filter(function (k) {
          return k.id !== t.key_id;
        })
        .map(function (k) {
          return (
            '<option value="' +
            esc(k.id) +
            '">' +
            esc(k.label + (k.active ? " (live)" : "") + " · " + (k.preview || "")) +
            "</option>"
          );
        })
        .join("")
    );
  }

  function targetChip(t) {
    if (!t.ok) return '<span class="chip t-crit"><span class="chip__dot"></span>Unreachable</span>';
    if (!t.exists) return '<span class="chip t-none"><span class="chip__dot"></span>No settings.json yet</span>';
    if (t.key_id) {
      var k = keyById(t.key_id);
      var live = k && k.active;
      return (
        '<span class="chip t-' +
        (live ? "ok" : "warn") +
        '"><span class="chip__dot"></span>' +
        esc(k ? (live ? "Uses the live key · " : "Uses ") + k.label : "Known key") +
        "</span>"
      );
    }
    if (t.key_preview)
      return (
        '<span class="chip t-warn"><span class="chip__dot"></span>Key not stored here · ' +
        esc(t.key_preview) +
        "</span>"
      );
    return '<span class="chip t-none"><span class="chip__dot"></span>No key in env</span>';
  }

  function targetCard(t) {
    var models = (t.models || [])
      .map(function (m) {
        return (
          '<label><span class="lab mono">' +
          esc(MODEL_LABELS[m[0]] || m[0]) +
          '</span><input class="mono" data-model="' +
          esc(m[0]) +
          '" value="' +
          esc(m[1] || "") +
          '" placeholder="default" spellcheck="false" /></label>'
        );
      })
      .join("");
    var dis = t.ok ? "" : " disabled";
    return (
      '<article class="card target" data-id="' +
      esc(t.id) +
      '">' +
      '<div class="target__head"><div class="target__icon">' +
      (t.kind === "local" ? "💻" : "🖥") +
      '</div><div class="grow"><div class="target__name">' +
      esc(t.label) +
      '</div><div class="target__path mono">' +
      esc(t.path) +
      "</div></div>" +
      targetChip(t) +
      (t.kind === "local" ? "" : '<button class="ghost danger icon" data-act="remove" title="Forget this machine">✕</button>') +
      "</div>" +
      (t.ok ? "" : '<div class="target__err">' + esc(t.error) + "</div>") +
      (t.key_mismatch
        ? '<div class="help">⚠ <code>ANTHROPIC_API_KEY</code> and <code>ANTHROPIC_AUTH_TOKEN</code> hold different keys. Applying a key sets both.</div>'
        : "") +
      '<label class="switch"><input type="checkbox" data-act="sync"' +
      (t.sync ? " checked" : "") +
      ' /><span class="switch__text"><span class="lab">Follow the live key</span><span class="help">Rewrite this file\'s key and base URL whenever failover switches.</span></span></label>' +
      '<div class="grid2">' +
      '<label><span class="lab">API key</span><select data-f="key"' +
      dis +
      ">" +
      keyOptions(t) +
      "</select></label>" +
      '<label><span class="lab">ANTHROPIC_BASE_URL</span><input data-f="base" spellcheck="false" value="' +
      esc(t.base_url || "") +
      '" placeholder="not set"' +
      dis +
      " /></label>" +
      "</div>" +
      '<div class="models">' +
      models +
      '<label><span class="lab mono">model</span><input class="mono" data-f="model" value="' +
      esc(t.model || "") +
      '" placeholder="default" spellcheck="false"' +
      dis +
      " /></label></div>" +
      '<div class="target__foot"><span class="probe grow" data-probe></span>' +
      '<button data-act="reload">Reload</button>' +
      '<button class="primary" data-act="apply"' +
      dis +
      ">Apply</button></div>" +
      "</article>"
    );
  }

  function renderTargets() {
    var list = state.targets || [];
    $("targetList").innerHTML = list.map(targetCard).join("");
    $("navTargetCount").textContent = list.length ? String(list.length) : "";
  }

  async function loadTargets() {
    if (!state.targets) {
      $("targetList").innerHTML =
        '<div class="empty"><b>Reading settings.json…</b>Remote machines are read over ssh and can take a few seconds.</div>';
    } else {
      $("targetList").classList.add("is-loading");
    }
    try {
      if (!state.keys) await loadKeys();
      state.targets = await invoke("list_claude_targets");
      renderTargets();
      /* Reading targets refreshes which key each one uses. */
      loadKeys();
    } catch (e) {
      say(errText(e), "err");
    } finally {
      $("targetList").classList.remove("is-loading");
    }
  }

  function replaceTarget(view) {
    state.targets = (state.targets || []).map(function (t) {
      return t.id === view.id ? view : t;
    });
    renderTargets();
  }

  $("targetList").addEventListener("change", async function (e) {
    var card = e.target.closest(".target");
    if (!card) return;
    var id = card.getAttribute("data-id");
    if (e.target.getAttribute("data-act") === "sync") {
      try {
        await invoke("set_target_sync", { id: id, sync: e.target.checked });
        state.targets.forEach(function (t) {
          if (t.id === id) t.sync = e.target.checked;
        });
        say(e.target.checked ? "Will follow the live key." : "Sync off.", "ok");
      } catch (err) {
        e.target.checked = !e.target.checked;
        say(errText(err), "err");
      }
    } else if (e.target.getAttribute("data-f") === "key" && e.target.value) {
      /* A key with its own proxy brings that URL along. */
      var k = keyById(e.target.value);
      var base = card.querySelector('[data-f="base"]');
      if (k && base) base.value = k.effective_base_url;
    }
  });

  $("targetList").addEventListener("click", async function (e) {
    var btn = e.target.closest("button[data-act]");
    if (!btn) return;
    var card = btn.closest(".target");
    var id = card.getAttribute("data-id");
    var act = btn.getAttribute("data-act");
    var probe = card.querySelector("[data-probe]");
    var show = function (text, kind) {
      probe.className = "probe grow" + (kind ? " " + kind : "");
      probe.textContent = text;
    };
    try {
      if (act === "reload") {
        await loadTargets();
      } else if (act === "remove") {
        if (state.armed !== id) {
          state.armed = id;
          btn.textContent = "Remove?";
          setTimeout(function () {
            if (state.armed === id) {
              state.armed = null;
              btn.textContent = "✕";
            }
          }, 3500);
          return;
        }
        state.armed = null;
        await invoke("remove_claude_target", { id: id });
        state.targets = state.targets.filter(function (t) {
          return t.id !== id;
        });
        renderTargets();
      } else if (act === "apply") {
        var models = [];
        card.querySelectorAll("[data-model]").forEach(function (el) {
          models.push([el.getAttribute("data-model"), el.value]);
        });
        var keyId = card.querySelector('[data-f="key"]').value || null;
        show("Writing…");
        card.classList.add("is-loading");
        var view = await invoke("apply_claude", {
          apply: {
            target_id: id,
            key_id: keyId,
            base_url: card.querySelector('[data-f="base"]').value,
            models: models,
            model: card.querySelector('[data-f="model"]').value
          }
        });
        replaceTarget(view);
        var fresh = $("targetList").querySelector('.target[data-id="' + id + '"] [data-probe]');
        if (fresh) {
          fresh.className = "probe grow ok";
          fresh.textContent = "Written. A backup is next to the file as .quotabar-bak.";
        }
        loadKeys();
      }
    } catch (err) {
      card.classList.remove("is-loading");
      show(errText(err), "err");
    }
  });

  $("addHost").addEventListener("click", async function () {
    try {
      await invoke("add_claude_target", { host: $("newHost").value, path: $("newPath").value || null });
      $("newHost").value = "";
      $("newPath").value = "";
      say("Added. Reading it…");
      await loadTargets();
      say("");
    } catch (e) {
      say(errText(e), "err");
    }
  });

  $("reloadTargets").addEventListener("click", loadTargets);

  $("syncNow").addEventListener("click", async function () {
    say("Pushing the live key…");
    try {
      var done = await invoke("sync_claude_now");
      say("Updated " + done.join(", ") + ".", "ok");
      await loadTargets();
    } catch (e) {
      say(errText(e), "err");
    }
  });

  /* ── general ────────────────────────────────────────────────────────*/
  async function loadGeneral() {
    var cfg = await invoke("get_settings");
    $("base").value = cfg.base_url || "";
    $("interval").value = cfg.poll_interval_secs || 60;
    $("width").value = cfg.bar_width || 340;
    $("pro").value = cfg.pro_usd || 20;
    $("dailyQuota").value = cfg.daily_quota_usd || 6400;
    $("reset").value = cfg.daily_reset_utc || "";
  }

  $("save").onclick = async function () {
    say("Saving…");
    try {
      await invoke("save_settings", {
        settings: {
          base_url: $("base").value.trim(),
          poll_interval_secs: Number($("interval").value),
          bar_width: Number($("width").value),
          pro_usd: Number($("pro").value),
          daily_quota_usd: Number($("dailyQuota").value),
          daily_reset_utc: $("reset").value.trim() || null
        }
      });
      await loadGeneral();
      say("Saved. Fetching again now.", "ok");
      await invoke("refresh_now");
    } catch (e) {
      say(errText(e), "err");
    }
  };

  $("pane-general").addEventListener("keydown", function (e) {
    if (e.key === "Enter" && e.target.tagName === "INPUT") $("save").click();
  });

  $("cancel").onclick = function () {
    getCurrentWindow().hide();
  };

  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape") {
      if (state.editing) {
        state.editing = null;
        renderKeys();
        return;
      }
      getCurrentWindow().hide();
    }
  });

  /* ── live updates ───────────────────────────────────────────────────*/
  var lastKeyLoad = 0;
  listen("quota-update", function () {
    if (document.hidden || keysBusy()) return;
    if (Date.now() - lastKeyLoad < 5000) return;
    lastKeyLoad = Date.now();
    loadKeys();
  });
  listen("key-switched", function (ev) {
    var sw = ev.payload || {};
    say("Failover: now on " + (sw.to_label || "another key") + ".", "ok");
    loadKeys();
    if (state.targets) loadTargets();
  });

  /* Hidden, never closed: pick up changes made while it was away. */
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden) {
      loadKeys();
      loadGeneral();
    }
  });

  loadGeneral().catch(function (e) {
    say(errText(e), "err");
  });
  loadKeys();
})();
