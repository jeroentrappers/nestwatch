// SPDX-License-Identifier: AGPL-3.0-or-later
// Renders the app icon (design/logo.svg on the night ink) as a 1024 px PNG:
//   swift apps/ios/scripts/render-icon.swift apps/ios/Nestwatch/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size = 1024
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.png"
let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: cs,
                    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}
ctx.setFillColor(rgb(0x0F1017))
ctx.fill(CGRect(x: 0, y: 0, width: size, height: size))
// SVG coordinates (64 × 64, y down) scaled to 78 % of the icon, centred.
let k = CGFloat(size) * 0.78 / 64
let off = CGFloat(size) * 0.11
ctx.translateBy(x: 0, y: CGFloat(size))
ctx.scaleBy(x: 1, y: -1)
ctx.translateBy(x: off, y: off)
ctx.scaleBy(x: k, y: k)
// Crescent: the big circle minus the offset one.
ctx.saveGState()
ctx.addRect(CGRect(x: -10, y: -10, width: 84, height: 84))
ctx.addEllipse(in: CGRect(x: 20, y: 4, width: 40, height: 40))
ctx.clip(using: .evenOdd)
ctx.setFillColor(rgb(0xF4BD7E))
ctx.fillEllipse(in: CGRect(x: 6, y: 8, width: 48, height: 48))
ctx.restoreGState()
ctx.setLineWidth(3.2)
ctx.setLineCap(.round)
ctx.setStrokeColor(rgb(0xB3A6FF))
ctx.move(to: CGPoint(x: 40, y: 38))
ctx.addCurve(to: CGPoint(x: 40, y: 27), control1: CGPoint(x: 43.5, y: 35.4), control2: CGPoint(x: 43.5, y: 29.6))
ctx.strokePath()
ctx.setStrokeColor(rgb(0xB3A6FF, 0.6))
ctx.move(to: CGPoint(x: 46, y: 43))
ctx.addCurve(to: CGPoint(x: 46, y: 21), control1: CGPoint(x: 53, y: 37.8), control2: CGPoint(x: 53, y: 26.2))
ctx.strokePath()
let img = ctx.makeImage()!
let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: out) as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(dest, img, nil)
CGImageDestinationFinalize(dest)
print("wrote \(out)")
