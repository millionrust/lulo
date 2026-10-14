// macax: the Mac robot's accessibility, input and image helper.
//
// Built by mac_walk.py into .claude/mac-captures/bin/macax (never committed):
//     swiftc -O -o .claude/mac-captures/bin/macax scripts/robot/mac/macax.swift
//
// Every command prints one JSON document on stdout.
//
//   macax apps                              running GUI apps
//   macax windows PID                       AX windows (+ CGWindowID) of an app
//   macax tree PID TARGET [--max N]         AX tree summary; TARGET is
//                                           focused | w<i> | menubar | app | extras | path
//   macax menubar PID [--deep]              static menu-bar tree (no opening)
//   macax openmenus PID                     menus that are open right now (frames)
//   macax press PID PATH [ACTION]           AXPress (or ACTION) on a path from `tree`
//   macax select PID PATH                   set AXSelected=true on a row (navigate)
//   macax popups PID                        on-screen pop-up windows (menus, popovers)
//   macax key CODE [cmd,shift,alt,ctrl]     post one key press
//   macax keydown CODE [mods] / keyup CODE [mods]
//   macax click left|right X Y              click at a screen point (pointer restored)
//   macax move X Y                          move the pointer (hover)
//   macax pointer                           current pointer location
//   macax idle                              seconds since the last HID input
//   macax score A B                         perceptual similarity of two images (0..1)
//   macax sbs A B OUT [LABEL_A LABEL_B]     side-by-side JPEG of A and B
//   macax shrink IN OUT MAXW                write IN as a JPEG (q 0.8), at most MAXW px wide
//
// Paths: "w0/3/1" is window 0, child 3, child 1. "mb/2/0/5" is menu-bar item
// 2, its menu, item 5. "ex/1" is extras menu-bar item 1. "app/…" starts at
// the application element.

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import ImageIO
import IOKit
import UniformTypeIdentifiers

// MARK: - JSON output

func emit(_ value: Any) {
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
}

func fail(_ message: String) -> Never {
    emit(["error": message])
    exit(1)
}

// MARK: - AX helpers

AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 3.0)

func attr(_ el: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    let err = AXUIElementCopyAttributeValue(el, name as CFString, &value)
    return err == .success ? value : nil
}

func attrs(_ el: AXUIElement, _ names: [String]) -> [String: AnyObject] {
    var values: CFArray?
    let err = AXUIElementCopyMultipleAttributeValues(el, names as CFArray, AXCopyMultipleAttributeOptions(rawValue: 0), &values)
    var out: [String: AnyObject] = [:]
    guard err == .success, let array = values as? [AnyObject] else { return out }
    for (index, name) in names.enumerated() where index < array.count {
        let value = array[index]
        if CFGetTypeID(value) == AXValueGetTypeID() {
            let axv = value as! AXValue
            if AXValueGetType(axv) == .axError { continue }
        }
        out[name] = value
    }
    return out
}

func children(_ el: AXUIElement) -> [AXUIElement] {
    return (attr(el, "AXChildren") as? [AXUIElement]) ?? []
}

func point(_ v: AnyObject?) -> CGPoint? {
    guard let v = v, CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var p = CGPoint.zero
    return AXValueGetValue(v as! AXValue, .cgPoint, &p) ? p : nil
}

func size(_ v: AnyObject?) -> CGSize? {
    guard let v = v, CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var s = CGSize.zero
    return AXValueGetValue(v as! AXValue, .cgSize, &s) ? s : nil
}

func text(_ v: AnyObject?, limit: Int = 160) -> String? {
    guard let v = v else { return nil }
    if let s = v as? String {
        if s.isEmpty { return nil }
        return s.count > limit ? String(s.prefix(limit)) + "…" : s
    }
    if let n = v as? NSNumber { return n.stringValue }
    if let a = v as? NSAttributedString { return text(a.string as AnyObject, limit: limit) }
    return nil
}

func frame(_ el: AXUIElement) -> CGRect? {
    let a = attrs(el, ["AXPosition", "AXSize"])
    guard let p = point(a["AXPosition"]), let s = size(a["AXSize"]) else { return nil }
    return CGRect(origin: p, size: s)
}

