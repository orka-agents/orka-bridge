import AppKit
import CoreGraphics
import ApplicationServices
import Vision
import ImageIO

func require(_ condition: Bool, _ message: String) {
    if !condition { fputs("\(message)\n", stderr); exit(1) }
}
let output = CommandLine.arguments[1]
let theme = CommandLine.arguments[3]
require(["light", "dark"].contains(theme), "Expected light or dark theme")
require(NSRunningApplication.runningApplications(withBundleIdentifier: "ai.orka.bridge").isEmpty,
        "Quit running Bridge before running the native walk")
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
    // Small Inter labels can be grouped with subtitles by Vision. Identify the
    // real enabled native button, but still pointer-click it and verify the
    // rendered destination in the next capture (not AXPress or renderer hooks).
    let header = observations.filter { 1 - $0.boundingBox.maxY < 0.10 }
        .compactMap { $0.topCandidates(1).first?.string }
    print("NAV \(page): header OCR \(header)")
    func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
    }
    var nodes = [AXUIElementCreateApplication(app.processIdentifier)]
    var index = 0
    var matches: [AXUIElement] = []
    while index < nodes.count && nodes.count < 4000 {
        let element = nodes[index]; index += 1
        if attribute(element, "AXRole") as? String == "AXButton",
            attribute(element, "AXEnabled") as? Bool == true,
            ["AXTitle", "AXDescription"].contains(where: { attribute(element, $0) as? String == page }) {
            matches.append(element)
        }
        for key in ["AXChildren", "AXWindows"] {
            for child in attribute(element, key) as? [AXUIElement] ?? []
                where !nodes.contains(where: { CFEqual($0, child) }) { nodes.append(child) }
        }
    }
    require(matches.count == 1, "Expected one enabled native tab named \(page), found \(matches.count)")
    let target = matches[0]
    guard let position = attribute(target, "AXPosition"), CFGetTypeID(position) == AXValueGetTypeID(),
        let size = attribute(target, "AXSize"), CFGetTypeID(size) == AXValueGetTypeID() else {
        fatalError("Native tab has no bounds: \(page)")
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
        let space = CGColorSpace(name: CGColorSpace.sRGB) else { fatalError("Cannot decode screenshot") }
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
    if name == "cluster-conditional" || name == "cluster-denied" {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        // Vision can split the camel-case label even though the native field is present.
        let accountName = text.contains("ServiceAccount name") || text.contains("Service Account name")
        if name == "cluster-conditional" {
            require(accountName && text.contains("existing account in agents"), "Conditional name-only access prompt missing: \(text)")
        } else {
            require(text.contains("Access was denied") && !accountName, "Denial incorrectly offered identity substitution: \(text)")
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
        require(text.contains("Past Sends") && text.contains("Saved checks: pass") && text.contains("Saved checks: unknown"), "Saved pass and unknown missing after restart: \(text)")
        require(text.contains("bridge-run-") && text.contains("fixture-cluster"), "Saved destination or planned Agent missing: \(text)")
        print("PASS send-history-restarted: saved summaries after app restart; no reconnect or Send")
        return observations
    }
    if name.hasPrefix("send-") {
        let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        require(text.contains("Send your agent") || text.contains("Checks on your cluster") || text.contains("Checks:") || text.contains("Running checks") || text.contains("Planned Agent"), "Send panel not visible: \(text)")
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
        let selected = text.contains("Get your workspace ready")
        require(welcome || choice || selected, "Workspace heading not visible; observed: \(text)")
        if !welcome {
            require(text.contains("Workspace setup is not included"),
                    "Release setup IPC did not report its real integration state")
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
    let pages = ["Overview", "Designer", "Clusters", "web-reader"]
    let headings = Set(observations.compactMap { observation -> String? in
        let top = 1 - observation.boundingBox.maxY
        guard top > 0.10 && top < 0.25,
            let text = observation.topCandidates(1).first?.string,
            pages.contains(text) else { return nil }
        return text
    })
    // Negative control: persistent nav labels must not qualify as page headings.
    for wrong in pages where wrong != page && !(page == "Designer" && wrong == "web-reader") {
        require(!headings.contains(wrong), "Unexpected \(wrong) page heading; expected \(page)")
    }
    let expectedHeading = page == "Designer" ? "web-reader" : page
    require(headings == Set([expectedHeading]), "Native screenshot does not show the expected \(expectedHeading) page heading; observed \(headings)")
    print("PASS \(name): native \(page) heading observed")
    return observations
}
func scrollMain(_ pixels: Int32) {
    let rect = window()!.1
    mouse(.mouseMoved, CGPoint(x: rect.maxX - 20, y: rect.midY))
    CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: pixels, wheel2: 0, wheel3: 0)?.post(tap: .cghidEventTap)
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
        if let control = observed.first(where: { $0 == label || (label == "Details" && $0.lowercased().contains("details")) }) {
            clickText(control, in: seen)
            return
        }
        scrollMain(pixels)
    }
    fatalError("Send control not visible: \(label); observed: \(observed)")
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
    // not injected renderer data; the later setup preview must remain read-only.
    // Welcome focuses the choice heading. Clusters and Designer precede it.
    key(48, shift: true); key(48, shift: true); key(49)
    let form = try capture("agent-form", "Designer")
    guard let label = form.first(where: {
        let text = $0.topCandidates(1).first?.string ?? ""
        return text == "Instructions" || (text.hasPrefix("Instructions ") && text.contains("What the agent"))
    }) else {
        fatalError("Instructions field is not visible")
    }
    let owned = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Orka/agents/web-reader/web-reader.agent.md")
    let before = try String(contentsOf: owned, encoding: .utf8)
    let rect = window()!.1
    let fieldPoint = CGPoint(x: rect.midX,
        y: rect.minY + (1 - label.boundingBox.midY) * rect.height + 70)
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
clickTab("Designer", overview)
let designer = try capture("designer-pointer", "Designer")
clickTab("Clusters", designer)
try capture("clusters-pointer", "Clusters")
// From Clusters, Designer and Overview are the preceding navigation stops.
key(48, shift: true); key(48, shift: true); key(49)
try capture("overview-keyboard", "Overview")
key(48); key(49); try capture("designer-keyboard", "Designer")
key(48); key(36); try capture("clusters-keyboard", "Clusters")
// Setup previews must remain read-only, including native keyboard editing attempts.
let clusters = try capture("before-file", "Clusters")
clickTab("Designer", clusters)
let agent = try capture("agent-view", "Designer")
guard let fileButton = agent.first(where: { $0.topCandidates(1).first?.string == "File" }) else {
    fatalError("File view switch is not visible")
}
let fileRect = window()!.1
let filePoint = CGPoint(x: fileRect.minX + fileButton.boundingBox.midX * fileRect.width,
                        y: fileRect.minY + (1 - fileButton.boundingBox.midY) * fileRect.height)
mouse(.leftMouseDown, filePoint); mouse(.leftMouseUp, filePoint)
Thread.sleep(forTimeInterval: 0.5)
let fileView = try capture("file-view", "Designer")
require(fileView.contains { $0.topCandidates(1).first?.string == "Read-only during setup" },
        "Native setup preview is not labelled read-only")
let ownedFile = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Orka/agents/web-reader/web-reader.agent.md")
let existedBeforeEdit = FileManager.default.fileExists(atPath: ownedFile.path)
let bytesBeforeEdit = existedBeforeEdit ? try Data(contentsOf: ownedFile) : nil
// A pointer-activated button need not receive focus in the native webview.
// Establish editor focus, then verify keyboard exit/re-entry before typing.
let editorPoint = CGPoint(x: fileRect.midX, y: fileRect.minY + fileRect.height * 0.6)
mouse(.leftMouseDown, editorPoint); mouse(.leftMouseUp, editorPoint)
key(48, shift: true); key(48)
// Unlike the owned-editor append probe, a read-only edit attempt needs no
// end-of-file navigation: Command-Down can scroll the page heading out of view.
let marker = "\n\nNative read-only edit probe \(UUID().uuidString)."
// Return is an editing command, not part of a multi-character Unicode event.
for character in marker.utf16 {
    if character == 10 { key(36); continue }
    var unit = character
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
        event.keyboardSetUnicodeString(stringLength: 1, unicodeString: &unit)
        event.post(tap: .cghidEventTap)
    }
}
Thread.sleep(forTimeInterval: 1)
try capture("file-edited", "Designer")
let existsAfterEdit = FileManager.default.fileExists(atPath: ownedFile.path)
require(existsAfterEdit == existedBeforeEdit, "Setup preview changed owned-file existence")
if let bytesBeforeEdit {
    require(try Data(contentsOf: ownedFile) == bytesBeforeEdit,
            "Setup preview persisted a native keyboard edit")
}
let readOnlyFile = try capture("file-read-only", "Designer")
require(readOnlyFile.contains { $0.topCandidates(1).first?.string == "Read-only during setup" },
        "Native setup preview lost its read-only state after the edit attempt")
print("PASS read-only preview: keyboard edit attempted; owned file unchanged or still absent")
clickTab("Clusters", readOnlyFile)
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
let compactDesigner = try capture("designer-compact", "Designer")
clickTab("Clusters", compactDesigner)
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
    func capturePreflight(_ name: String, _ expected: String) throws -> [VNRecognizedTextObservation] {
        for _ in 0..<8 {
            let seen = try capture("send-probe", "Designer")
            if seen.contains(where: { ($0.topCandidates(1).first?.string ?? "").contains(expected) }) {
                return try capture(name, "Designer")
            }
            scrollMain(-100)
        }
        fatalError("Preflight state missing: \(expected)")
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
    try clickSendControl("Send and run checks", scrolling: -350)
    Thread.sleep(forTimeInterval: 1)
    let result = try capture("send-result", "Designer")
    require(result.contains { $0.topCandidates(1).first?.string == "Checks: pass" }, "Fixture-backed Send did not pass its real assertions")
    try clickSendControl("Details", scrolling: -350)
    scrollMain(-350)
    let details = try capture("send-result-details", "Designer")
    let detailsText = details.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    require(detailsText.contains("Planned Agent") && detailsText.contains("Copy"), "Short Agent identity and Copy are not visible in native Send details")
    // The fixture's second Agent stays Running so this is a real Stop, not an
    // injected renderer state. Previously created resources remain on the fixture.
    try clickSendControl("Send and run checks", scrolling: 350)
    Thread.sleep(forTimeInterval: 0.5)
    try capture("send-running", "Designer")
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
}
