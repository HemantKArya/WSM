// GPU acceptance window (round 2 work/gpu-followup/anim.swift + a static mode for drag tests).
//   anim <seconds>          recolour the content every 1/60 s (frame-rate test)
//   anim <seconds> static   keep still; frames come only from dragging it (drag test)
// Prints the title bar centre in top-left screen coordinates, which is what QMP's tablet uses.
import AppKit
let args = CommandLine.arguments.dropFirst()
let seconds = Double(args.first ?? "20") ?? 20
let animate = !args.contains("static")
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let w = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 800, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
w.title = "WSM frame-rate test"
let v = NSView(frame: w.contentView!.bounds); v.wantsLayer = true; w.contentView = v
v.layer?.backgroundColor = NSColor.systemTeal.cgColor
w.makeKeyAndOrderFront(nil); app.activate(ignoringOtherApps: true)
let screenH = NSScreen.main?.frame.height ?? 1080
let titleH = w.frame.height - w.contentLayoutRect.height
print(String(format: "titlebar %.0f %.0f", w.frame.midX, screenH - w.frame.maxY + titleH / 2)); fflush(stdout)
var frames = 0; let start = Date()
let t = Timer(timeInterval: 1.0 / 60, repeats: true) { _ in
    frames += 1
    if animate {
        let h = CGFloat(frames % 120) / 120
        v.layer?.backgroundColor = NSColor(hue: h, saturation: 0.8, brightness: 0.9, alpha: 1).cgColor
    }
    if Date().timeIntervalSince(start) > seconds {
        print(String(format: "app ticks %d in %.1f s = %.1f/s", frames, Date().timeIntervalSince(start), Double(frames) / Date().timeIntervalSince(start)))
        fflush(stdout); app.terminate(nil)
    }
}
RunLoop.main.add(t, forMode: .common)
print("anim started"); fflush(stdout)
app.run()