func rectJSON(_ r: CGRect, origin: CGPoint = .zero) -> [Int] {
    return [Int((r.origin.x - origin.x).rounded()), Int((r.origin.y - origin.y).rounded()),
            Int(r.size.width.rounded()), Int(r.size.height.rounded())]
}

func appElement(_ pid: pid_t) -> AXUIElement {
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 3.0)
    return app
}

func windows(_ app: AXUIElement) -> [AXUIElement] {
    return (attr(app, "AXWindows") as? [AXUIElement]) ?? []
}

func resolve(_ pid: pid_t, _ path: String) -> AXUIElement? {
    let app = appElement(pid)
    var parts = path.split(separator: "/").map(String.init)
    guard !parts.isEmpty else { return nil }
    let head = parts.removeFirst()
    var el: AXUIElement
    if head == "app" {
        el = app
    } else if head == "mb" || head == "ex" {
        guard let bar = attr(app, head == "mb" ? "AXMenuBar" : "AXExtrasMenuBar") else { return nil }
        el = bar as! AXUIElement
    } else if head == "focused" {
        guard let w = attr(app, "AXFocusedWindow") else { return nil }
        el = w as! AXUIElement
    } else if head.hasPrefix("w"), let i = Int(head.dropFirst()) {
        let ws = windows(app)
        guard i < ws.count else { return nil }
        el = ws[i]
    } else {
        return nil
    }
    for part in parts {
        guard let i = Int(part) else { return nil }
        let kids = children(el)
        guard i < kids.count else { return nil }
        el = kids[i]
    }
    return el
}

// Containers whose rows are content (files, mail, notes, processes): only
// the first few rows are walked, the rest are counted.
let contentContainers: Set<String> = ["AXTable", "AXOutline", "AXList", "AXBrowser", "AXGrid", "AXCollection"]
let keyNames = ["AXRole", "AXSubrole", "AXTitle", "AXDescription", "AXValue", "AXEnabled", "AXSelected",
                "AXIdentifier", "AXPosition", "AXSize", "AXHelp", "AXPlaceholderValue", "AXRoleDescription",
                "AXMenuItemCmdChar", "AXMenuItemCmdModifiers", "AXMenuItemMarkChar", "AXChildren"]

func walk(_ root: AXUIElement, rootPath: String, origin: CGPoint, maxNodes: Int, maxDepth: Int, rowCap: Int) -> [[String: Any]] {
    var out: [[String: Any]] = []
    func visit(_ el: AXUIElement, _ path: String, _ depth: Int) {
        if out.count >= maxNodes { return }
        let a = attrs(el, keyNames)
        let role = (a["AXRole"] as? String) ?? "?"
        var node: [String: Any] = ["p": path, "d": depth, "role": role]
        if let s = a["AXSubrole"] as? String { node["sub"] = s }
        if let t = text(a["AXTitle"]) { node["title"] = t }
        if let t = text(a["AXDescription"]) { node["desc"] = t }
        if let t = text(a["AXHelp"]) { node["help"] = t }
        if let t = text(a["AXPlaceholderValue"]) { node["ph"] = t }
        if let t = text(a["AXIdentifier"]) { node["id"] = t }
        if role != "AXTextArea" && role != "AXWebArea", let t = text(a["AXValue"], limit: 80) { node["value"] = t }
        if let e = a["AXEnabled"] as? Bool, !e { node["disabled"] = true }
        if let s = a["AXSelected"] as? Bool, s { node["selected"] = true }
        if let c = text(a["AXMenuItemCmdChar"]) { node["key"] = c; node["mods"] = (a["AXMenuItemCmdModifiers"] as? NSNumber)?.intValue ?? 0 }
        if let m = text(a["AXMenuItemMarkChar"]) { node["mark"] = m }
        if let p = point(a["AXPosition"]), let s = size(a["AXSize"]) {
            node["f"] = rectJSON(CGRect(origin: p, size: s), origin: origin)
        }
        let kids = (a["AXChildren"] as? [AXUIElement]) ?? []
        node["n"] = kids.count
        out.append(node)
        if depth >= maxDepth { return }
        let cap = contentContainers.contains(role) ? rowCap : kids.count
        // A closed menu bar's menus are walked by `menubar`, not here.
        if role == "AXMenuBar" && depth > 0 { return }
        for (i, kid) in kids.prefix(cap).enumerated() {
            visit(kid, path + "/" + String(i), depth + 1)
        }
    }
    visit(root, rootPath, 0)
    return out
}

