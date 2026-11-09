import SwiftUI

/// Original vector artwork for the bundled preview. No remote artwork or account is used.
struct PreviewArtwork: View {
    let music: Bool
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                LinearGradient(colors: music
                    ? [Color(red: 0.20, green: 0.08, blue: 0.25), Color(red: 0.72, green: 0.23, blue: 0.29)]
                    : [Color(red: 0.02, green: 0.12, blue: 0.22), Color(red: 0.05, green: 0.44, blue: 0.48)],
                    startPoint: .topLeading, endPoint: .bottomTrailing)
                Circle().fill(Color.white.opacity(0.8))
                    .frame(width: geometry.size.width * 0.17, height: geometry.size.width * 0.17)
                    .position(x: geometry.size.width * 0.72, y: geometry.size.height * 0.28)
                Canvas { context, size in
                    for layer in 0..<5 {
                        let y = size.height * (0.48 + Double(layer) * 0.11)
                        var wave = Path()
                        wave.move(to: CGPoint(x: 0, y: y))
                        wave.addCurve(to: CGPoint(x: size.width, y: y - size.height * 0.06),
                                      control1: CGPoint(x: size.width * 0.35, y: y - size.height * 0.24),
                                      control2: CGPoint(x: size.width * 0.62, y: y + size.height * 0.20))
                        wave.addLine(to: CGPoint(x: size.width, y: size.height))
                        wave.addLine(to: CGPoint(x: 0, y: size.height)); wave.closeSubpath()
                        let color = music
                            ? Color(red: 0.96, green: 0.55 + Double(layer) * 0.05, blue: 0.43)
                            : Color(red: 0.13, green: 0.65 + Double(layer) * 0.06, blue: 0.69)
                        context.fill(wave, with: .color(color.opacity(0.4 + Double(layer) * 0.1)))
                    }
                }
            }.clipped()
        }.accessibilityHidden(true)
    }
}
