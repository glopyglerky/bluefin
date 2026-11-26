// Derived from the repository vector artwork in tools/DrawIcon.swift.
import AppKit

func image(width: Int, height: Int, foreground: Int, background: Bool, dark: Bool = true) -> Data {
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
    if foreground != 0 {
        let side = Double(min(width, height))
        let transform = NSAffineTransform()
        transform.translateX(by: (Double(width) - side) / 2, yBy: (Double(height) - side) / 2)
        transform.scale(by: side / 1024)
        transform.concat()
        if foreground == 1 || foreground == 3 {
        let fin = NSBezierPath()
        fin.move(to: NSPoint(x: 225, y: 420))
        fin.curve(to: NSPoint(x: 620, y: 785), controlPoint1: NSPoint(x: 388, y: 490), controlPoint2: NSPoint(x: 410, y: 680))
        fin.curve(to: NSPoint(x: 790, y: 420), controlPoint1: NSPoint(x: 580, y: 625), controlPoint2: NSPoint(x: 670, y: 470))
        fin.curve(to: NSPoint(x: 225, y: 420), controlPoint1: NSPoint(x: 560, y: 470), controlPoint2: NSPoint(x: 420, y: 337))
        (dark
            ? NSColor(srgbRed: 204.0 / 255, green: 208.0 / 255, blue: 211.0 / 255, alpha: 1)
            : NSColor(srgbRed: 6.0 / 255, green: 23.0 / 255, blue: 39.0 / 255, alpha: 1)).setFill(); fin.fill()
        }
        if foreground == 2 || foreground == 3 {
        for baseline in [300.0, 240.0] {
            let wave = NSBezierPath(); wave.move(to: NSPoint(x: 235, y: baseline))
            wave.curve(to: NSPoint(x: 790, y: baseline + 12), controlPoint1: NSPoint(x: 430, y: baseline - 65), controlPoint2: NSPoint(x: 575, y: baseline + 80))
            wave.lineWidth = 20; wave.lineCapStyle = .round
            NSColor(srgbRed: 130.0 / 255, green: 137.0 / 255, blue: 165.0 / 255, alpha: 1).setStroke(); wave.stroke()
        }
    }
        }
    NSGraphicsContext.restoreGraphicsState()
    return NSBitmapImageRep(cgImage: context.makeImage()!).representation(using: .png, properties: [:])!
}
func json(_ value: Any, at path: URL) throws {
    try FileManager.default.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
    try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]).write(to: path)
}
// Derives platform assets from tools/DrawIcon.swift's existing fin and waves.
// Run with a destination root, outside the source checkout when reviewing.
let info: [String: Any] = ["author": "org.glopyglerky.bluefin", "version": 1]
guard CommandLine.arguments.count == 2 else { fatalError("Supply candidate root") }
let root = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
func catalog(_ path: String) throws -> URL {
    let url = root.appendingPathComponent("apps/bluefin-apple/" + path)
    try json(["info":info], at: url.appendingPathComponent("Contents.json"))
    return url
}
let mac = try catalog("Mac/Resources/Assets.xcassets").appendingPathComponent("BluefinMacIcon.appiconset")
var macImages: [[String: Any]] = []
for size in [16,32,128,256,512] {
    for scale in [1,2] {
        let filename = "Icon-\(size)@\(scale)x.png"
        try json(["info":info], at: mac.appendingPathComponent("Contents.json"))
        try image(width:size*scale,height:size*scale,foreground:3,background:true).write(to:mac.appendingPathComponent(filename))
        macImages.append(["idiom":"mac","size":"\(size)x\(size)","scale":"\(scale)x","filename":filename])
    }
}
// Catalyst uses its iPad idiom for the source icon as well as native Mac slots.
try image(width:1024,height:1024,foreground:3,background:true).write(to:mac.appendingPathComponent("Bluefin.png"))
macImages.append(["idiom":"universal","platform":"ios","size":"1024x1024","filename":"Bluefin.png"])
try json(["info":info,"images":macImages], at: mac.appendingPathComponent("Contents.json"))
let watch = try catalog("Watch/Resources/Assets.xcassets").appendingPathComponent("BluefinWatchIcon.appiconset")
try json(["info":info,"images":[["idiom":"universal","platform":"watchos","size":"1024x1024","filename":"Bluefin.png"]]], at: watch.appendingPathComponent("Contents.json"))
try image(width:1024,height:1024,foreground:3,background:true).write(to:watch.appendingPathComponent("Bluefin.png"))
let vision = try catalog("Vision/Resources/Assets.xcassets").appendingPathComponent("BluefinVisionIcon.solidimagestack")
try json(["info":info,"layers":[["filename":"Front.solidimagestacklayer"],["filename":"Middle.solidimagestacklayer"],["filename":"Back.solidimagestacklayer"]]], at:vision.appendingPathComponent("Contents.json"))
for (name, foreground, background) in [("Front",1,false),("Middle",2,false),("Back",0,true)] {
    let layer = vision.appendingPathComponent(name + ".solidimagestacklayer")
    try json(["info":info], at:layer.appendingPathComponent("Contents.json"))
    let content = layer.appendingPathComponent("Content.imageset")
    try json(["info":info,"images":[["idiom":"vision","scale":"2x","filename":name + ".png"]]], at:content.appendingPathComponent("Contents.json"))
    try image(width:1024,height:1024,foreground:foreground,background:background).write(to:content.appendingPathComponent(name + ".png"))
}