// MARK: - CG windows

func cgWindows(pid: pid_t?) -> [[String: Any]] {
    guard let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return [] }
    var out: [[String: Any]] = []
    for w in info {
        let owner = (w[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? -1
        if let pid = pid, owner != pid { continue }
        guard let b = w[kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: b as CFDictionary) else { continue }
        out.append([
            "id": (w[kCGWindowNumber as String] as? NSNumber)?.intValue ?? 0,
            "pid": Int(owner),
            "owner": (w[kCGWindowOwnerName as String] as? String) ?? "",
            "layer": (w[kCGWindowLayer as String] as? NSNumber)?.intValue ?? 0,
            "name": (w[kCGWindowName as String] as? String) ?? "",
            "f": rectJSON(rect),
        ])
    }
    return out
}

// MARK: - Input

func post(_ event: CGEvent?) {
    event?.post(tap: .cghidEventTap)
}

func modifierFlags(_ spec: String) -> CGEventFlags {
    var flags: CGEventFlags = []
    for m in spec.split(separator: ",") {
        switch m {
        case "cmd": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt", "opt": flags.insert(.maskAlternate)
        case "ctrl": flags.insert(.maskControl)
        case "fn": flags.insert(.maskSecondaryFn)
        default: break
        }
    }
    return flags
}

func key(_ code: CGKeyCode, _ mods: String, down: Bool? = nil) {
    let src = CGEventSource(stateID: .hidSystemState)
    let flags = modifierFlags(mods)
    if down != false {
        let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: true)
        e?.flags = flags
        post(e)
    }
    if down == nil { usleep(30_000) }
    if down != true {
        let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: false)
        e?.flags = flags
        post(e)
    }
}

func pointerLocation() -> CGPoint {
    return CGEvent(source: nil)?.location ?? .zero
}

func click(_ button: String, _ p: CGPoint) {
    let previous = pointerLocation()
    let (down, up, b): (CGEventType, CGEventType, CGMouseButton) =
        button == "right" ? (.rightMouseDown, .rightMouseUp, .right) : (.leftMouseDown, .leftMouseUp, .left)
    post(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left))
    usleep(60_000)
    post(CGEvent(mouseEventSource: nil, mouseType: down, mouseCursorPosition: p, mouseButton: b))
    usleep(50_000)
    post(CGEvent(mouseEventSource: nil, mouseType: up, mouseCursorPosition: p, mouseButton: b))
    usleep(300_000)
    if button == "left" { CGWarpMouseCursorPosition(previous) }
}

func move(_ p: CGPoint) {
    post(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left))
}

