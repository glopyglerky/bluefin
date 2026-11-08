// Development-only asset generator. Uses original paths, not SF Symbols.
import AppKit

let destination = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
let size = 1024
let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
let gradient = NSGradient(colors: [NSColor(srgbRed: 0.02, green: 0.15, blue: 0.25, alpha: 1), NSColor(srgbRed: 0.015, green: 0.035, blue: 0.08, alpha: 1)])!
gradient.draw(in: NSRect(x: 0, y: 0, width: size, height: size), angle: -65)
let fin = NSBezierPath()
fin.move(to: NSPoint(x: 225, y: 420))
fin.curve(to: NSPoint(x: 620, y: 785), controlPoint1: NSPoint(x: 388, y: 490), controlPoint2: NSPoint(x: 410, y: 680))
fin.curve(to: NSPoint(x: 790, y: 420), controlPoint1: NSPoint(x: 580, y: 625), controlPoint2: NSPoint(x: 670, y: 470))
fin.curve(to: NSPoint(x: 225, y: 420), controlPoint1: NSPoint(x: 560, y: 470), controlPoint2: NSPoint(x: 420, y: 337))
NSColor(srgbRed: 0.1, green: 0.84, blue: 0.93, alpha: 1).setFill()
fin.fill()
for baseline in [300.0, 240.0] {
    let wave = NSBezierPath()
    wave.move(to: NSPoint(x: 235, y: baseline))
    wave.curve(to: NSPoint(x: 790, y: baseline + 12), controlPoint1: NSPoint(x: 430, y: baseline - 65), controlPoint2: NSPoint(x: 575, y: baseline + 80))
    wave.lineWidth = 20
    wave.lineCapStyle = .round
    NSColor(srgbRed: 0.37, green: 0.85, blue: 0.94, alpha: 0.7).setStroke()
    wave.stroke()
}
NSGraphicsContext.restoreGraphicsState()
try bitmap.representation(using: .png, properties: [:])!.write(to: destination.appendingPathComponent("Bluefin.png"))
