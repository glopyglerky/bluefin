// Development-only original vector artwork. No SF Symbols or third-party assets.
import AppKit

func image(width: Int, height: Int, foreground: Bool, background: Bool) -> Data {
    let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
        bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: background ? CGImageAlphaInfo.noneSkipLast.rawValue : CGImageAlphaInfo.premultipliedLast.rawValue)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
    if background {
        let gradient = NSGradient(colors: [NSColor(srgbRed: 0.02, green: 0.15, blue: 0.25, alpha: 1),
            NSColor(srgbRed: 0.015, green: 0.035, blue: 0.08, alpha: 1)])!
        gradient.draw(in: NSRect(x: 0, y: 0, width: width, height: height), angle: -65)
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
        NSColor(srgbRed: 0.1, green: 0.84, blue: 0.93, alpha: 1).setFill(); fin.fill()
        for baseline in [300.0, 240.0] {
            let wave = NSBezierPath(); wave.move(to: NSPoint(x: 235, y: baseline))
            wave.curve(to: NSPoint(x: 790, y: baseline + 12), controlPoint1: NSPoint(x: 430, y: baseline - 65), controlPoint2: NSPoint(x: 575, y: baseline + 80))
            wave.lineWidth = 20; wave.lineCapStyle = .round
            NSColor(srgbRed: 0.37, green: 0.85, blue: 0.94, alpha: 0.7).setStroke(); wave.stroke()
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
try image(width: 1024, height: 1024, foreground: true, background: true).write(to: destination.appendingPathComponent("Bluefin.png"))
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