func idleSeconds() -> Double {
    let service = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOHIDSystem"))
    defer { IOObjectRelease(service) }
    guard let value = IORegistryEntryCreateCFProperty(service, "HIDIdleTime" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? NSNumber else { return -1 }
    return value.doubleValue / 1_000_000_000
}

// MARK: - Images

func loadImage(_ path: String) -> CGImage? {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(src, 0, nil)
}

func writeJPEG(_ image: CGImage, _ path: String, quality: Double = 0.8) {
    guard let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else { fail("cannot write \(path)") }
    CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
    CGImageDestinationFinalize(dest)
}

func render(_ image: CGImage, width: Int, height: Int, gray: Bool) -> [UInt8] {
    let space = gray ? CGColorSpaceCreateDeviceGray() : CGColorSpaceCreateDeviceRGB()
    let bpp = gray ? 1 : 4
    var buf = [UInt8](repeating: 0, count: width * height * bpp)
    let info = gray ? CGImageAlphaInfo.none.rawValue : CGImageAlphaInfo.noneSkipLast.rawValue
    buf.withUnsafeMutableBytes { raw in
        let ctx = CGContext(data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8,
                            bytesPerRow: width * bpp, space: space, bitmapInfo: info)!
        ctx.interpolationQuality = .high
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    }
    return buf
}

// A structural similarity over a 64x64 grey thumbnail (mean SSIM of 8x8
// blocks) blended with a colour-histogram intersection: 1 is identical.
func perceptual(_ a: CGImage, _ b: CGImage) -> [String: Double] {
    let n = 64
    let ga = render(a, width: n, height: n, gray: true).map(Double.init)
    let gb = render(b, width: n, height: n, gray: true).map(Double.init)
    var total = 0.0, blocks = 0.0
    let c1 = 6.5025, c2 = 58.5225
    for by in stride(from: 0, to: n, by: 8) {
        for bx in stride(from: 0, to: n, by: 8) {
            var xs: [Double] = [], ys: [Double] = []
            for y in by..<by + 8 { for x in bx..<bx + 8 { xs.append(ga[y * n + x]); ys.append(gb[y * n + x]) } }
            let mx = xs.reduce(0, +) / 64, my = ys.reduce(0, +) / 64
            var vx = 0.0, vy = 0.0, cov = 0.0
            for i in 0..<64 { vx += (xs[i] - mx) * (xs[i] - mx); vy += (ys[i] - my) * (ys[i] - my); cov += (xs[i] - mx) * (ys[i] - my) }
            vx /= 63; vy /= 63; cov /= 63
            total += ((2 * mx * my + c1) * (2 * cov + c2)) / ((mx * mx + my * my + c1) * (vx + vy + c2))
            blocks += 1
        }
    }
    let ssim = total / blocks
    let ca = render(a, width: 32, height: 32, gray: false), cb = render(b, width: 32, height: 32, gray: false)
    var ha = [Double](repeating: 0, count: 64), hb = [Double](repeating: 0, count: 64)
    for i in stride(from: 0, to: ca.count, by: 4) {
        ha[Int(ca[i] >> 6) * 16 + Int(ca[i + 1] >> 6) * 4 + Int(ca[i + 2] >> 6)] += 1
        hb[Int(cb[i] >> 6) * 16 + Int(cb[i + 1] >> 6) * 4 + Int(cb[i + 2] >> 6)] += 1
    }
    var inter = 0.0
    for i in 0..<64 { inter += min(ha[i], hb[i]) }
    let hist = inter / 1024.0
    let aspectA = Double(a.width) / Double(max(a.height, 1)), aspectB = Double(b.width) / Double(max(b.height, 1))
    let aspect = min(aspectA, aspectB) / max(aspectA, aspectB)
    let score = max(0, 0.6 * ssim + 0.3 * hist + 0.1 * aspect)
    return ["score": (score * 1000).rounded() / 1000, "ssim": (ssim * 1000).rounded() / 1000,
            "hist": (hist * 1000).rounded() / 1000, "aspect": (aspect * 1000).rounded() / 1000]
}

func sideBySide(_ a: CGImage, _ b: CGImage, _ out: String, _ la: String, _ lb: String) {
    let h = max(a.height, b.height) + 28
    let w = a.width + b.width + 12
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
    ctx.setFillColor(CGColor(gray: 0.5, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
    ctx.draw(a, in: CGRect(x: 0, y: h - 28 - a.height, width: a.width, height: a.height))
    ctx.draw(b, in: CGRect(x: a.width + 12, y: h - 28 - b.height, width: b.width, height: b.height))
    let ns = NSGraphicsContext(cgContext: ctx, flipped: false)
    NSGraphicsContext.current = ns
    let style: [NSAttributedString.Key: Any] = [.font: NSFont.boldSystemFont(ofSize: 15), .foregroundColor: NSColor.white]
    (la as NSString).draw(at: NSPoint(x: 6, y: h - 22), withAttributes: style)
    (lb as NSString).draw(at: NSPoint(x: a.width + 18, y: h - 22), withAttributes: style)
    NSGraphicsContext.current = nil
    writeJPEG(ctx.makeImage()!, out)
}

// MARK: - Menu bar (static)

func menuItems(_ menu: AXUIElement, depth: Int, path: String, deep: Bool) -> [[String: Any]] {
    var out: [[String: Any]] = []
    for (i, item) in children(menu).enumerated() {
        let a = attrs(item, ["AXTitle", "AXEnabled", "AXMenuItemCmdChar", "AXMenuItemCmdModifiers", "AXMenuItemCmdVirtualKey",
                             "AXMenuItemMarkChar", "AXChildren", "AXSize"])
        let title = (a["AXTitle"] as? String) ?? ""
        var row: [String: Any] = ["p": path + "/" + String(i), "label": title]
        if let e = a["AXEnabled"] as? Bool, !e { row["disabled"] = true }
        if let c = text(a["AXMenuItemCmdChar"]) { row["key"] = c }
        if let v = a["AXMenuItemCmdVirtualKey"] as? NSNumber { row["vkey"] = v.intValue }
        if let m = a["AXMenuItemCmdModifiers"] as? NSNumber { row["mods"] = m.intValue }
        if let m = text(a["AXMenuItemMarkChar"]) { row["mark"] = m }
        if let s = size(a["AXSize"]), s.height == 0 { row["hidden"] = true }
        let kids = (a["AXChildren"] as? [AXUIElement]) ?? []
        if let sub = kids.first {
            let count = children(sub).count
            row["submenu"] = count
            let skip = ["Services", "Open Recent", "Recent Items", "Import from iPhone", "Import from iPhone or iPad"]
            if deep && depth < 4 && count <= 40 && !skip.contains(title) {
                row["items"] = menuItems(sub, depth: depth + 1, path: path + "/" + String(i) + "/0", deep: deep)
            }
        }
        out.append(row)
    }
    return out
}

// MARK: - Commands

var args = Array(CommandLine.arguments.dropFirst())
guard !args.isEmpty else { fail("usage: macax COMMAND …") }
let command = args.removeFirst()

func flag(_ name: String, _ fallback: Int) -> Int {
    if let i = args.firstIndex(of: name), i + 1 < args.count, let v = Int(args[i + 1]) { return v }
    return fallback
}

func pidArg(_ i: Int) -> pid_t {
    guard i < args.count, let p = Int32(args[i]) else { fail("missing PID") }
    return p
}

switch command {
case "trusted":
    emit(["trusted": AXIsProcessTrusted(), "screen": CGPreflightScreenCaptureAccess()])

case "apps":
    let apps = NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular || $0.bundleIdentifier == "com.apple.dock" }
    emit(apps.map { ["pid": Int($0.processIdentifier), "name": $0.localizedName ?? "", "bundle": $0.bundleIdentifier ?? "",
                     "active": $0.isActive, "hidden": $0.isHidden] })

case "pidof":
    guard args.count > 0 else { fail("pidof NAME|BUNDLE") }
    let match = NSWorkspace.shared.runningApplications.first { $0.localizedName == args[0] || $0.bundleIdentifier == args[0] }
    emit(["pid": match.map { Int($0.processIdentifier) } ?? 0])

case "frontmost":
    let f = NSWorkspace.shared.frontmostApplication
    emit(["pid": Int(f?.processIdentifier ?? 0), "name": f?.localizedName ?? "", "bundle": f?.bundleIdentifier ?? ""])

case "activate":
    let pid = pidArg(0)
    let ok = NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateAllWindows]) ?? false
    emit(["ok": ok])

case "windows":
    let pid = pidArg(0)
    let app = appElement(pid)
    let cg = cgWindows(pid: pid)
    let focused = attr(app, "AXFocusedWindow").map { $0 as! AXUIElement }
    var out: [[String: Any]] = []
    for (i, w) in windows(app).enumerated() {
        let a = attrs(w, ["AXTitle", "AXSubrole", "AXRole", "AXMain", "AXMinimized", "AXModal"])
        var row: [String: Any] = ["i": i, "title": (a["AXTitle"] as? String) ?? "", "sub": (a["AXSubrole"] as? String) ?? "",
                                  "role": (a["AXRole"] as? String) ?? "", "main": (a["AXMain"] as? Bool) ?? false,
                                  "minimized": (a["AXMinimized"] as? Bool) ?? false]
        if let f = focused, CFEqual(f, w) { row["focused"] = true }
        if let r = frame(w) {
            row["f"] = rectJSON(r)
            let fr = rectJSON(r)
            if let match = cg.first(where: { ($0["f"] as! [Int]) == fr && ($0["layer"] as! Int) == 0 }) { row["cg"] = match["id"] }
        }
        row["sheets"] = children(w).filter { (attr($0, "AXRole") as? String) == "AXSheet" }.count
        out.append(row)
    }
    emit(["windows": out, "cg": cg])

case "cgwindows":
    emit(cgWindows(pid: args.isEmpty ? nil : pidArg(0)))

case "tree":
    let pid = pidArg(0)
    guard args.count > 1 else { fail("tree PID TARGET") }
    let target = args[1]
    let app = appElement(pid)
    let maxNodes = flag("--max", 2500), maxDepth = flag("--depth", 40), rowCap = flag("--rows", 8)
    var root: AXUIElement?
    var rootPath = target
    switch target {
    case "focused":
        root = attr(app, "AXFocusedWindow").map { $0 as! AXUIElement }
        if let r = root, let i = windows(app).firstIndex(where: { CFEqual($0, r) }) { rootPath = "w\(i)" }
    case "menubar": root = attr(app, "AXMenuBar").map { $0 as! AXUIElement }; rootPath = "mb"
    case "extras": root = attr(app, "AXExtrasMenuBar").map { $0 as! AXUIElement }; rootPath = "ex"
    case "app": root = app
    default: root = resolve(pid, target)
    }
    guard let r = root else { fail("no element for \(target)") }
    let origin: CGPoint
    if args.contains("--abs") { origin = .zero } else { origin = frame(r)?.origin ?? .zero }
    let nodes = walk(r, rootPath: rootPath, origin: origin, maxNodes: maxNodes, maxDepth: maxDepth, rowCap: rowCap)
    emit(["root": rootPath, "origin": [Int(origin.x), Int(origin.y)], "nodes": nodes, "truncated": nodes.count >= maxNodes])

case "menubar":
    let pid = pidArg(0)
    let app = appElement(pid)
    let which = args.contains("--extras") ? "AXExtrasMenuBar" : "AXMenuBar"
    guard let barObj = attr(app, which) else { fail("no menu bar") }
    let bar = barObj as! AXUIElement
    var menus: [[String: Any]] = []
    let deep = args.contains("--deep")
    for (i, item) in children(bar).enumerated() {
        let a = attrs(item, ["AXTitle", "AXDescription", "AXEnabled", "AXSubrole", "AXIdentifier"])
        var row: [String: Any] = ["p": (which == "AXMenuBar" ? "mb/" : "ex/") + String(i),
                                  "label": (a["AXTitle"] as? String) ?? "", "desc": (a["AXDescription"] as? String) ?? ""]
        if let id = a["AXIdentifier"] as? String { row["id"] = id }
        if let r = frame(item) { row["f"] = rectJSON(r) }
        if let menu = children(item).first, deep {
            row["items"] = menuItems(menu, depth: 1, path: (row["p"] as! String) + "/0", deep: true)
        }
        menus.append(row)
    }
    emit(["menus": menus])

case "openmenus":
    // Menus on screen: CG windows of the app at the pop-up menu layer.
    let pid = pidArg(0)
    let cg = cgWindows(pid: pid).filter { ($0["layer"] as! Int) > 0 }
    emit(["windows": cg])

case "menuat":
    // The menu (or popover) under a screen point, with its items: used for
    // context menus, which hang off whatever element they were opened on.
    guard args.count > 1, let x = Float(args[0]), let y = Float(args[1]) else { fail("menuat X Y") }
    var hit: AXUIElement?
    AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), x, y, &hit)
    var el = hit
    var steps = 0
    while let e = el, steps < 12 {
        let role = (attr(e, "AXRole") as? String) ?? ""
        if role == "AXMenu" || role == "AXPopover" { break }
        el = attr(e, "AXParent").map { $0 as! AXUIElement }
        steps += 1
    }
    guard let menu = el, ["AXMenu", "AXPopover"].contains((attr(menu, "AXRole") as? String) ?? "") else {
        emit(["found": false, "hit": hit.map { (attr($0, "AXRole") as? String) ?? "?" } ?? "none"]); exit(0)
    }
    var pid: pid_t = 0
    AXUIElementGetPid(menu, &pid)
    let nodes = walk(menu, rootPath: "menu", origin: .zero, maxNodes: 400, maxDepth: (attr(menu, "AXRole") as? String) == "AXMenu" ? 1 : 12, rowCap: 30)
    emit(["found": true, "pid": Int(pid), "nodes": nodes])

