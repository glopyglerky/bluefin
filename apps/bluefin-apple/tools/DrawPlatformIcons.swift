// Derived from the repository vector artwork in tools/DrawIcon.swift.
import AppKit

func image(width: Int, height: Int, foreground: Int, background: Bool) -> Data {
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
        NSColor(srgbRed: 0.1, green: 0.84, blue: 0.93, alpha: 1).setFill(); fin.fill()
        }
        if foreground == 2 || foreground == 3 {
        for baseline in [300.0, 240.0] {
            let wave = NSBezierPath(); wave.move(to: NSPoint(x: 235, y: baseline))
            wave.curve(to: NSPoint(x: 790, y: baseline + 12), controlPoint1: NSPoint(x: 430, y: baseline - 65), controlPoint2: NSPoint(x: 575, y: baseline + 80))
            wave.lineWidth = 20; wave.lineCapStyle = .round
            NSColor(srgbRed: 0.37, green: 0.85, blue: 0.94, alpha: 0.7).setStroke(); wave.stroke()
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
