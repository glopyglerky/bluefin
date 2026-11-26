// Development-only original vector artwork. No SF Symbols or third-party assets.
import AppKit

func image(width: Int, height: Int, foreground: Bool, background: Bool, dark: Bool = true) -> Data {
    let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
        bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: background ? CGImageAlphaInfo.noneSkipLast.rawValue : CGImageAlphaInfo.premultipliedLast.rawValue)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
    if background {
        let paper = dark
            ? NSColor(srgbRed: 6.0 / 255, green: 23.0 / 255, blue: 39.0 / 255, alpha: 1)
            : NSColor(srgbRed: 163.0 / 255, green: 172.0 / 255, blue: 177.0 / 255, alpha: 1)
        paper.setFill()
        NSRect(x: 0, y: 0, width: width, height: height).fill()
        // Fine static engraving stays in the background. Small icons remain clean.
        if min(width, height) > 64 {
            let lines = NSBezierPath()
            let spacing = max(4.0, Double(min(width, height)) / 64)
            for y in stride(from: 0.0, through: Double(height), by: spacing) {
                lines.move(to: NSPoint(x: 0, y: y))
                lines.curve(to: NSPoint(x: Double(width), y: y + 2),
                            controlPoint1: NSPoint(x: Double(width) / 3, y: y - 2),
                            controlPoint2: NSPoint(x: Double(width) * 2 / 3, y: y - 2))
            }
            lines.lineWidth = max(0.35, Double(min(width, height)) / 2048)
            (dark ? NSColor.white : NSColor.black).withAlphaComponent(0.035).setStroke()
            lines.stroke()
        }
    } else {
        NSColor.clear.setFill()
        NSRect(x: 0, y: 0, width: width, height: height).fill(using: .copy)
    }
    if foreground {
        let side = Double(min(width, height))
        let transform = NSAffineTransform()
        transform.translateX(by: (Double(width) - side) / 2, yBy: (Double(height) - side) / 2)
        transform.scale(by: side / 1024)
        transform.concat()
        let fin = NSBezierPath()
        fin.move(to: NSPoint(x: 225, y: 420))
        fin.curve(to: NSPoint(x: 620, y: 785), controlPoint1: NSPoint(x: 388, y: 490), controlPoint2: NSPoint(x: 410, y: 680))
        fin.curve(to: NSPoint(x: 790, y: 420), controlPoint1: NSPoint(x: 580, y: 625), controlPoint2: NSPoint(x: 670, y: 470))
        fin.curve(to: NSPoint(x: 225, y: 420), controlPoint1: NSPoint(x: 560, y: 470), controlPoint2: NSPoint(x: 420, y: 337))
        (dark
            ? NSColor(srgbRed: 204.0 / 255, green: 208.0 / 255, blue: 211.0 / 255, alpha: 1)
            : NSColor(srgbRed: 6.0 / 255, green: 23.0 / 255, blue: 39.0 / 255, alpha: 1)).setFill(); fin.fill()
        for baseline in [300.0, 240.0] {
            let wave = NSBezierPath(); wave.move(to: NSPoint(x: 235, y: baseline))
            wave.curve(to: NSPoint(x: 790, y: baseline + 12), controlPoint1: NSPoint(x: 430, y: baseline - 65), controlPoint2: NSPoint(x: 575, y: baseline + 80))
            wave.lineWidth = 20; wave.lineCapStyle = .round
            NSColor(srgbRed: 130.0 / 255, green: 137.0 / 255, blue: 165.0 / 255, alpha: 1).setStroke(); wave.stroke()
        }
    }
    NSGraphicsContext.restoreGraphicsState()
    return NSBitmapImageRep(cgImage: context.makeImage()!).representation(using: .png, properties: [:])!
}
func json(_ value: Any, at path: URL) throws {
    try FileManager.default.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
    try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]).write(to: path)
}
let info: [String: Any] = ["author": "org.glopyglerky.bluefin", "version": 1]
guard CommandLine.arguments.count == 2 || CommandLine.arguments.count == 3 else { fatalError("Supply iOS icon folder and optional tvOS brand folder") }
let destination = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
try image(width: 1024, height: 1024, foreground: true, background: true, dark: false).write(to: destination.appendingPathComponent("Bluefin.png"))
try image(width: 1024, height: 1024, foreground: true, background: true).write(to: destination.appendingPathComponent("Bluefin-dark.png"))
try json(["info": info, "images": [
    ["filename": "Bluefin.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"],
    ["filename": "Bluefin-dark.png", "idiom": "universal", "platform": "ios", "size": "1024x1024",
     "appearances": [["appearance": "luminosity", "value": "dark"]]]
]], at: destination.appendingPathComponent("Contents.json"))
if CommandLine.arguments.count == 3 {
    let brand = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
    var assets: [[String: Any]] = []
    for (name, width, height, scales) in [("Small", 400, 240, [1,2]), ("Large", 1280, 768, [1])] {
        let stack = brand.appendingPathComponent(name + ".imagestack", isDirectory: true)
        try json(["info": info, "layers": [["filename":"Foreground.imagestacklayer"],["filename":"Background.imagestacklayer"]]], at: stack.appendingPathComponent("Contents.json"))
        for layer in ["Foreground", "Background"] {
            let folder = stack.appendingPathComponent(layer + ".imagestacklayer/Content.imageset", isDirectory: true)
            var images: [[String:Any]] = []
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            for scale in scales {
                let filename = layer + "@\(scale)x.png"
                try image(width: width * scale, height: height * scale, foreground: layer == "Foreground", background: layer == "Background").write(to: folder.appendingPathComponent(filename))
                images.append(["filename":filename,"idiom":"tv","scale":"\(scale)x"])
            }
            try json(["images":images,"info":info], at: folder.appendingPathComponent("Contents.json"))
        }
        assets.append(["filename":name + ".imagestack","idiom":"tv","role":"primary-app-icon","size":"\(width)x\(height)"])
    }
    let shelf = brand.appendingPathComponent("TopShelf.imageset", isDirectory: true)
    try FileManager.default.createDirectory(at: shelf, withIntermediateDirectories: true)
    var images: [[String: Any]] = []
    for scale in [1,2] {
        let filename = "TopShelf@\(scale)x.png"
        try image(width: 1920 * scale, height: 720 * scale, foreground: true, background: true).write(to: shelf.appendingPathComponent(filename))
        images.append(["filename":filename,"idiom":"tv","scale":"\(scale)x"])
    }
    try json(["images":images,"info":info], at: shelf.appendingPathComponent("Contents.json"))
    assets.append(["filename":"TopShelf.imageset","idiom":"tv","role":"top-shelf-image","size":"1920x720"])
    try json(["assets":assets,"info":info], at: brand.appendingPathComponent("Contents.json"))
}