case "press":
    let pid = pidArg(0)
    guard args.count > 1, let el = resolve(pid, args[1]) else { fail("no element") }
    let action = args.count > 2 && !args[2].hasPrefix("--") ? args[2] : "AXPress"
    let err = AXUIElementPerformAction(el, action as CFString)
    emit(["ok": err == .success, "err": err.rawValue])

case "actions":
    let pid = pidArg(0)
    guard args.count > 1, let el = resolve(pid, args[1]) else { fail("no element") }
    var names: CFArray?
    AXUIElementCopyActionNames(el, &names)
    emit(["actions": (names as? [String]) ?? []])

case "select":
    let pid = pidArg(0)
    guard args.count > 1, let el = resolve(pid, args[1]) else { fail("no element") }
    let err = AXUIElementSetAttributeValue(el, "AXSelected" as CFString, kCFBooleanTrue)
    emit(["ok": err == .success, "err": err.rawValue])

case "frame":
    let pid = pidArg(0)
    guard args.count > 1, let el = resolve(pid, args[1]) else { fail("no element") }
    guard let r = frame(el) else { fail("no frame") }
    emit(["f": rectJSON(r)])

case "key", "keydown", "keyup":
    guard args.count > 0, let code = UInt16(args[0]) else { fail("key CODE [mods]") }
    let mods = args.count > 1 ? args[1] : ""
    key(code, mods, down: command == "key" ? nil : command == "keydown")
    emit(["ok": true])

