import AppKit
import CoreGraphics
import Vision

// What OCR read from the screenshot being checked, kept with the reason if a check fails.
var evidence: (file: String, text: String)?
func require(_ condition: Bool, _ message: String) {
    if condition { return }
    var report = "\(message)\n"
    if let evidence {
        report += evidence.text
        try? report.write(toFile: evidence.file, atomically: true, encoding: .utf8)
    }
    fputs(report, stderr); exit(1)
}
let output = CommandLine.arguments[1]
let theme = CommandLine.arguments[3]
require(["light", "dark"].contains(theme), "Expected light or dark theme")
require(NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").isEmpty,
        "Quit running Bridge before running the native walk")
func launch() throws -> NSRunningApplication {
let opened = Process()
opened.executableURL = URL(fileURLWithPath: "/usr/bin/open")
opened.arguments = [CommandLine.arguments[2]]
try opened.run()
opened.waitUntilExit()
require(opened.terminationStatus == 0, "Cannot open Bridge")
var running: NSRunningApplication?
for _ in 0..<30 {
    running = NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").first
    if running != nil { break }
    Thread.sleep(forTimeInterval: 1)
}
guard let launched = running else { fatalError("Bridge process did not appear") }
return launched
}
var app = try launch()
defer {
    app.terminate()
    for _ in 0..<30 {
        if app.isTerminated { break }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
    }
    require(app.isTerminated, "Bridge did not quit after the native walk")
}
app.activate(options: [])
func window() -> (CGWindowID, CGRect)? {
    let list = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
    guard let w = list.first(where: {
        ($0[kCGWindowOwnerPID as String] as? Int32) == app.processIdentifier &&
        ($0[kCGWindowLayer as String] as? Int) == 0
    }), let id = w[kCGWindowNumber as String] as? UInt32,
        let bounds = w[kCGWindowBounds as String] as? [String: Any],
        let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
    return (id, rect)
}
var found = false
for _ in 0..<30 {
    if window() != nil { found = true; break }
    Thread.sleep(forTimeInterval: 1)
}
require(found, "Bridge native window did not appear")
Thread.sleep(forTimeInterval: 2)
func mouse(_ type: CGEventType, _ point: CGPoint) {
    CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
}
func clickText(_ label: String, in observations: [VNRecognizedTextObservation]) {
    guard let target = observations.first(where: { $0.topCandidates(1).first?.string == label }) else {
        fatalError("Control not visible: \(label)")
    }
    let rect = window()!.1
    let point = CGPoint(x: rect.minX + target.boundingBox.midX * rect.width,
                        y: rect.minY + (1 - target.boundingBox.midY) * rect.height)
    mouse(.leftMouseDown, point); mouse(.leftMouseUp, point)
    Thread.sleep(forTimeInterval: 0.5)
}
func clickTab(_ page: String, _ observations: [VNRecognizedTextObservation]) {
    clickText(page, in: observations.filter { 1 - $0.boundingBox.maxY < 0.10 })
}
func key(_ code: CGKeyCode, shift: Bool = false) {
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
        event.flags = shift ? .maskShift : []
        event.post(tap: .cghidEventTap)
    }
    Thread.sleep(forTimeInterval: 0.3)
}
@discardableResult
func capture(_ name: String, _ page: String) throws -> [VNRecognizedTextObservation] {
    let file = "\(output)/\(name).png"
    let command = Process()
    command.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    // Exclude the window shadow so OCR coordinates are relative to the window.
    command.arguments = ["-x", "-o", "-l", String(window()!.0), file]
    try command.run(); command.waitUntilExit()
    require(command.terminationStatus == 0, "Cannot capture Bridge")
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    try VNImageRequestHandler(url: URL(fileURLWithPath: file)).perform([request])
    let observations = request.results ?? []
    // Record what OCR read before any check, so a failure shows whether the page had rendered.
    let read = observations.map {
        String(format: "%.2f ", 1 - $0.boundingBox.maxY) + ($0.topCandidates(1).first?.string ?? "")
    }
    let transcript = "\(name).png OCR (count \(read.count), top edge as a fraction of window height):\n"
        + read.map { "\($0)\n" }.joined()
    let record = "\(output)/\(name).txt"
    try transcript.write(toFile: record, atomically: true, encoding: .utf8)
    evidence = (record, transcript)
    defer { evidence = nil }
    guard let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: file))),
        let colour = bitmap.colorAt(x: bitmap.pixelsWide - 20, y: bitmap.pixelsHigh - 20)?.usingColorSpace(NSColorSpace.deviceRGB)
        else { fatalError("Cannot sample native screenshot") }
    let brightness = (colour.redComponent + colour.greenComponent + colour.blueComponent) / 3
    require(theme == "dark" ? brightness < 0.2 : brightness > 0.9,
            "Native screenshot does not show the expected \(theme) theme")
    if page == "Clusters" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        // Vision can read the capital I in AI as lowercase l. Match the rest of the heading.
        let welcome = text.contains("Build an") && text.contains("does a real job")
        let choice = text.contains("Where should your agents run")
        let selected = text.contains("Get your workspace ready")
        require(welcome || choice || selected, "Workspace heading not visible; observed: \(text)")
        if !welcome {
            require(text.contains("Workspace setup is not included"),
                    "Release setup IPC did not report its real integration state")
        }
        if ["local-selected", "choice-retained"].contains(name) {
            require(selected && !welcome && !choice, "Native preference was not acknowledged or retained")
            require(text.contains("Model connection locked"), "The model gate is not visible")
        }
        print("PASS \(name): native workspace screen observed")
        return observations
    }
    let pages = ["Overview", "Designer", "Clusters"]
    let headings = Set(observations.compactMap { observation -> String? in
        let top = 1 - observation.boundingBox.maxY
        guard top > 0.10 && top < 0.25,
            let text = observation.topCandidates(1).first?.string,
            pages.contains(text) else { return nil }
        return text
    })
    // Negative control: persistent nav labels must not qualify as page headings.
    for wrong in pages where wrong != page {
        require(!headings.contains(wrong), "Unexpected \(wrong) page heading; expected \(page)")
    }
    require(headings == Set([page]), "Native screenshot does not show the expected \(page) page heading; observed \(headings)")
    print("PASS \(name): native \(page) heading observed")
    return observations
}
try FileManager.default.createDirectory(atPath: output, withIntermediateDirectories: true)
var initial = try capture("initial", "Clusters")
// CI's light pass owns a fresh profile; dark reuses the saved native choice.
let initiallyWelcome = initial.contains { $0.topCandidates(1).first?.string == "Get started" }
require(theme == "light" ? initiallyWelcome : !initiallyWelcome,
        "Expected a first-run welcome in light and a retained choice in dark")
