// Exercise the real compiled frontend in the separate synthetic native fixture.
// This is UI evidence only, not a VM, OS credential-store or inference walk.
import AppKit
import CoreGraphics
import ApplicationServices
import Vision

let display = CGMainDisplayID()
var previousDisplayMode: CGDisplayMode?
func restoreDisplay() {
    if let previous = previousDisplayMode {
        previousDisplayMode = nil
        if CGDisplaySetDisplayMode(display, previous, nil) != .success {
            fputs("Cannot restore CI display mode\n", stderr)
        }
    }
}
func require(_ value: Bool, _ message: String) {
    if !value { fputs("\(message)\n", stderr); restoreDisplay(); exit(1) }
}
// The preceding native walk restores the hosted Mac's small default display.
// Enlarge only that disposable CI display for the local 540x900 result captures.
if CGDisplayBounds(display).width < 1280 || CGDisplayBounds(display).height < 1080 {
    require(ProcessInfo.processInfo.environment["GITHUB_ACTIONS"] == "true", "Local result captures need a 1280x1080 CI display")
    let options = [kCGDisplayShowDuplicateLowResolutionModes: kCFBooleanTrue] as CFDictionary
    let modes = CGDisplayCopyAllDisplayModes(display, options) as? [CGDisplayMode] ?? []
    guard let previous = CGDisplayCopyDisplayMode(display),
        let larger = modes.filter({ $0.isUsableForDesktopGUI() && $0.width >= 1280 && $0.height >= 1080 })
            .min(by: { $0.width * $0.height < $1.width * $1.height }) else { fatalError("No suitable CI display mode") }
    previousDisplayMode = previous
    require(CGDisplaySetDisplayMode(display, larger, nil) == .success, "Cannot enlarge CI display")
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 1))
}
defer { restoreDisplay() }
let output = CommandLine.arguments[1]
let theme = CommandLine.arguments[3]
require(["light", "dark"].contains(theme), "Expected light or dark")
try FileManager.default.createDirectory(atPath: output, withIntermediateDirectories: true)
let child = Process()
child.executableURL = URL(fileURLWithPath: CommandLine.arguments[2])
try child.run()
defer { if child.isRunning { child.terminate(); child.waitUntilExit() } }
func window() -> (CGWindowID, CGRect)? {
    let list = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
    guard let w = list.first(where: {
        ($0[kCGWindowOwnerPID as String] as? Int32) == child.processIdentifier &&
        ($0[kCGWindowLayer as String] as? Int) == 0
    }), let id = w[kCGWindowNumber as String] as? UInt32,
        let bounds = w[kCGWindowBounds as String] as? [String: Any],
        let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
    return (id, rect)
}
for _ in 0..<30 {
    if window() != nil { break }
    Thread.sleep(forTimeInterval: 1)
}
require(window() != nil, "Native model fixture did not open")
NSRunningApplication(processIdentifier: child.processIdentifier)?.activate(options: [])
Thread.sleep(forTimeInterval: 2)
func capture(_ name: String, _ expected: [String]) throws -> [VNRecognizedTextObservation] {
    let file = "\(output)/\(name).png"
    let command = Process()
    command.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    command.arguments = ["-x", "-o", "-l", String(window()!.0), file]
    try command.run(); command.waitUntilExit()
    require(command.terminationStatus == 0, "Cannot capture native model UI")
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    try VNImageRequestHandler(url: URL(fileURLWithPath: file)).perform([request])
    let rows = request.results ?? []
    let text = rows.compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
    try text.write(toFile: "\(output)/\(name).txt", atomically: true, encoding: .utf8)
    // Vision can interleave the adjacent setup column between wrapped lines.
    // Check phrases within the model panel, not the whole window's reading order.
    let visible: String
    if name.hasPrefix("local-") {
        visible = rows.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    } else {
        guard let panel = rows.first(where: { $0.topCandidates(1).first?.string == "Connect a model" })
            else { fatalError("Model panel heading was not recognized") }
        visible = rows.filter {
            $0.boundingBox.minX >= panel.boundingBox.minX - 0.02 &&
            $0.boundingBox.maxY <= panel.boundingBox.maxY + 0.01
        }.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    }
    for value in expected {
        let found = visible.contains(value)
        let nativeFailure = !found && name.hasPrefix("local-") && nativeFailureTextIsVisible(value)
        require(found || nativeFailure, "Missing \(value) in \(name): \(text)")
    }
    guard let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: file))),
          let colour = bitmap.colorAt(x: bitmap.pixelsWide - 20, y: bitmap.pixelsHigh - 20)?.usingColorSpace(.deviceRGB)
        else { fatalError("Cannot sample native theme") }
    let brightness = (colour.redComponent + colour.greenComponent + colour.blueComponent) / 3
    require(theme == "dark" ? brightness < 0.25 : brightness > 0.8, "Wrong system theme")
    print("PASS \(name): real native frontend, synthetic IPC only")
    return rows
}
func enabledButton(_ label: String) -> AXUIElement? {
    let buttons = nativeElements().filter { element in
        attribute(element, "AXRole") as? String == "AXButton"
            && attribute(element, "AXEnabled") as? Bool == true
            && ["AXTitle", "AXDescription"].contains(where: { attribute(element, $0) as? String == label })
    }
    return buttons.count == 1 ? buttons[0] : nil
}
func click(_ label: String, _ rows: [VNRecognizedTextObservation]) {
    guard let button = enabledButton(label) else { require(false, "Native button is disabled or missing: \(label)"); return }
    let rect = window()!.1
    let p: CGPoint
    if let row = rows.first(where: { $0.topCandidates(1).first?.string == label }) {
        p = CGPoint(x: rect.minX + row.boundingBox.midX * rect.width,
                    y: rect.minY + (1 - row.boundingBox.midY) * rect.height)
    } else {
        // Vision can join Connected and Remove sign-in on one line. Still
        // pointer-click the exact enabled native button, never the joined row.
        guard let bounds = nativeBounds(button), rect.contains(bounds) else {
            require(false, "Native button is not visible: \(label)"); return
        }
        p = CGPoint(x: bounds.midX, y: bounds.midY)
    }
    for type in [CGEventType.leftMouseDown, .leftMouseUp] {
        CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
    }
    Thread.sleep(forTimeInterval: 1)
}
var rows = try capture("before-connect", ["Use GitHub sign-in", "Remove sign-in"])
click("Use GitHub sign-in", rows)
rows = try capture("connected", ["Model connected", "Remove sign-in"])
click("Remove sign-in", rows)
rows = try capture("remove-failed", ["Remove sign-in again", "Copy details for help"])
click("Remove sign-in", rows)
rows = try capture("removed", ["no longer has a saved sign-in", "GitHub CLI stays signed in", "Use GitHub sign-in"])
click("Use GitHub sign-in", rows)
_ = try capture("reconnected", ["Model connected", "Remove sign-in"])
func scroll(_ pixels: Int32) {
    let rect = window()!.1
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: CGPoint(x: rect.maxX - 20, y: rect.midY), mouseButton: .left)?.post(tap: .cghidEventTap)
    CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: pixels, wheel2: 0, wheel3: 0)?.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.3)
}
func seek(_ label: String, _ pixels: Int32) throws -> [VNRecognizedTextObservation] {
    for _ in 0..<6 {
        let seen = try capture("local-probe", [])
        if seen.contains(where: { ($0.topCandidates(1).first?.string ?? "").contains(label) }) {
            if label == "Run tests" && enabledButton(label) == nil { Thread.sleep(forTimeInterval: 0.3); continue }
            return seen
        }
        scroll(pixels)
    }
    fatalError("Native local result control missing: \(label)")
}
func localSize(_ width: CGFloat, _ height: CGFloat) {
    let app = AXUIElementCreateApplication(child.processIdentifier)
    var windows: CFTypeRef?
    require(AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &windows) == .success, "Cannot read fixture window")
    let native = (windows as! [AXUIElement])[0]
    var origin = CGPoint(x: 0, y: 40)
    require(AXUIElementSetAttributeValue(native, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &origin)!) == .success, "Cannot position fixture window")
    var size = NSWindow.frameRect(forContentRect: CGRect(x: 0, y: 0, width: width, height: height), styleMask: [.titled, .closable, .miniaturizable, .resizable]).size
    require(AXUIElementSetAttributeValue(native, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!) == .success, "Cannot resize fixture window")
    Thread.sleep(forTimeInterval: 0.3)
    require(abs(window()!.1.width - size.width) < 3 && abs(window()!.1.height - size.height) < 3, "Fixture content size not applied")
}
// Synthetic IPC renders the real local result component; no VM or inference.
rows = try seek("Open example", -200)
click("Open example", rows)
rows = try seek("Tests on local", -250)
_ = try capture("local-result-wide", ["Tests on local", "1 of 2 passed"])
_ = try seek("The answer did not include", -150)
_ = try capture("local-answer-wide", ["Which date is shown", "No date was given", "2099-12-31"])
localSize(540, 900)
scroll(4000)
_ = try seek("Tests on local", -250)
_ = try capture("local-result", ["Tests on local", "1 of 2 passed"])
_ = try seek("The answer did not include", -150)
_ = try capture("local-answer", ["Which date is shown", "No date was given", "2099-12-31"])
// Edit the fixture's Instructions field through the native text control. Its
// writer is in-memory only, so no creator file or credential can be changed.
scroll(4000)
func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}
func nativeElements() -> [AXUIElement] {
    var elements = [AXUIElementCreateApplication(child.processIdentifier)]
    var index = 0
    while index < elements.count && elements.count < 4000 {
        let element = elements[index]; index += 1
        for key in ["AXChildren", "AXWindows"] {
            for item in attribute(element, key) as? [AXUIElement] ?? []
                where !elements.contains(where: { CFEqual($0, item) }) { elements.append(item) }
        }
    }
    return elements
}
func nativeBounds(_ element: AXUIElement) -> CGRect? {
    guard let position = attribute(element, "AXPosition"), CFGetTypeID(position) == AXValueGetTypeID(),
        let size = attribute(element, "AXSize"), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
    var origin = CGPoint.zero; var extent = CGSize.zero
    guard AXValueGetValue(position as! AXValue, .cgPoint, &origin)
        && AXValueGetValue(size as! AXValue, .cgSize, &extent)
        && extent.width > 0 && extent.height > 0 else { return nil }
    return CGRect(origin: origin, size: extent)
}
// Vision can read 2099 as 9099 in small Inter text. Require the exact rendered
// failure text through native accessibility, fully within this window, instead
// of accepting either number or weakening the expected value.
func nativeFailureTextIsVisible(_ expected: String) -> Bool {
    nativeElements().contains { element in
        guard attribute(element, "AXRole") as? String == "AXStaticText",
            let text = attribute(element, "AXValue") as? String,
            text.hasPrefix("The answer did not include"), text.contains(expected),
            let bounds = nativeBounds(element) else { return false }
        return window()!.1.contains(bounds)
    }
}
// Match the fixture's known Instructions value, not a label plus a guessed
// offset which can land outside the editor after responsive reflow.
let editors = nativeElements().filter {
    attribute($0, "AXRole") as? String == "AXTextArea"
        && (attribute($0, "AXValue") as? String)?.hasPrefix("Answer from the web page named in the request.") == true
}
require(editors.count == 1, "Expected the fixture Instructions editor")
let editor = editors[0]
require(AXUIElementSetAttributeValue(editor, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "Cannot focus fixture Instructions")
Thread.sleep(forTimeInterval: 0.3)
for down in [true, false] {
    let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
    event.flags = .maskCommand; event.post(tap: .cghidEventTap)
}
for character in "Changed fixture instructions.".utf16 {
    var unit = character
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
        event.keyboardSetUnicodeString(stringLength: 1, unicodeString: &unit)
        event.flags = []; event.post(tap: .cghidEventTap)
    }
}
Thread.sleep(forTimeInterval: 1)
require((attribute(editor, "AXValue") as? String)?.contains("Changed fixture instructions.") == true, "Native keystrokes did not edit Instructions")
_ = try seek("Not run since your edit", -250)
_ = try capture("local-stale", ["Not run since your edit"])
rows = try seek("Run tests", -150)
click("Run tests", rows)
_ = try seek("Waiting for answers", -150)
_ = try capture("local-running", ["Waiting for answers", "Observed for"])
localSize(1280, 800)
scroll(4000)
_ = try seek("Waiting for answers", -250)
_ = try capture("local-running-wide", ["Waiting for answers", "Observed for"])
