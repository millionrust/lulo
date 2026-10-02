// Read interaction-probe facts from a macOS process over the AX API.
// Called by mac_probe.py: osascript -l JavaScript mac_observe.js PROCESS MODE ARGS...
//
// Unlike scripts/behavior/mac_observe.js's "menu" fact (which deliberately
// skips AXMenuBar: a *context* menu hangs off the element it was opened on,
// never off the menu bar), this file's "menu_open" mode looks at exactly one
// named AXMenuBarItem, because that is the surface under test here.
function run(argv) {
  var se = Application("System Events");
  var process = argv[0];
  var mode = argv[1];
  var p = se.processes.byName(process);

  function A(el, n) {
    try { return el.attributes.byName(n).value(); } catch (e) { return undefined; }
  }
  function kids(el) { return A(el, "AXChildren") || []; }
  function bounds(el) {
    var pos = A(el, "AXPosition"), size = A(el, "AXSize");
    if (!pos || !size) return null;
    return [pos[0], pos[1], size[0], size[1]];
  }
  function walk(el, depth, visit) {
    if (visit(el) === false || depth <= 0) return;
    var k = kids(el);
    for (var i = 0; i < k.length; i++) walk(k[i], depth - 1, visit);
  }
  function menuBar() {
    // Both an app's own menu bar (Finder's "File") and the menu-extras bar
    // (ControlCenter's "Control Centre", "Clock") are AXMenuBar at index 0.
    var top = kids(p);
    for (var i = 0; i < top.length; i++) if (A(top[i], "AXRole") === "AXMenuBar") return top[i];
    return null;
  }
  function findBarItem(matcher) {
    var bar = menuBar();
    if (!bar) return null;
    var items = kids(bar);
    for (var i = 0; i < items.length; i++) {
      var item = items[i];
      if (matcher.kind === "title" && A(item, "AXTitle") === matcher.value) return item;
      if (matcher.kind === "desc" && A(item, "AXDescription") === matcher.value) return item;
    }
    return null;
  }
  function parseMatcher(raw) {
    var sep = raw.indexOf(":");
    return { kind: raw.slice(0, sep), value: raw.slice(sep + 1) };
  }

  if (mode === "bar_item_bounds") {
    var item = findBarItem(parseMatcher(argv[2]));
    if (!item) return "none";
    var box = bounds(item);
    return box ? box.join(" ") : "none";
  }

  if (mode === "menu_open") {
    // A pulldown's own menu ("menu 1 of menu bar item X") always exists as
    // a static AXMenu child of the bar item, whether or not it is showing.
    // While closed it sits at a degenerate AXSize [0, 0] and a parked
    // AXPosition (measured live: [0, 956] after Escape/an outside click,
    // but [0, 651] after switching straight to a *different* menu bar item
    // - the parked Y is not a fixed constant). Presence alone (what this
    // file's "menu_items" mode and scripts/behavior/mac_observe.js's "menu"
    // fact both read) is therefore not "is it open"; nor is non-zero size
    // alone, which a just-switched-away-from menu can still briefly report.
    // Every real top-level pulldown renders flush under the menu bar, so a
    // non-zero size *and* a Y position near the bar (not parked, wherever
    // "parked" happens to be that time) is the reliable signal.
    var target = findBarItem(parseMatcher(argv[2]));
    if (!target) return "unknown";
    var children = kids(target);
    for (var j = 0; j < children.length; j++) {
      if (A(children[j], "AXRole") !== "AXMenu") continue;
      var size = A(children[j], "AXSize");
      var pos = A(children[j], "AXPosition");
      if (size && pos && size[0] > 0 && size[1] > 0 && pos[1] >= 0 && pos[1] < 60) return "true";
    }
    return "false";
  }

  if (mode === "menu_items") {
    var target2 = findBarItem(parseMatcher(argv[2]));
    if (!target2) return "none";
    var out = [];
    var children2 = kids(target2);
    for (var k = 0; k < children2.length; k++) {
      if (A(children2[k], "AXRole") !== "AXMenu") continue;
      var rows = kids(children2[k]);
      for (var m = 0; m < rows.length; m++) {
        var t = A(rows[m], "AXTitle");
        if (t) out.push(t);
      }
    }
    return out.length ? out.join("\u0001") : "none";
  }

  if (mode === "window_count") {
    var wins = A(p, "AXWindows") || [];
    var titleWanted = argv[2] || "";
    var count = 0;
    for (var w = 0; w < wins.length; w++) {
      if (!titleWanted || A(wins[w], "AXTitle") === titleWanted) count++;
    }
    return String(count);
  }

  if (mode === "control_bounds") {
    // A control (AXSlider etc.) by AXDescription, searched inside this
    // process's windows only (never system-wide).
    var role = argv[2], description = argv[3];
    var wins2 = A(p, "AXWindows") || [];
    var hit = null;
    for (var w2 = 0; w2 < wins2.length && !hit; w2++) {
      walk(wins2[w2], 14, function (el) {
        if (hit) return false;
        if (A(el, "AXRole") === role && A(el, "AXDescription") === description) { hit = el; return false; }
      });
    }
    if (!hit) return "none";
    var box2 = bounds(hit);
    var value = A(hit, "AXValue");
    return box2 ? (box2.join(" ") + " " + (typeof value === "number" ? value : "")) : "none";
  }

  if (mode === "frontmost") {
    try { return se.processes.whose({ frontmost: true })[0].name(); } catch (e) { return "none"; }
  }

  return "error:unknown-mode:" + mode;
}