case "click":
    guard args.count > 2, let x = Double(args[1]), let y = Double(args[2]) else { fail("click BUTTON X Y") }
    click(args[0], CGPoint(x: x, y: y))
    emit(["ok": true])

case "move":
    guard args.count > 1, let x = Double(args[0]), let y = Double(args[1]) else { fail("move X Y") }
    move(CGPoint(x: x, y: y))
    emit(["ok": true])

case "pointer":
    let p = pointerLocation()
    emit(["x": p.x, "y": p.y])

case "idle":
    emit(["idle": idleSeconds()])

case "score":
    guard args.count > 1, let a = loadImage(args[0]), let b = loadImage(args[1]) else { fail("score A B") }
    emit(perceptual(a, b))

case "sbs":
    guard args.count > 2, let a = loadImage(args[0]), let b = loadImage(args[1]) else { fail("sbs A B OUT") }
    sideBySide(a, b, args[2], args.count > 3 ? args[3] : "Mac", args.count > 4 ? args[4] : "Lulo")
    emit(["ok": true])

case "shrink":
    guard args.count > 2, let image = loadImage(args[0]), let maxW = Int(args[2]) else { fail("shrink IN OUT MAXW") }
    let scale = min(1.0, Double(maxW) / Double(image.width))
    let w = max(1, Int(Double(image.width) * scale)), h = max(1, Int(Double(image.height) * scale))
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
    ctx.interpolationQuality = .high
    ctx.setFillColor(CGColor(gray: 1, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
    writeJPEG(ctx.makeImage()!, args[1])
    emit(["ok": true, "w": w, "h": h])

default:
    fail("unknown command \(command)")
}
