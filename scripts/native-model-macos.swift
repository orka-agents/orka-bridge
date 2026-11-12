// Exercise the real compiled frontend in the separate synthetic native fixture.
// This is UI evidence only, not a VM, OS credential-store or inference walk.
import AppKit
import CoreGraphics
import Vision

func require(_ value: Bool, _ message: String) {
    if !value { fputs("\(message)\n", stderr); exit(1) }
}
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
    let visible = text.replacingOccurrences(of: "\n", with: " ")
    for value in expected { require(visible.contains(value), "Missing \(value) in \(name): \(text)") }
    guard let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: file))),
          let colour = bitmap.colorAt(x: bitmap.pixelsWide - 20, y: bitmap.pixelsHigh - 20)?.usingColorSpace(.deviceRGB)
        else { fatalError("Cannot sample native theme") }
    let brightness = (colour.redComponent + colour.greenComponent + colour.blueComponent) / 3
    require(theme == "dark" ? brightness < 0.25 : brightness > 0.8, "Wrong system theme")
    print("PASS \(name): real native frontend, synthetic IPC only")
    return rows
}
func click(_ label: String, _ rows: [VNRecognizedTextObservation]) {
    guard let row = rows.first(where: { $0.topCandidates(1).first?.string == label })
        else { fatalError("Native control not visible: \(label)") }
    let rect = window()!.1
    let p = CGPoint(x: rect.minX + row.boundingBox.midX * rect.width,
                    y: rect.minY + (1 - row.boundingBox.midY) * rect.height)
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