if initial.contains(where: { $0.topCandidates(1).first?.string == "Get started" }) {
    clickText("Get started", in: initial)
    initial = try capture("chooser", "Clusters")
}
if initial.contains(where: { $0.topCandidates(1).first?.string == "Continue" }) {
    // Selecting the default radio does not start anything; Continue is explicit.
    clickText("Continue", in: initial)
}
try capture("local-selected", "Clusters")
app.terminate()
for _ in 0..<30 {
    if app.isTerminated { break }
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
}
require(app.isTerminated, "Bridge did not quit before preference check")
app = try launch()
app.activate(options: [])
for _ in 0..<30 {
    if window() != nil { break }
    Thread.sleep(forTimeInterval: 1)
}
require(window() != nil, "Relaunched Bridge window did not appear")
Thread.sleep(forTimeInterval: 2)
let retained = try capture("choice-retained", "Clusters")
clickTab("Overview", retained)
let overview = try capture("overview", "Overview")
clickTab("Designer", overview)
let designer = try capture("designer-pointer", "Designer")
clickTab("Clusters", designer)
try capture("clusters-pointer", "Clusters")
// The keyboard-reachable Search explanation adds a stop after navigation.
key(48, shift: true); key(48, shift: true); key(48, shift: true); key(49)
try capture("overview-keyboard", "Overview")
key(48); key(49); try capture("designer-keyboard", "Designer")
key(48); key(36); try capture("clusters-keyboard", "Clusters")
// Resize this CI-owned window using its native bottom-right handle.
let rect = window()!.1
// The display may constrain the initial size. Derive the minimum frame from
// AppKit's content-to-frame conversion, not the requested initial dimensions.
let minimumFrame = NSWindow.frameRect(
    forContentRect: CGRect(x: 0, y: 0, width: 960, height: 640),
    styleMask: [.titled, .closable, .miniaturizable, .resizable])
let start = CGPoint(x: rect.maxX - 10, y: rect.maxY - 5)
let end = CGPoint(x: rect.minX + minimumFrame.width - 10, y: rect.minY + minimumFrame.height - 5)
mouse(.leftMouseDown, start)
for step in 1...20 {
    let t = CGFloat(step) / 20
    mouse(.leftMouseDragged, CGPoint(x: start.x + (end.x - start.x) * t, y: start.y + (end.y - start.y) * t))
    Thread.sleep(forTimeInterval: 0.02)
}
mouse(.leftMouseUp, end)
Thread.sleep(forTimeInterval: 0.5)
let minimum = window()!.1
require(abs(minimum.width - minimumFrame.width) < 3 && abs(minimum.height - minimumFrame.height) < 3, "Native minimum-size resize not verified: \(minimum), expected \(minimumFrame.size)")
try capture("clusters-minimum", "Clusters")
print("PASS native walk: pointer, Shift-Tab/Tab, Space, Enter and minimum window")
