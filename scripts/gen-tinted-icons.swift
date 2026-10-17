// Generates tinted copies of the grid app icons: `swift scripts/gen-tinted-icons.swift`.
// Each icon is recolored by luminance (white -> accent, black stays black) and written as
// ios/Assets.xcassets/AppIcon-<Design>-<Tint>.appiconset. Run once; the output is committed.
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let designs = ["GridMinimal", "GridPrompts", "GridRounded", "GridSymbols", "GridSymbolsBold"]
// Same colors as AppAccent (blue, teal, green, orange, red, pink, purple).
let tints: [(String, (Double, Double, Double))] = [
    ("Blue", (0.04, 0.52, 1.0)), ("Teal", (0.19, 0.69, 0.78)), ("Green", (0.20, 0.78, 0.35)),
    ("Orange", (1.0, 0.58, 0.0)), ("Red", (1.0, 0.27, 0.23)), ("Pink", (1.0, 0.31, 0.55)),
    ("Purple", (0.69, 0.32, 0.87)),
]
let assets = "ios/Assets.xcassets"
let fm = FileManager.default

for d in designs {
    let src = "\(assets)/AppIcon-\(d).appiconset/AppIcon-\(d).png"
    guard let provider = CGDataProvider(filename: src), let img = CGImage(pngDataProviderSource: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent) else { fatalError("cannot read \(src)") }
    let w = img.width, h = img.height
    var base = [UInt8](repeating: 0, count: w * h * 4)
    let cs = CGColorSpaceCreateDeviceRGB()
    let info = CGImageAlphaInfo.noneSkipLast.rawValue
    base.withUnsafeMutableBytes { p in
        let ctx = CGContext(data: p.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: cs, bitmapInfo: info)!
        ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
    }
    for (name, c) in tints {
        var out = base
        for i in stride(from: 0, to: out.count, by: 4) {
            let l = (0.299 * Double(base[i]) + 0.587 * Double(base[i + 1]) + 0.114 * Double(base[i + 2])) / 255
            out[i] = UInt8(min(255, l * c.0 * 255))
            out[i + 1] = UInt8(min(255, l * c.1 * 255))
            out[i + 2] = UInt8(min(255, l * c.2 * 255))
            out[i + 3] = 255
        }
        let dir = "\(assets)/AppIcon-\(d)-\(name).appiconset"
        try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let file = "AppIcon-\(d)-\(name).png"
        out.withUnsafeMutableBytes { p in
            let ctx = CGContext(data: p.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: cs, bitmapInfo: info)!
            let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: "\(dir)/\(file)") as CFURL, UTType.png.identifier as CFString, 1, nil)!
            CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
            CGImageDestinationFinalize(dest)
        }
        let json = """
        {
          "images" : [
            { "filename" : "\(file)", "idiom" : "universal", "platform" : "ios", "size" : "1024x1024" }
          ],
          "info" : { "author" : "xcode", "version" : 1 }
        }
        """
        try json.write(toFile: "\(dir)/Contents.json", atomically: true, encoding: .utf8)
    }
}
print("done")
