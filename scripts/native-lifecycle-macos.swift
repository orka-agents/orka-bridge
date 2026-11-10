// CI-only integrated debug shell. No workspace selection, VM, credentials or injected Ready state.
import AppKit
import ApplicationServices
import Vision

let args = CommandLine.arguments
var app: NSRunningApplication?
defer {
    if let app, !app.isTerminated {
        fputs("Walk failed before native quit; forcing only the CI-owned process down.\n", stderr)
        app.forceTerminate()
    }
}
func check(_ value: Bool, _ message: String) {
    if !value {
        fputs("FAIL: \(message)\n", stderr)
        // Only this walk's process, after an assertion failure; never counted as a clean stop.
        app?.forceTerminate()
        exit(1)
    }
}
check(args.count == 4, "Usage: native-lifecycle-macos.swift OUTPUT DEBUG_APP light|dark")
let (output, bundle, theme) = (args[1], args[2], args[3])
let env = ProcessInfo.processInfo.environment
check(env["GITHUB_ACTIONS"] == "true" && ["light", "dark"].contains(theme), "Hosted CI walk only")
let home = URL(fileURLWithPath: env["HOME"]!)
let data = home.appendingPathComponent("Library/Application Support/ai.orka.bridge")
let selected = data.appendingPathComponent("setup-v1/local-vm").path
check(!FileManager.default.fileExists(atPath: selected), "Existing saved local selection; will not launch or erase it")
check(!FileManager.default.fileExists(atPath: data.appendingPathComponent("local-vm-v1").path), "Existing VM data; refusing walk")
if theme == "light" { check(!FileManager.default.fileExists(atPath: data.path), "Expected fresh CI application profile") }
let source = env["BRIDGE_DEV_IMAGE_DIR"] ?? ""
check(!source.isEmpty, "Expected empty source directory")
check(try FileManager.default.contentsOfDirectory(atPath: source).isEmpty, "Expected empty source directory")
check(AXIsProcessTrusted(), "CI runner needs Accessibility permission for real native UI actions")
try FileManager.default.createDirectory(atPath: output, withIntermediateDirectories: true)
func wait(_ message: String, _ predicate: () -> Bool) {
    for _ in 0..<100 {
        if predicate() { return }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
    }
    check(false, message)
}
@discardableResult
func command(_ executable: String, _ arguments: [String]) throws -> Process {
    let p = Process(); p.executableURL = URL(fileURLWithPath: executable); p.arguments = arguments
    try p.run()
    wait("Command timed out: \(executable)") { !p.isRunning }
    check(p.terminationStatus == 0, "Command failed: \(executable)")
    return p
}
func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var result: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &result) == .success ? result : nil
}
func nodes(_ root: AXUIElement) -> [AXUIElement] {
    var all = [root]; var i = 0
    // Bounded AX traversal, including native status-item menus rather than renderer hooks.
    while i < all.count && all.count < 4000 {
        let element = all[i]; i += 1
        for key in ["AXChildren", "AXWindows"] {
            for child in attribute(element, key) as? [AXUIElement] ?? [] where !all.contains(where: { CFEqual($0, child) }) { all.append(child) }
        }
        for key in ["AXMenuBar", "AXExtrasMenuBar", "AXShownMenuUIElement"] {
            if let value = attribute(element, key), CFGetTypeID(value) == AXUIElementGetTypeID() {
                let child = value as! AXUIElement
                if !all.contains(where: { CFEqual($0, child) }) { all.append(child) }
            }
        }
    }
    return all
}
func text(_ element: AXUIElement) -> String {
    ["AXTitle", "AXDescription", "AXHelp"].compactMap { attribute(element, $0) as? String }.joined(separator: " | ")
}
func find(_ root: AXUIElement, _ predicate: (AXUIElement) -> Bool) -> AXUIElement {
    var result: AXUIElement?
    wait("Native AX control missing") { result = nodes(root).first(where: predicate); return result != nil }
    return result!
}
func press(_ element: AXUIElement) {
    check(AXUIElementPerformAction(element, "AXPress" as CFString) == .success, "Cannot activate native AX control: \(text(element))")
}
func windowInfo() -> (CGWindowID, CGRect)? {
    let list = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
    guard let w = list.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == app?.processIdentifier && ($0[kCGWindowLayer as String] as? Int) == 0 }),
        let id = w[kCGWindowNumber as String] as? UInt32,
        let bounds = w[kCGWindowBounds as String] as? [String: Any],
        let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
    return (id, rect)
}
func capture(_ name: String, menu: Bool = false) throws {
    let file = "\(output)/\(name).png"
    let options = menu ? ["-x", file] : ["-x", "-o", "-l", String(windowInfo()!.0), file]
    try command("/usr/sbin/screencapture", options)
    let request = VNRecognizeTextRequest(); request.recognitionLevel = .accurate
    try VNImageRequestHandler(url: URL(fileURLWithPath: file)).perform([request])
    let seen = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    if menu {
        check(seen.contains("Open Bridge") && seen.contains("Stop workspace and quit"), "Native menu not drawn in screenshot: \(seen)")
    } else {
        check(seen.contains("web-reader.agent.md"), "Reopened window lost File editor view: \(seen)")
        guard let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: file))),
            let colour = bitmap.colorAt(x: bitmap.pixelsWide - 20, y: bitmap.pixelsHigh - 20)?.usingColorSpace(.deviceRGB) else { fatalError("Cannot sample screenshot") }
        let brightness = (colour.redComponent + colour.greenComponent + colour.blueComponent) / 3
        check(theme == "dark" ? brightness < 0.2 : brightness > 0.9, "Window did not follow requested system theme")
    }
}
func launch() throws -> AXUIElement {
    check(!FileManager.default.fileExists(atPath: selected) && !FileManager.default.fileExists(atPath: data.appendingPathComponent("local-vm-v1").path), "Saved selection or VM data appeared; refusing to resume")
    check(NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").isEmpty, "Bridge is already running")
    // open supplies the override explicitly; release smoke uses its unchanged environment.
    try command("/usr/bin/open", [bundle, "--env", "BRIDGE_DEV_IMAGE_DIR=\(source)"])
    wait("Bridge did not launch") {
        app = NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").first
        return app != nil
    }
    app!.activate(options: [])
    wait("Main window did not appear") { windowInfo() != nil }
    return AXUIElementCreateApplication(app!.processIdentifier)
}
func menu(_ root: AXUIElement) -> [AXUIElement] {
    guard let extras = attribute(root, "AXExtrasMenuBar") else { fatalError("Missing native status menu bar") }
    let item = find(extras as! AXUIElement) { attribute($0, "AXRole") as? String == "AXMenuBarItem" && text($0).contains("Bridge") }
    press(item)
    let open = find(root) { attribute($0, "AXTitle") as? String == "Open Bridge" }
    let stop = find(root) { attribute($0, "AXTitle") as? String == "Stop workspace and quit" }
    check(attribute(open, "AXEnabled") as? Bool == true && attribute(stop, "AXEnabled") as? Bool == true, "Native menu commands disabled")
    return [open, stop]
}
func key(_ code: CGKeyCode, flags: CGEventFlags = []) {
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
        event.flags = flags; event.post(tap: .cghidEventTap)
    }
}
let root = try launch()
let pid = app!.processIdentifier
let originalID = windowInfo()!.0
let main = find(root) { attribute($0, "AXRole") as? String == "AXWindow" }
press(find(main) { element in attribute(element, "AXRole") as? String == "AXButton" && ["AXTitle", "AXDescription"].contains(where: { attribute(element, $0) as? String == "Designer" }) })
press(find(main) { element in attribute(element, "AXRole") as? String == "AXButton" && ["AXTitle", "AXDescription"].contains(where: { attribute(element, $0) as? String == "File" }) })
let editor = find(main) { attribute($0, "AXRole") as? String == "AXTextArea" }
check(AXUIElementSetAttributeValue(editor, "AXFocused" as CFString, kCFBooleanTrue) == .success, "Cannot focus real editor")
key(125, flags: .maskCommand)
let marker = " lifecycle-\(UUID().uuidString)"
for character in marker.utf16 {
    var unit = character
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
        event.flags = []; event.keyboardSetUnicodeString(stringLength: 1, unicodeString: &unit); event.post(tap: .cghidEventTap)
    }
}
let owned = home.appendingPathComponent("Orka/agents/web-reader/web-reader.agent.md")
wait("Native editor change not saved") { (try? String(contentsOf: owned, encoding: .utf8))?.contains(marker) == true }
func retained() {
    check(app!.processIdentifier == pid && !app!.isTerminated, "Original process lost")
    check(windowInfo()?.0 == originalID, "Main native window replaced")
    check(CFEqual(find(root) { attribute($0, "AXRole") as? String == "AXWindow" }, main), "Main AX window replaced")
    check(attribute(editor, "AXValue") as? String != nil, "Editor AX element lost")
    check((attribute(find(main) { attribute($0, "AXRole") as? String == "AXTextArea" }, "AXValue") as? String)?.contains(marker) == true, "File view/draft lost on reopen")
    check(NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, "Reopen did not focus original process")
    check(!FileManager.default.fileExists(atPath: selected), "Walk accidentally selected a workspace")
}
guard let close = attribute(main, "AXCloseButton") else { fatalError("Missing native Close button") }
press(close as! AXUIElement)
wait("Native Close did not hide") { windowInfo() == nil }
check(!app!.isTerminated, "Close quit instead of hiding")
let commands = menu(root)
try capture("menu-hidden", menu: true)
press(commands[0])
wait("Open Bridge did not reopen/focus") { windowInfo() != nil && NSWorkspace.shared.frontmostApplication?.processIdentifier == pid }
retained(); try capture("reopened-menu")
press(close as! AXUIElement)
wait("Close before second launch did not hide") { windowInfo() == nil }
try command("\(bundle)/Contents/MacOS/orka-bridge", [])
wait("Second launch did not show/focus hidden window") { windowInfo() != nil && NSWorkspace.shared.frontmostApplication?.processIdentifier == pid }
retained(); try capture("reopened-second-launch-hidden")
check(AXUIElementSetAttributeValue(main, "AXMinimized" as CFString, kCFBooleanTrue) == .success, "Cannot minimize main window")
wait("Window did not minimize") { attribute(main, "AXMinimized") as? Bool == true }
try command("\(bundle)/Contents/MacOS/orka-bridge", [])
wait("Second executable launch did not unminimize/focus") { attribute(main, "AXMinimized") as? Bool == false && NSWorkspace.shared.frontmostApplication?.processIdentifier == pid }
retained(); try capture("reopened-second-launch-minimized")
check(NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").map(\.processIdentifier) == [pid], "Second Bridge instance survived")
let stop = menu(root)[1]
try capture("menu-stop", menu: true)
press(stop)
wait("Stop workspace and quit did not exit") { app!.isTerminated }
// A separate launch proves standard AppKit Quit, not a synthetic terminate selector.
let quitRoot = try launch()
guard let appMenu = attribute(quitRoot, "AXMenuBar") else { fatalError("Missing native application menu bar") }
press(find(appMenu as! AXUIElement) { attribute($0, "AXRole") as? String == "AXMenuBarItem" && attribute($0, "AXTitle") as? String == "Bridge" })
press(find(quitRoot) { attribute($0, "AXTitle") as? String == "Quit Bridge" })
wait("Standard Quit did not exit") { app!.isTerminated }
check(!FileManager.default.fileExists(atPath: selected) && !FileManager.default.fileExists(atPath: data.appendingPathComponent("local-vm-v1").path), "Saved local selection or VM data appeared")
let evidence = "PASS \(theme): native Close/hide, tray menu/Open, same PID \(pid)/window \(originalID), File view and edited text retained, hidden/minimized second launches, menu stop, standard Quit. Integrated debug shell only; empty image source, no VM/Ready/provider/cluster/session logout proof.\n"
try evidence.write(toFile: "\(output)/evidence.txt", atomically: true, encoding: .utf8)
print(evidence)
