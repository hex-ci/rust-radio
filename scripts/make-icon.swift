import AppKit
import ImageIO
import UniformTypeIdentifiers

// 与 crates/rust-radio-app/src/main.rs 的 app_icon() 同一套几何，按 1024 画布重绘。
let S = 1024
let cs = CGColorSpaceCreateDeviceRGB()
guard let ctx = CGContext(
    data: nil, width: S, height: S, bitsPerComponent: 8, bytesPerRow: 0,
    space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else { fatalError("ctx") }

ctx.setAllowsAntialiasing(true)
ctx.setShouldAntialias(true)
ctx.clear(CGRect(x: 0, y: 0, width: S, height: S))

// macOS 图标网格：824×824 的圆角方块居中
let inset: CGFloat = 100
let side: CGFloat = 824
let rect = CGRect(x: inset, y: inset, width: side, height: side)
// 与运行时图标一致：连续曲率的超椭圆（squircle），不是圆弧倒角
let shape = CGMutablePath()
let sqExp: CGFloat = 3.5
let sqSteps = 720
for i in 0...sqSteps {
    let t = CGFloat(i) / CGFloat(sqSteps) * 2 * .pi
    let ct = cos(t)
    let st = sin(t)
    let px = rect.midX + rect.width / 2 * copysign(pow(abs(ct), 2 / sqExp), ct)
    let py = rect.midY + rect.height / 2 * copysign(pow(abs(st), 2 / sqExp), st)
    if i == 0 {
        shape.move(to: CGPoint(x: px, y: py))
    } else {
        shape.addLine(to: CGPoint(x: px, y: py))
    }
}
shape.closeSubpath()

ctx.addPath(shape)
ctx.setFillColor(CGColor(srgbRed: 1.0, green: 0x5a / 255.0, blue: 0x3c / 255.0, alpha: 1))
ctx.fillPath()
ctx.restoreGState()

ctx.addPath(shape)
ctx.setFillColor(CGColor(srgbRed: 1.0, green: 0x5a / 255.0, blue: 0x3c / 255.0, alpha: 1))
ctx.fillPath()

ctx.saveGState()
ctx.addPath(shape)
ctx.clip()

let c = CGPoint(x: S / 2, y: S / 2)
let s = side / 60.0 // 原 64px 图标里有效方块是 60px
let dark = CGColor(srgbRed: 0x1a / 255.0, green: 0x09 / 255.0, blue: 0x06 / 255.0, alpha: 1)
ctx.setFillColor(dark)
ctx.setStrokeColor(dark)
ctx.setLineCap(.butt)

// 圆心
ctx.fillEllipse(in: CGRect(x: c.x - 5 * s, y: c.y - 5 * s, width: 10 * s, height: 10 * s))

// 两道开口朝左的弧（右半圆）
for (r, lw) in [(12.0, 4.6), (19.0, 4.6)] {
    ctx.setLineWidth(lw * s)
    ctx.addArc(center: c, radius: r * s, startAngle: -.pi / 2, endAngle: .pi / 2, clockwise: false)
    ctx.strokePath()
}
ctx.restoreGState()

// 画布边缘放一圈 alpha=1/255 的“不可见”像素：部分 macOS 版本会按非透明包围盒
// 缩放 Dock 图标，没有这圈像素时 824 的方块会被当成满画布，看起来比别的图标大一圈。
ctx.setAlpha(1.0 / 255.0)
ctx.setFillColor(CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 1))
ctx.fill(CGRect(x: 0, y: 0, width: CGFloat(S), height: 1))
ctx.fill(CGRect(x: 0, y: CGFloat(S - 1), width: CGFloat(S), height: 1))
ctx.fill(CGRect(x: 0, y: 0, width: 1, height: CGFloat(S)))
ctx.fill(CGRect(x: CGFloat(S - 1), y: 0, width: 1, height: CGFloat(S)))
ctx.setAlpha(1.0)

guard let image = ctx.makeImage() else { fatalError("image") }
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon-1024.png"
let url = URL(fileURLWithPath: out)
guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
    fatalError("dest")
}
CGImageDestinationAddImage(dest, image, nil)
guard CGImageDestinationFinalize(dest) else { fatalError("write") }
print(out)
