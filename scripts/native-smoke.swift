import AppKit
import CoreGraphics
import ApplicationServices
import Vision
import ImageIO

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
var restoreTemporaryTests: (() throws -> Void)?
func fail(_ message: String) -> Never {
    fputs("\(message)\n", stderr)
    do { try restoreTemporaryTests?() }
    catch { fputs("Cannot restore original test file: \(error)\n", stderr) }
    restoreDisplay()
    exit(1)
}
func require(_ condition: Bool, _ message: String) {
    if !condition { fail(message) }
}
let output = CommandLine.arguments[1]
let theme = CommandLine.arguments[3]
require(["light", "dark"].contains(theme), "Expected light or dark theme")
require(NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").isEmpty,
        "Quit running Bridge before running the native walk")
// Hosted Macs default to 1024x768, which constrains a 900pt content window.
// Only reconfigure the disposable CI display, never an operator's desktop.
if CGDisplayBounds(display).width < 1280 || CGDisplayBounds(display).height < 1080 {
    require(ProcessInfo.processInfo.environment["GITHUB_ACTIONS"] == "true",
            "The native walk needs a display of at least 1280x1080 logical points")
    let options = [kCGDisplayShowDuplicateLowResolutionModes: kCFBooleanTrue] as CFDictionary
    let modes = CGDisplayCopyAllDisplayModes(display, options) as? [CGDisplayMode] ?? []
    print("CI display modes: \(modes.map { "\($0.width)x\($0.height)" })")
    guard let previous = CGDisplayCopyDisplayMode(display),
        let larger = modes.filter({ $0.isUsableForDesktopGUI() && $0.width >= 1280 && $0.height >= 1080 })
            .min(by: { $0.width * $0.height < $1.width * $1.height }) else {
        fail("No CI display mode can fit the required native window")
    }
    previousDisplayMode = previous
    require(CGDisplaySetDisplayMode(display, larger, nil) == .success, "Cannot enlarge CI display")
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 1))
    require(CGDisplayBounds(display).width >= 1280 && CGDisplayBounds(display).height >= 1080,
            "CI display mode did not take effect: \(CGDisplayBounds(display))")
}
defer { restoreDisplay() }
func launch() throws -> NSRunningApplication {
let opened = Process()
opened.executableURL = URL(fileURLWithPath: "/usr/bin/open")
var arguments = [CommandLine.arguments[2]]
if let kubeconfig = ProcessInfo.processInfo.environment["BRIDGE_WALK_KUBECONFIG"] {
    arguments += ["--env", "KUBECONFIG=\(kubeconfig)"]
}
opened.arguments = arguments
try opened.run()
opened.waitUntilExit()
require(opened.terminationStatus == 0, "Cannot open Bridge")
var running: NSRunningApplication?
for _ in 0..<30 {
    running = NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").first
    if running != nil { break }
    Thread.sleep(forTimeInterval: 1)
}
guard let launched = running else { fail("Bridge process did not appear") }
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
        // Preserve the label's actual pointer action when small dark text is
        // unreadable to Vision; require one exact, fully visible native label.
        let labels = nativeElements().filter {
            attribute($0, "AXRole") as? String == "AXStaticText"
                && attribute($0, "AXValue") as? String == label
        }.compactMap { nativeBounds($0) }.filter { window()!.1.contains($0) }
        require(!labels.isEmpty, "No visible native label: \(label)")
        // WebKit can expose both a label and its text node. They must refer to
        // the same on-screen hit region; distinct/ambiguous labels still fail.
        let hit = labels.dropFirst().reduce(labels[0]) { $0.intersection($1) }
        require(!hit.isNull && !hit.isEmpty, "Ambiguous native label \(label): \(labels)")
        print("Native label \(label): \(labels), hit \(hit)")
        let point = CGPoint(x: hit.midX, y: hit.midY)
        mouse(.leftMouseDown, point); mouse(.leftMouseUp, point)
        Thread.sleep(forTimeInterval: 0.5)
        return
    }
    let rect = window()!.1
    let point = CGPoint(x: rect.minX + target.boundingBox.midX * rect.width,
                        y: rect.minY + (1 - target.boundingBox.midY) * rect.height)
    mouse(.leftMouseDown, point); mouse(.leftMouseUp, point)
    Thread.sleep(forTimeInterval: 0.5)
}
func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}
func nativeElements() -> [AXUIElement] {
    var nodes = [AXUIElementCreateApplication(app.processIdentifier)]
    var index = 0
    while index < nodes.count && nodes.count < 4000 {
        let element = nodes[index]; index += 1
        for key in ["AXChildren", "AXWindows"] {
            for child in attribute(element, key) as? [AXUIElement] ?? []
                where !nodes.contains(where: { CFEqual($0, child) }) { nodes.append(child) }
        }
    }
    return nodes
}
func nativeBounds(_ element: AXUIElement) -> CGRect? {
    guard let position = attribute(element, "AXPosition"), CFGetTypeID(position) == AXValueGetTypeID(),
        let size = attribute(element, "AXSize"), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero; var extent = CGSize.zero
    guard AXValueGetValue(position as! AXValue, .cgPoint, &point),
        AXValueGetValue(size as! AXValue, .cgSize, &extent), extent.width > 0, extent.height > 0 else { return nil }
    return CGRect(origin: point, size: extent)
}
func clickDesignerView(_ name: String) {
    let matches = nativeElements().filter { element in
        ["AXTitle", "AXDescription"].contains { attribute(element, $0) as? String == name }
            && attribute(element, "AXEnabled") as? Bool == true
            && (attribute(element, "AXRole") as? String == "AXCheckBox" || attribute(element, "AXRole") as? String == "AXButton")
    }.compactMap { nativeBounds($0) }.filter { window()!.1.contains($0) }
    require(matches.count == 1, "Cannot identify the visible Designer \(name) control")
    let point = CGPoint(x: matches[0].midX, y: matches[0].midY)
    mouse(.leftMouseDown, point); mouse(.leftMouseUp, point)
    Thread.sleep(forTimeInterval: 0.3)
}
func nativeTextIsVisible(_ expected: String) -> Bool {
    nativeElements().contains { element in
        guard attribute(element, "AXRole") as? String == "AXStaticText",
            (attribute(element, "AXValue") as? String)?.contains(expected) == true,
            let bounds = nativeBounds(element), let rect = window()?.1 else { return false }
        return rect.contains(bounds)
    }
}
@discardableResult
func clickTab(_ page: String, _ observations: [VNRecognizedTextObservation]) -> AXUIElement {
    // Small Inter labels can be grouped with subtitles by Vision. Identify the
    // real enabled native button, but still pointer-click it and verify the
    // rendered destination in the next capture (not AXPress or renderer hooks).
    let header = observations.filter { 1 - $0.boundingBox.maxY < 0.10 }
        .compactMap { $0.topCandidates(1).first?.string }
    print("NAV \(page): header OCR \(header)")
    let matches = nativeElements().filter { element in
        attribute(element, "AXRole") as? String == "AXButton"
            && attribute(element, "AXEnabled") as? Bool == true
            && ["AXTitle", "AXDescription"].contains(where: { attribute(element, $0) as? String == page })
    }
    require(matches.count == 1, "Expected one enabled native tab named \(page), found \(matches.count)")
    let target = matches[0]
    guard let position = attribute(target, "AXPosition"), CFGetTypeID(position) == AXValueGetTypeID(),
        let size = attribute(target, "AXSize"), CFGetTypeID(size) == AXValueGetTypeID() else {
        fail("Native tab has no bounds: \(page)")
    }
    var origin = CGPoint.zero; var extent = CGSize.zero
    require(AXValueGetValue(position as! AXValue, .cgPoint, &origin)
        && AXValueGetValue(size as! AXValue, .cgSize, &extent), "Cannot read native tab bounds")
    let rect = window()!.1
    let bounds = CGRect(origin: origin, size: extent)
    require(extent.width > 0 && extent.height > 0 && rect.contains(bounds)
        && bounds.maxY <= rect.minY + 110, "Native tab is not visible in the header: \(page), \(bounds)")
    let point = CGPoint(x: bounds.midX, y: bounds.midY)
    mouse(.leftMouseDown, point); mouse(.leftMouseUp, point)
    Thread.sleep(forTimeInterval: 0.5)
    return target
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
    // Render the PNG's profile into sRGB bytes; NSColor calibration changed the sampled values.
    guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: file) as CFURL, nil),
        let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
        let pixel = image.cropping(to: CGRect(x: CGFloat(image.width - 8), y: CGFloat(image.height - 8), width: 1, height: 1)),
        let space = CGColorSpace(name: CGColorSpace.sRGB) else { fail("Cannot decode screenshot") }
    var rgba = [UInt8](repeating: 0, count: 4)
    let sampled = rgba.withUnsafeMutableBytes { bytes in
        guard let context = CGContext(data: bytes.baseAddress, width: 1, height: 1,
            bitsPerComponent: 8, bytesPerRow: 4, space: space,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)
            else { return false }
        context.draw(pixel, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        return true
    }
    require(sampled, "Cannot sample screenshot in sRGB")
    let expected: [CGFloat] = theme == "dark" ? [26, 29, 33] : [228, 226, 219]
    let actual = rgba.prefix(3).map { CGFloat($0) }
    require(zip(actual, expected).allSatisfy { abs($0 - $1) < 8 },
            "Native screenshot does not show the expected \(theme) Estuary chrome: \(actual)")
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    try VNImageRequestHandler(url: URL(fileURLWithPath: file)).perform([request])
    let observations = request.results ?? []
    let recognized = observations.map { observation in
        "\(observation.boundingBox): \(observation.topCandidates(1).first?.string ?? "")"
    }.joined(separator: "\n")
    try recognized.write(toFile: "\(output)/\(name).ocr.txt", atomically: true, encoding: .utf8)
    if name == "send-probe" || name == "cluster-probe" { return observations }
    if name == "before-settings" || name == "designer-settings" {
        // These captures intentionally scroll the page heading out of view.
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(name == "before-settings" ? text.contains("More settings") : text.contains("Personality") && text.contains("Description"),
                "Designer settings section is not visible: \(text)")
        print("PASS \(name): native Designer settings section observed")
        return observations
    }
    if name.hasPrefix("cluster-reload") {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        if name == "cluster-reload-failed" {
            require(text.contains("kubeconfig could not be read") && text.contains("Copy details"), "Reload failure recovery missing: \(text)")
        } else if name == "cluster-reload-options" {
            require(text.contains("fixture-reloaded"), "Corrected kubeconfig context not reread: \(text)")
        } else {
            require(!text.contains("Access confirmed") && !text.contains("Orka found"), "Reload retained stale selection: \(text)")
        }
        print("PASS \(name): offline native reload, no real credentials")
        return observations
    }
    if name == "cluster-conditional" || name == "cluster-denied" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        // Vision can split the camel-case label even though the native field is present.
        let accountName = text.contains("ServiceAccount name") || text.contains("Service Account name")
        if name == "cluster-conditional" {
            require(accountName && text.contains("existing account in agents"), "Conditional name-only access prompt missing: \(text)")
        } else {
            require(text.contains("Orka identity") && text.contains("Access was denied") && !accountName, "Denial incorrectly offered identity substitution: \(text)")
        }
        print("PASS \(name): native fixture access state, no real credentials")
        return observations
    }
    if let expected = ["send-ready": "Built in to this Orka", "send-blocked": "Blocked", "send-cannot-check": "Bridge cannot check"][name] {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains(expected), "Native preflight evidence missing: \(expected); observed: \(text)")
        print("PASS \(name): observed native preflight evidence; no workload submitted")
        return observations
    }
    if name == "send-history-restarted" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains("Past Sends") && text.contains("Saved tests: fail") && text.contains("Saved tests: unknown"), "Newest saved results missing after restart: \(text)")
        require(text.contains("bridge-run-") && text.contains("fixture-cluster"), "Saved destination or planned Agent missing: \(text)")
        print("PASS send-history-restarted: saved summaries after app restart; no reconnect or Send")
        return observations
    }
    if name == "send-history-passed" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains("Saved tests: pass"), "Earlier saved pass missing: \(text)")
        return observations
    }
    if name.hasPrefix("send-") {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        let markers = ["Send your agent", "Tests on your cluster", "passed", "evaluating tests", "Planned Agent", "question", "answer", "Not run since your edit", "Stopped watching"]
        require(markers.contains { text.localizedCaseInsensitiveContains($0) }, "Send panel not visible: \(text)")
        print("PASS \(name): fixture-backed native Send screen; no real cluster or model")
        return observations
    }
    if name == "cluster-access" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains("Access confirmed") && text.contains("Open your agent"), "Fixture access not confirmed: \(text)")
        return observations
    }
    if name.hasPrefix("cluster-card") {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains("Orka found") && text.contains("orka:fixture"), "Fixture Orka not visible: \(text)")
        print("PASS \(name): native discovery card, fixture image observed; no Use or Send")
        return observations
    }
    if page == "Clusters" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        // Vision can read the capital I in AI as lowercase l. Match the rest of the heading.
        let welcome = text.contains("Build an") && text.contains("does a real job")
        let choice = text.contains("Where should your agents run")
        let selected = text.contains("Install your workspace files")
        require(welcome || choice || selected, "Workspace heading not visible; observed: \(text)")
        if !welcome {
            require(nativeTextIsVisible("Workspace setup is not included in this build. Nothing will be downloaded or started."),
                    "Release setup IPC did not visibly report its real integration state")
        }
        if ["local-selected", "choice-retained"].contains(name) {
            require(selected && !welcome && !choice, "Native preference was not acknowledged or retained")
            let modelRow = observations.first {
                let label = $0.topCandidates(1).first?.string ?? ""
                return label.contains("Choose the") && label.contains("your agent uses")
            }
            require(modelRow != nil && observations.contains {
                $0.topCandidates(1).first?.string == "Waiting" &&
                    abs($0.boundingBox.midY - modelRow!.boundingBox.midY) < 0.02
            }, "The AI setup step is not waiting")
            require(!observations.contains {
                ["Connect model", "Use GitHub sign-in", "Connect a model"].contains($0.topCandidates(1).first?.string ?? "")
            }, "Model connection is exposed before workspace readiness")
        }
        print("PASS \(name): native workspace screen observed")
        return observations
    }
    let pages = ["Your agents", "Designer", "Clusters", "web-reader"]
    let headings = Set(observations.compactMap { observation -> String? in
        let top = 1 - observation.boundingBox.maxY
        guard top > 0.10 && top < 0.25,
            let text = observation.topCandidates(1).first?.string,
            pages.contains(text) else { return nil }
        return text
    })
    // Negative control: persistent nav labels must not qualify as page headings.
    let expectedHeading = page == "Designer" ? "web-reader" : page == "Overview" ? "Your agents" : page
    for wrong in pages where wrong != expectedHeading && !(page == "Designer" && wrong == "Designer") {
        require(!headings.contains(wrong), "Unexpected \(wrong) page heading; expected \(page)")
    }
    require(headings == Set([expectedHeading]), "Native screenshot does not show the expected \(expectedHeading) page heading; observed \(headings)")
    print("PASS \(name): native \(page) heading observed")
    return observations
}
func scrollMain(_ pixels: Int32) {
    let rect = window()!.1
    let scroll = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: pixels, wheel2: 0, wheel3: 0)
    scroll?.location = CGPoint(x: rect.maxX - 20, y: rect.midY)
    scroll?.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.3)
}
func clickSendControl(_ label: String, scrolling pixels: Int32) throws {
    // Disabling Start during IPC can reset WebKit's keyboard focus. Locate the
    // real control after each state transition instead of counting Tab stops.
    var observed: [String] = []
    for _ in 0..<4 {
        let seen = try capture("send-probe", "Designer")
        observed = seen.compactMap { $0.topCandidates(1).first?.string }
        // Vision includes the native disclosure triangle as a bullet in this label.
        if let control = observed.first(where: { $0 == label || (label == "published-checksum" && $0.range(of: #"^\W*published-checksum\b"#, options: .regularExpression) != nil) || (label == "Details" && $0.lowercased().range(of: #"^\W*details\b"#, options: .regularExpression) != nil) }) {
            clickText(control, in: seen)
            return
        }
        scrollMain(pixels)
    }
    fail("Send control not visible: \(label); observed: \(observed)")
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
    // The owned form works before any workspace exists. Exercise its real writer,
    // not injected renderer data; editing remains owned during setup too.
    // Welcome focuses the choice heading. Clusters and Designer precede it.
    key(48, shift: true); key(48, shift: true); key(49)
    try capture("agent-form", "Designer")
    let instructions = nativeElements().filter { attribute($0, "AXRole") as? String == "AXTextArea"
        && ((attribute($0, "AXTitle") as? String == "Instructions") || (attribute($0, "AXDescription") as? String == "Instructions")) }
    require(instructions.count == 1, "Cannot identify the owned Instructions field")
    guard let fieldBounds = nativeBounds(instructions[0]) else { fail("Instructions field bounds missing") }
    let rect = window()!.1
    let visibleField = fieldBounds.intersection(rect)
    require(!visibleField.isEmpty && !visibleField.isNull, "Instructions field is not visible")
    let owned = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Orka/agents/web-reader/web-reader.agent.md")
    let before = try String(contentsOf: owned, encoding: .utf8)
    let fieldPoint = CGPoint(x: visibleField.midX, y: visibleField.midY)
    mouse(.leftMouseDown, fieldPoint); mouse(.leftMouseUp, fieldPoint)
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 125, keyDown: down)!
        event.flags = .maskCommand; event.post(tap: .cghidEventTap)
    }
    let suffix = " Native form probe \(UUID().uuidString)."
    for character in suffix.utf16 {
        var unit = character
        for down in [true, false] {
            let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
            event.keyboardSetUnicodeString(stringLength: 1, unicodeString: &unit)
            // Do not inherit Command from the caret-navigation events still in flight.
            event.flags = []
            event.post(tap: .cghidEventTap)
        }
    }
    var saved = false
    for _ in 0..<20 {
        if (try String(contentsOf: owned, encoding: .utf8)) == before + suffix { saved = true; break }
        Thread.sleep(forTimeInterval: 0.1)
    }
    require(saved, "Native form did not preserve the file and append the exact instructions suffix")
    let savedForm = try capture("agent-form-saved", "Designer")
    let savedFooter = savedForm.filter { $0.boundingBox.maxY < 0.1 }
        .compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    require(savedFooter.range(of: "\\bSaved\\b", options: .regularExpression) != nil
        && !savedFooter.localizedCaseInsensitiveContains("not saved"),
        "Form save was not acknowledged in footer: \(savedFooter)")
    clickTab("Clusters", savedForm)
    initial = try capture("chooser-after-form", "Clusters")
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
require(nativeTextIsVisible("Agent list unavailable with this Mac image"),
        "Overview must explain the source-inferred old Mac image limit")
require(nativeTextIsVisible("Saved file") && nativeTextIsVisible("web-reader"),
        "Overview must show the saved file opened through the native Designer")
clickTab("Designer", overview)
let designer = try capture("designer-pointer", "Designer")
let clustersTab = clickTab("Clusters", designer)
try capture("clusters-pointer", "Clusters")
// Pointer activation on macOS need not focus a button. Establish and verify the
// native keyboard starting point, then exercise actual traversal/activation.
require(AXUIElementSetAttributeValue(clustersTab, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success,
        "Cannot focus Clusters for native keyboard traversal")
var focused: CFTypeRef?
require(AXUIElementCopyAttributeValue(clustersTab, kAXFocusedAttribute as CFString, &focused) == .success
    && focused as? Bool == true, "Clusters did not receive native keyboard focus")
// From Clusters, Designer and Overview are the preceding navigation stops.
key(48, shift: true); key(48, shift: true); key(49)
try capture("overview-keyboard", "Overview")
key(48); key(49); try capture("designer-keyboard", "Designer")
key(48); key(36); try capture("clusters-keyboard", "Clusters")
// The same owned file stays editable during selected-local setup.
let clusters = try capture("before-file", "Clusters")
clickTab("Designer", clusters)
let agent = try capture("agent-view", "Designer")
guard let fileButton = agent.first(where: { $0.topCandidates(1).first?.string == "File" }) else {
    fail("File view switch is not visible")
}
let fileRect = window()!.1
let filePoint = CGPoint(x: fileRect.minX + fileButton.boundingBox.midX * fileRect.width,
                        y: fileRect.minY + (1 - fileButton.boundingBox.midY) * fileRect.height)
mouse(.leftMouseDown, filePoint); mouse(.leftMouseUp, filePoint)
Thread.sleep(forTimeInterval: 0.5)
let fileView = try capture("file-view", "Designer")
require(!fileView.contains { $0.topCandidates(1).first?.string == "Read-only during setup" },
        "Owned File view is still labelled read-only")
let ownedFile = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Orka/agents/web-reader/web-reader.agent.md")
let bytesBeforeEdit = try String(contentsOf: ownedFile, encoding: .utf8)
let editors = nativeElements().filter { attribute($0, "AXRole") as? String == "AXTextArea"
    && ((attribute($0, "AXTitle") as? String == "Agent file") || (attribute($0, "AXDescription") as? String == "Agent file")) }
require(editors.count == 1, "Cannot identify the owned native File editor")
guard let editorBounds = nativeBounds(editors[0]) else { fail("Native File editor bounds missing") }
let visibleEditor = editorBounds.intersection(fileRect)
require(!visibleEditor.isEmpty && !visibleEditor.isNull, "Native File editor is offscreen")
let editorPoint = CGPoint(x: visibleEditor.midX, y: visibleEditor.midY)
mouse(.leftMouseDown, editorPoint); mouse(.leftMouseUp, editorPoint)
Thread.sleep(forTimeInterval: 0.2)
require(attribute(editors[0], "AXFocused") as? Bool == true, "Native File editor did not receive focus")
for down in [true, false] {
    let event = CGEvent(keyboardEventSource: nil, virtualKey: 125, keyDown: down)!
    event.flags = .maskCommand; event.post(tap: .cghidEventTap)
}
let marker = " Native setup edit probe \(UUID().uuidString)."
for character in marker.utf16 {
    var unit = character
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
        event.keyboardSetUnicodeString(stringLength: 1, unicodeString: &unit)
        event.flags = []; event.post(tap: .cghidEventTap)
    }
}
var editSaved = false
for _ in 0..<20 {
    if (try String(contentsOf: ownedFile, encoding: .utf8)) == bytesBeforeEdit + marker { editSaved = true; break }
    Thread.sleep(forTimeInterval: 0.1)
}
require(editSaved, "Setup edit did not append the exact bytes to the owned file")
scrollMain(1000)
let editedFile = try capture("file-edited", "Designer")
let savedFooter = editedFile.filter { $0.boundingBox.maxY < 0.1 }
    .compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
require(savedFooter.range(of: "\\bSaved\\b", options: .regularExpression) != nil
    && !savedFooter.localizedCaseInsensitiveContains("not saved"), "Setup file save not acknowledged: \(savedFooter)")
print("PASS owned setup editor: native keyboard append persisted exact bytes, without a ready VM")
// Exercise the real settings disclosure; reveal controls are observed, not
// activated, so the walk never opens Finder or touches the clipboard.
clickDesignerView("Agent")
scrollMain(-400)
let beforeSettings = try capture("before-settings", "Designer")
clickText("More settings", in: beforeSettings)
try capture("designer-settings", "Designer")
scrollMain(1000)
let settingsTop = try capture("designer-settings-top", "Designer")
clickTab("Clusters", settingsTop)
// Resize this CI-owned window using its native bottom-right handle.
let rect = window()!.1
// The display may constrain the initial size. Derive the minimum frame from
// AppKit's content-to-frame conversion, not the requested initial dimensions.
let minimumFrame = NSWindow.frameRect(
    forContentRect: CGRect(x: 0, y: 0, width: 540, height: 640),
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
// Set the CI-owned native content area to 540x900, not a browser viewport.
let application = AXUIElementCreateApplication(app.processIdentifier)
var windowsValue: CFTypeRef?
require(AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &windowsValue) == .success,
        "Cannot read native windows for compact walk")
let nativeWindow = (windowsValue as! [AXUIElement])[0]
var origin = CGPoint(x: 20, y: 40)
require(AXUIElementSetAttributeValue(nativeWindow, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &origin)!) == .success, "Cannot position compact window")
let compactFrame = NSWindow.frameRect(forContentRect: CGRect(x: 0, y: 0, width: 540, height: 900),
    styleMask: [.titled, .closable, .miniaturizable, .resizable])
var compactSize = compactFrame.size
require(AXUIElementSetAttributeValue(nativeWindow, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &compactSize)!) == .success, "Cannot resize compact window")
Thread.sleep(forTimeInterval: 0.5)
require(abs(window()!.1.width - compactSize.width) < 3 && abs(window()!.1.height - compactSize.height) < 3,
        "Native 540x900 content area not verified: \(window()!.1)")
