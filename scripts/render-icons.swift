// Renders the Grapevine "Trellis" icon assets (a grape bunch drawn as a git
// merge graph):
//   tray.png         - menubar template glyph, pure black + alpha
//   appicon-1024.png - macOS app icon (squircle + gradient + violet glyph)
//
// To regenerate the shipped icons:
//   swift scripts/render-icons.swift /tmp/out
//   cp /tmp/out/tray.png src-tauri/icons/tray.png
//   npm run tauri icon -- /tmp/out/appicon-1024.png   # then delete the
//   android/ and ios/ dirs it emits; this app bundles for macOS only

import SwiftUI
import AppKit

// MARK: - Glyph geometry (24x24 design units)

let strokeWidth: CGFloat = 1.7
let dotRadius: CGFloat = 2.4

let segments: [[CGPoint]] = [
    [CGPoint(x: 12, y: 1.9), CGPoint(x: 12, y: 4.6)],                            // stem tick
    [CGPoint(x: 7.4, y: 8.6), CGPoint(x: 12, y: 4.6), CGPoint(x: 16.6, y: 8.6)], // top vee
    [CGPoint(x: 7.4, y: 8.6), CGPoint(x: 9.4, y: 13.8)],                         // left rail
    [CGPoint(x: 16.6, y: 8.6), CGPoint(x: 14.6, y: 13.8)],                       // right rail
    [CGPoint(x: 9.4, y: 13.8), CGPoint(x: 12, y: 18.4)],                         // merge left
    [CGPoint(x: 14.6, y: 13.8), CGPoint(x: 12, y: 18.4)],                        // merge right
]

let dots: [CGPoint] = [
    CGPoint(x: 7.4, y: 8.6), CGPoint(x: 16.6, y: 8.6),
    CGPoint(x: 9.4, y: 13.8), CGPoint(x: 14.6, y: 13.8),
    CGPoint(x: 12, y: 18.4),
]

// Ink extents in design units: x 5.0...19.0, y 1.05...20.8
// (left/right from dot radius; top from stem round cap; bottom from merge dot)
let inkRect = CGRect(x: 5.0, y: 1.05, width: 14.0, height: 19.75)

struct GlyphTransform {
    let scale: CGFloat
    let offset: CGPoint // canvas position of design-space origin

    func apply(_ p: CGPoint) -> CGPoint {
        CGPoint(x: p.x * scale + offset.x, y: p.y * scale + offset.y)
    }
}

struct GlyphView: View {
    let t: GlyphTransform
    let color: Color

    var body: some View {
        var strokes = Path()
        for seg in segments {
            strokes.move(to: t.apply(seg[0]))
            for p in seg.dropFirst() { strokes.addLine(to: t.apply(p)) }
        }
        var fills = Path()
        for d in dots {
            let c = t.apply(d)
            let r = dotRadius * t.scale
            fills.addEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
        }
        return ZStack {
            strokes.stroke(color, style: StrokeStyle(
                lineWidth: strokeWidth * t.scale, lineCap: .round, lineJoin: .round))
            fills.fill(color)
        }
    }
}

// MARK: - Tray template image

// tray-icon scales the image to 18pt menubar height, so one high-res PNG
// serves 1x and 2x. Canvas hugs the ink with 0.5u padding: the glyph then
// renders at ~17.1pt of the 18pt image height.
let trayPad: CGFloat = 0.5
let trayScale: CGFloat = 8
let traySize = CGSize(width: (inkRect.width + 2 * trayPad) * trayScale,
                      height: (inkRect.height + 2 * trayPad) * trayScale)

struct TrayIcon: View {
    var body: some View {
        GlyphView(
            t: GlyphTransform(scale: trayScale,
                              offset: CGPoint(x: (trayPad - inkRect.minX) * trayScale,
                                              y: (trayPad - inkRect.minY) * trayScale)),
            color: .black)
        .frame(width: traySize.width, height: traySize.height)
    }
}

// MARK: - App icon

let canvas: CGFloat = 1024
let bodySize: CGFloat = 824       // Apple icon grid: artwork square inside 1024 canvas
let bodyCornerRadius: CGFloat = 185.4 // Apple icon grid corner radius at this size

struct AppIcon: View {
    var body: some View {
        // Glyph design box at 56% of the icon width; ink bbox centered in the squircle.
        let scale = canvas * 0.56 / 24.0
        let inkCenter = CGPoint(x: inkRect.midX * scale, y: inkRect.midY * scale)

        ZStack {
            RoundedRectangle(cornerRadius: bodyCornerRadius, style: .continuous)
                .fill(LinearGradient(
                    colors: [Color(red: 0x2E / 255.0, green: 0x28 / 255.0, blue: 0x39 / 255.0),
                             Color(red: 0x19 / 255.0, green: 0x15 / 255.0, blue: 0x21 / 255.0)],
                    startPoint: UnitPoint(x: 0.42, y: 0), endPoint: UnitPoint(x: 0.58, y: 1)))
                .frame(width: bodySize, height: bodySize)
                .shadow(color: .black.opacity(0.3), radius: 22, y: 11)
            GlyphView(
                t: GlyphTransform(scale: scale,
                                  offset: CGPoint(x: canvas / 2 - inkCenter.x,
                                                  y: canvas / 2 - inkCenter.y)),
                color: Color(red: 0xB3 / 255.0, green: 0x93 / 255.0, blue: 0xEA / 255.0))
        }
        .frame(width: canvas, height: canvas)
    }
}

// MARK: - Render

@MainActor
func writePNG(_ view: some View, to url: URL) {
    let renderer = ImageRenderer(content: view)
    renderer.scale = 1.0
    guard let cg = renderer.cgImage else { fatalError("render failed for \(url.lastPathComponent)") }
    let rep = NSBitmapImageRep(cgImage: cg)
    guard let data = rep.representation(using: .png, properties: [:]) else {
        fatalError("png encode failed for \(url.lastPathComponent)")
    }
    try! data.write(to: url)
    print("wrote \(url.path) (\(cg.width)x\(cg.height))")
}

MainActor.assumeIsolated {
    let outDir = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")
    writePNG(TrayIcon(), to: outDir.appendingPathComponent("tray.png"))
    writePNG(AppIcon(), to: outDir.appendingPathComponent("appicon-1024.png"))
}
