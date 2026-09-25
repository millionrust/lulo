// Read portable behaviour facts from a macOS app over the AX API.
// Called by record_mac.py: osascript -l JavaScript mac_observe.js PROCESS FACTS BASELINE
// FACTS is a comma list; BASELINE is a JSON list of window keys that existed
// before the scenario started (the owner's windows), which are never reported.
function run(argv) {
  var se = Application("System Events");
  var procName = argv[0];
  var facts = (argv[1] || "").split(",");
  var baseline = JSON.parse(argv[2] || "[]");
  var out = {};

  function A(el, n) {
    try { return el.attributes.byName(n).value(); } catch (e) { return undefined; }
  }
  function winKey(w) {
    try { return (A(w, "AXTitle") || "") + "|" + A(w, "AXSubrole") + "|" + JSON.stringify(A(w, "AXPosition")); }
    catch (e) { return "?"; }
  }
  function kids(el) { return A(el, "AXChildren") || []; }
  function walk(el, depth, visit) {
    if (visit(el) === false || depth <= 0) return;
    var k = kids(el);
    for (var i = 0; i < k.length; i++) walk(k[i], depth - 1, visit);
  }
  function str(v) { return (v === undefined || v === null || typeof v === "object") ? null : String(v); }
  function pos(el) { var p = A(el, "AXPosition"); return p ? [Math.round(p[1] / 6), p[0]] : [0, 0]; }

  var front = null;
  try { front = se.processes.whose({ frontmost: true })[0].name(); } catch (e) {}
  out.frontmost = front;
  var proc = se.processes.byName(procName);
  var exists = false;
  try { exists = proc.exists(); } catch (e) {}
  out.running = exists;
  if (!exists) return JSON.stringify(out);

  var wins = kids(proc).filter(function (w) { return A(w, "AXRole") === "AXWindow"; });
  var ours = wins.filter(function (w) { return baseline.indexOf(winKey(w)) < 0; });
  var focusedWin = A(proc, "AXFocusedWindow");
  var focusedKey = focusedWin ? winKey(focusedWin) : null;
  out.guard = {
    windows: wins.length,
    ours: ours.length,
    focused_window_is_ours: focusedKey !== null && baseline.indexOf(focusedKey) < 0,
    focused_window: focusedKey === null ? null : (A(focusedWin, "AXTitle") || ""),
  };
  if (facts.indexOf("baseline") >= 0) {
    out.baseline = wins.map(winKey);
  }

  var dialogKinds = ["AXDialog", "AXSystemDialog"];
  if (facts.indexOf("windows") >= 0) {
    var plain = ours.filter(function (w) { return dialogKinds.indexOf(A(w, "AXSubrole")) < 0; });
    out.windows = {
      count: plain.length,
      front: focusedWin && baseline.indexOf(focusedKey) < 0 && dialogKinds.indexOf(A(focusedWin, "AXSubrole")) < 0
        ? (A(focusedWin, "AXTitle") || "") : null,
      titles: plain.map(function (w) { return A(w, "AXTitle") || ""; }),
      subroles: plain.map(function (w) { return A(w, "AXSubrole") || ""; }),
    };
  }

  if (facts.indexOf("focus") >= 0) {
    var f = A(proc, "AXFocusedUIElement");
    if (f) {
      var range = A(f, "AXSelectedTextRange");
      out.focus = {
        ax_role: A(f, "AXRole") || null,
        ax_subrole: A(f, "AXSubrole") || null,
        value: str(A(f, "AXValue")),
        range: range || null,
        label: str(A(f, "AXTitle")) || str(A(f, "AXDescription")) || str(A(f, "AXPlaceholderValue")),
      };
    } else {
      out.focus = null;
    }
  }

  function dialogOf() {
    var w = focusedWin && baseline.indexOf(focusedKey) < 0 ? focusedWin : null;
    if (!w) return null;
    if (A(w, "AXRole") === "AXSheet") return { el: w, kind: "sheet" };
    var sheets = kids(w).filter(function (c) { return A(c, "AXRole") === "AXSheet"; });
    if (sheets.length) return { el: sheets[0], kind: "sheet" };
    if (dialogKinds.indexOf(A(w, "AXSubrole")) >= 0) return { el: w, kind: "window" };
    return null;
  }
  if (facts.indexOf("dialog") >= 0) {
    var d = dialogOf();
    if (!d) {
      out.dialog = { present: false };
    } else {
      var texts = [], buttons = [];
      walk(d.el, 8, function (el) {
        var role = A(el, "AXRole");
        if (role === "AXStaticText") {
          var v = str(A(el, "AXValue"));
          if (v) texts.push(v);
        } else if (role === "AXButton" && ["AXCloseButton", "AXMinimizeButton", "AXZoomButton", "AXFullScreenButton"].indexOf(A(el, "AXSubrole")) < 0) {
          var t = str(A(el, "AXTitle")) || str(A(el, "AXDescription"));
          if (t) buttons.push({ title: t, pos: pos(el) });
        } else if (role === "AXOutline" || role === "AXTable" || role === "AXBrowser") {
          return false;
        }
      });
      buttons.sort(function (a, b) { return a.pos[0] - b.pos[0] || a.pos[1] - b.pos[1]; });
      var def = A(d.el, "AXDefaultButton") || A(A(d.el, "AXParent") || d.el, "AXDefaultButton");
      // An alert's title is its message: the first sentence-like text, not
      // a field label such as "Where:".
      var sentence = texts.filter(function (t) { return t.split(/\s+/).length >= 3; })[0] || null;
      out.dialog = {
        present: true,
        kind: d.kind,
        title: str(A(d.el, "AXTitle")) || sentence,
        texts: texts,
        buttons: buttons.map(function (b) { return b.title; }),
        default: def ? (str(A(def, "AXTitle")) || str(A(def, "AXDescription"))) : null,
      };
    }
  }

  if (facts.indexOf("menu") >= 0) {
    // A context menu hangs off the element it was opened on (Finder: the
    // list's AXOutline), never off the menu bar.
    var menus = [];
    walk(proc, 12, function (el) {
      var role = A(el, "AXRole");
      if (role === "AXMenu") { menus.push(el); return false; }
      if (role === "AXMenuBar" || role === "AXRow" || role === "AXStaticText" || role === "AXButton") return false;
    });
    if (!menus.length) {
      out.menu = { present: false };
    } else {
      // Option-key alternates ("Duplicate Exactly") share their primary
      // item's row, so only the first item at each height is visible.
      var rows = {};
      var items = [];
      if (facts.indexOf("menu-debug") >= 0) {
        out.menu_debug = kids(menus[menus.length - 1]).map(function (item) {
          return [str(A(item, "AXTitle")), A(item, "AXPosition"), A(item, "AXSize"), A(item, "AXEnabled")];
        });
      }
      kids(menus[menus.length - 1]).forEach(function (item) {
        var p = A(item, "AXPosition");
        var s = A(item, "AXSize");
        var t = str(A(item, "AXTitle"));
        if (s && s[1] === 0) return;
        // Untitled full-height items are hidden placeholders; a separator
        // is a short untitled row.
        if (!t && s && s[1] >= 16) return;
        var key = p ? String(Math.round(p[1])) : String(items.length);
        if (rows[key]) return;
        rows[key] = true;
        if (!t) { items.push("-"); return; }
        var mark = str(A(item, "AXMenuItemMarkChar"));
        items.push((mark ? "✓ " : "") + t + (A(item, "AXEnabled") === false ? " [disabled]" : ""));
      });
      out.menu = { present: true, items: items };
    }
  }

  if (facts.indexOf("tabs") >= 0) {
    var titles = null;
    if (focusedWin) {
      walk(focusedWin, 3, function (el) {
        if (titles) return false;
        if (A(el, "AXRole") === "AXTabGroup") {
          titles = kids(el).filter(function (c) { return A(c, "AXRole") === "AXRadioButton"; })
            .map(function (c) { return str(A(c, "AXTitle")) || str(A(c, "AXDescription")) || ""; });
          return false;
        }
      });
    }
    if (!titles || !titles.length) titles = focusedWin ? [A(focusedWin, "AXTitle") || ""] : [];
    out.tabs = { count: titles.length, titles: titles };
  }

  if (facts.indexOf("display") >= 0) {
    var value = null;
    if (focusedWin) {
      walk(focusedWin, 8, function (el) {
        if (value !== null) return false;
        if (A(el, "AXIdentifier") === "StandardInputView") {
          walk(el, 2, function (t) { if (value === null && A(t, "AXRole") === "AXStaticText") value = str(A(t, "AXValue")); });
          return false;
        }
      });
    }
    out.display = { value: value };
  }

  if (facts.indexOf("selection") >= 0 && procName === "Finder") {
    try {
      var finder = Application("Finder");
      out.selection = { items: finder.selection().map(function (i) { return i.name(); }) };
    } catch (e) {
      out.selection = { items: [] };
    }
  }
  return JSON.stringify(out);
}