let compactClusters = try capture("clusters-compact", "Clusters")
clickTab("Designer", compactClusters)
try capture("designer-compact", "Designer")
clickDesignerView("File")
require(nativeElements().contains { attribute($0, "AXRole") as? String == "AXTextArea"
    && attribute($0, "AXTitle") as? String == "Agent file" }, "Compact File editor did not open")
let compactFile = try capture("file-compact", "Designer")
clickTab("Clusters", compactFile)
print("PASS native walk: pointer, Shift-Tab/Tab, Space, Enter and minimum window")
if ProcessInfo.processInfo.environment["BRIDGE_WALK_KUBECONFIG"] != nil {
    let local = try capture("before-cluster", "Clusters")
    clickText("Use a cluster you already have instead", in: local)
    Thread.sleep(forTimeInterval: 1)
    // The label focuses its native select. Choose the first synthetic context.
    let chooserFile = "\(output)/cluster-chooser.png"
    let command = Process()
    command.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    command.arguments = ["-x", "-o", "-l", String(window()!.0), chooserFile]
    try command.run(); command.waitUntilExit()
    let request = VNRecognizeTextRequest()
    try VNImageRequestHandler(url: URL(fileURLWithPath: chooserFile)).perform([request])
    clickText("kubectl context", in: request.results ?? [])
    // Move past the placeholder whether the first Down opens the popup or advances it.
    key(125); key(125); key(36)
    Thread.sleep(forTimeInterval: 2)
    // Capture discovery before the explicit access confirmation.
    try capture("cluster-card-compact", "Clusters")
    // Exercise the existing conditional field and a denial, without issuing a token.
    func chooseFixture(_ index: Int) throws -> [VNRecognizedTextObservation] {
        scrollMain(1000)
        let chooser = try capture("cluster-probe", "Clusters")
        clickText("kubectl context", in: chooser)
        key(49); key(115) // Open the native select; Home selects its placeholder.
        for _ in 0..<index { key(125) }
        key(36)
        Thread.sleep(forTimeInterval: 1)
        return try capture("cluster-card-selected", "Clusters")
    }
    for (index, name) in [(2, "cluster-conditional"), (3, "cluster-denied")] {
        let selected = try chooseFixture(index)
        clickText("Use this cluster", in: selected)
        Thread.sleep(forTimeInterval: 1)
        scrollMain(-250)
        try capture(name, "Clusters")
    }
    // Change only the CI-owned synthetic kubeconfig, then prove the native session
    // rereads it and invalidates prior access on both failure and correction.
    let configURL = URL(fileURLWithPath: ProcessInfo.processInfo.environment["BRIDGE_WALK_KUBECONFIG"]!)
    let originalConfig = try Data(contentsOf: configURL)
    func reloadFixture() throws {
        for _ in 0..<6 {
            let seen = try capture("cluster-probe", "Clusters")
            if seen.contains(where: { $0.topCandidates(1).first?.string == "Reload kubeconfig" }) {
                clickText("Reload kubeconfig", in: seen)
                Thread.sleep(forTimeInterval: 1)
                return
            }
            scrollMain(-200)
        }
        fatalError("Reload kubeconfig control not found")
    }
    do {
        defer { try? originalConfig.write(to: configURL, options: .atomic) }
        try Data("invalid kubeconfig: [".utf8).write(to: configURL, options: .atomic)
        try reloadFixture()
        try capture("cluster-reload-failed", "Clusters")
        var corrected = try JSONSerialization.jsonObject(with: originalConfig) as! [String: Any]
        var configuredContexts = corrected["contexts"] as! [[String: Any]]
        configuredContexts[0]["name"] = "fixture-reloaded"
        corrected["contexts"] = configuredContexts
        corrected["current-context"] = "fixture-reloaded"
        try JSONSerialization.data(withJSONObject: corrected).write(to: configURL, options: .atomic)
        try reloadFixture()
        scrollMain(1000)
        let reread = try capture("cluster-reload-cleared", "Clusters")
        clickText("kubectl context", in: reread)
        key(49)
        try capture("cluster-reload-options", "Clusters")
        key(53) // Close the native context popup without selecting or using it.
        try originalConfig.write(to: configURL, options: .atomic)
        try reloadFixture()
    }
    func capturePreflight(_ name: String, _ expected: String) throws -> [VNRecognizedTextObservation] {
        for _ in 0..<8 {
            let seen = try capture("send-probe", "Designer")
            if seen.contains(where: { ($0.topCandidates(1).first?.string ?? "").contains(expected) }) {
                let captured = try capture(name, "Designer")
                require(captured.contains { ($0.topCandidates(1).first?.string ?? "").contains(expected) }, "Captured state missing in \(name): \(expected)")
                return captured
            }
            scrollMain(-100)
        }
        fail("Preflight state missing: \(expected)")
    }
    // Read-only preflight states use distinct synthetic identities. No workload
    // is submitted for either blocked card.
    for (index, name, expected) in [(4, "send-blocked", "Blocked"), (5, "send-cannot-check", "Bridge cannot check")] {
        let selected = try chooseFixture(index)
        clickText("Use this cluster", in: selected)
        Thread.sleep(forTimeInterval: 1)
        scrollMain(-400)
        let connected = try capture("cluster-access", "Clusters")
        clickText("Open your agent", in: connected)
        Thread.sleep(forTimeInterval: 0.5)
        let agent = try capture("agent-cluster-owned", "Designer")
        clickText("Send to a cluster", in: agent)
        Thread.sleep(forTimeInterval: 1)
        let checked = try capturePreflight(name, expected)
        clickTab("Clusters", checked)
    }
    let normal = try chooseFixture(1)
    // Closing a native select popup can return focus to navigation. Activate the
    // observed Use control instead of assuming Tab left focus on it.
    clickText("Use this cluster", in: normal)
    Thread.sleep(forTimeInterval: 1)
    scrollMain(-400)
    let connected = try capture("cluster-access", "Clusters")
    clickText("Open your agent", in: connected)
    Thread.sleep(forTimeInterval: 0.5)
    let ownedAgent = try capture("agent-cluster-owned", "Designer")
    clickText("Send to a cluster", in: ownedAgent)
    Thread.sleep(forTimeInterval: 1)
    let allowed = try capturePreflight("send-ready", "Built in to this Orka")
    require(allowed.contains { ($0.topCandidates(1).first?.string ?? "").contains("Built in to this Orka") }, "Allowed preflight evidence is missing")
    func focusSend() throws {
        // Pixel scroll offsets are not stable across responsive reflow. Focus
        // the real entry button, then pointer-click its verified native bounds.
        let matches = nativeElements().filter { element in
            attribute(element, "AXRole") as? String == "AXButton"
                && attribute(element, "AXEnabled") as? Bool == true
                && ["AXTitle", "AXDescription"].contains { attribute(element, $0) as? String == "Send to a cluster" }
        }
        require(matches.count == 1, "Expected one enabled Send entry")
        let entry = matches[0]
        require(AXUIElementSetAttributeValue(entry, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "Cannot focus Send entry")
        Thread.sleep(forTimeInterval: 0.3)
        guard let bounds = nativeBounds(entry), window()!.1.contains(bounds) else { fail("Focused Send entry is not visible") }
        mouse(.leftMouseDown, CGPoint(x: bounds.midX, y: bounds.midY))
        mouse(.leftMouseUp, CGPoint(x: bounds.midX, y: bounds.midY))
        Thread.sleep(forTimeInterval: 0.5)
    }
    func resultSize(_ width: CGFloat, _ height: CGFloat) throws {
        let frame = NSWindow.frameRect(forContentRect: CGRect(x: 0, y: 0, width: width, height: height),
            styleMask: [.titled, .closable, .miniaturizable, .resizable])
        var size = frame.size
        require(AXUIElementSetAttributeValue(nativeWindow, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!) == .success, "Cannot resize result window")
        Thread.sleep(forTimeInterval: 0.3)
        require(abs(window()!.1.width - size.width) < 3 && abs(window()!.1.height - size.height) < 3, "Result window size was not applied")
        // Reflow can move the result header above the preserved scroll offset.
        // Reopen the existing panel without starting another Send.
        try focusSend()
    }
    func captureAnswer(_ name: String) throws {
        let seen = try capturePreflight("send-probe", "QUESTION")
        guard let question = seen.first(where: { ($0.topCandidates(1).first?.string ?? "").contains("QUESTION") })
            else { fail("Question heading missing after native search") }
        let y = (1 - question.boundingBox.maxY) * window()!.1.height
        scrollMain(-Int32(max(0, y - 180)))
        let answer = try capture(name, "Designer")
        let text = answer.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains("What SHA-256 checksum does") && text.contains("Fixture answer:"), "Question and answer are not visible together")
    }
    try clickSendControl("Send and run tests", scrolling: -350)
    Thread.sleep(forTimeInterval: 1)
    let result = try capturePreflight("send-result", "1 of 1 passed")
    require(result.contains { ($0.topCandidates(1).first?.string ?? "").contains("1 of 1 passed") }, "Fixture-backed Send did not pass its real tests")
    try resultSize(1280, 800)
    try capturePreflight("send-result-wide", "1 of 1 passed")
    try clickSendControl("published-checksum", scrolling: -250)
    try captureAnswer("send-answer-wide")
    try resultSize(540, 900)
    try captureAnswer("send-answer")
    try clickSendControl("published-checksum", scrolling: 250)
    try clickSendControl("Details", scrolling: -350)
    let details = try capturePreflight("send-result-details", "Planned Agent")
    let detailsText = details.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    require(detailsText.contains("Planned Agent") && detailsText.contains("Copy"), "Short Agent identity and Copy are not visible in native Send details")
    try clickSendControl("Details", scrolling: 350)
    // Add a distinct failing test to the CI-owned saved file. Reload through the
    // existing preflight flow; the earlier pass must turn stale, not stay green.
    let testsURL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Orka/agents/web-reader/web-reader.agent.test.md")
    let originalTests = try Data(contentsOf: testsURL)
    restoreTemporaryTests = { try originalTests.write(to: testsURL, options: .atomic) }
    defer {
        do { try restoreTemporaryTests?(); restoreTemporaryTests = nil }
        catch { fail("Cannot restore original test file: \(error)") }
    }
    var mixedTests = originalTests
    mixedTests.append(Data("\n```yaml agent-test\nid: has-a-date\ninput: Which date is shown on the page?\nassertions:\n  - id: date\n    type: contains\n    value: '2099-12-31'\n```\n".utf8))
    try mixedTests.write(to: testsURL, options: .atomic)
    try clickSendControl("Check again", scrolling: 350)
    try focusSend()
    try capturePreflight("send-disk-stale", "Not run since your edit")
    try clickSendControl("Reload saved files", scrolling: -250)
    try focusSend()
    try capturePreflight("send-stale", "Not run since your edit")
    try resultSize(1280, 800)
    try capturePreflight("send-stale-wide", "Not run since your edit")
    try clickSendControl("Send and run tests", scrolling: 350)
    Thread.sleep(forTimeInterval: 1)
    try capturePreflight("send-mixed-wide", "1 of 2 passed")
    let failed = try capturePreflight("send-failed-wide", "The answer did not include")
    require(failed.contains { ($0.topCandidates(1).first?.string ?? "").contains("2099-12-31") }, "Plain test failure reason missing")
    try resultSize(540, 900)
    try capturePreflight("send-failed", "The answer did not include")
    // The fixture's third Agent stays Running. This still stops watching only;
    // it does not claim to cancel already-submitted cluster work.
    try clickSendControl("Send and run tests", scrolling: 350)
    Thread.sleep(forTimeInterval: 0.5)
    try capturePreflight("send-running", "Waiting for answers and evaluating tests")
    try resultSize(1280, 800)
    try capturePreflight("send-running-wide", "Waiting for answers and evaluating tests")
    try focusSend()
    try clickSendControl("Stop", scrolling: -350)
    Thread.sleep(forTimeInterval: 0.5)
    let stopped = try capture("send-stopped", "Designer")
    require(stopped.contains { ($0.topCandidates(1).first?.string ?? "").contains("Stopped watching") }, "Native Stop did not return its result")
    clickTab("Clusters", stopped); clickTab("Designer", stopped)
    let returned = try capture("agent-cluster-return", "Designer")
    clickText("Send to a cluster", in: returned)
    let returnedResult = try capturePreflight("send-page-return", "Stopped watching")
    require(returnedResult.contains { ($0.topCandidates(1).first?.string ?? "").contains("Stopped watching") }, "Page return lost the stopped Send result")
    // Reopen without reconnecting the cluster or sending again. Only the small
    // saved summaries survive; detailed results and live handles do not.
    app.terminate()
    for _ in 0..<30 {
        if app.isTerminated { break }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
    }
    require(app.isTerminated, "Bridge did not quit before Send history check")
    app = try launch()
    app.activate(options: [])
    for _ in 0..<30 {
        if window() != nil { break }
        Thread.sleep(forTimeInterval: 1)
    }
    require(window() != nil, "Relaunched Bridge window did not appear for Send history")
    Thread.sleep(forTimeInterval: 2)
    let restarted = try capture("choice-retained", "Clusters")
    clickTab("Designer", restarted)
    Thread.sleep(forTimeInterval: 1)
    scrollMain(3000)
    var historyFound = false
    for _ in 0..<12 {
        let seen = try capture("send-probe", "Designer")
        if let heading = seen.first(where: { $0.topCandidates(1).first?.string == "Past Sends" }) {
            // Position the heading near the top so the two newest summaries fit
            // even when the dark walk also retains the light walk's history.
            let y = (1 - heading.boundingBox.maxY) * window()!.1.height
            scrollMain(-Int32(max(0, y - 110)))
            historyFound = true
            break
        }
        scrollMain(-250)
    }
    require(historyFound, "Saved Send history was not reachable after restart")
    try capture("send-history-restarted", "Designer")
    // The passing Send is now third. Prove it is reachable instead of requiring
    // three summaries to fit in the two-row restart capture.
    var savedPass = false
    for _ in 0..<6 {
        let seen = try capture("send-probe", "Designer")
        if seen.contains(where: { ($0.topCandidates(1).first?.string ?? "").contains("Saved tests: pass") }) {
            try capture("send-history-passed", "Designer")
            savedPass = true
            break
        }
        scrollMain(-150)
    }
    require(savedPass, "Earlier saved pass was not reachable after restart")
}
