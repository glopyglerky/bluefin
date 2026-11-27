import SwiftUI

/// Semantic colors shared by the native library and playback views.
enum BluefinStyle {
    static let background = adaptive(light: 0xA3ACB1, dark: 0x061727)
    static let secondaryBackground = adaptive(light: 0xCCD0D3, dark: 0x102435)
    static let hover = adaptive(light: 0xBDC5C9, dark: 0x182D3D)
    static let text = adaptive(light: 0x061727, dark: 0xCCD0D3)
    static let secondaryText = adaptive(light: 0x2B4050, dark: 0xA3ACB1)
    static let accent = adaptive(light: 0x333B5A, dark: 0xADB5D1)
    static let brand = Color(red: 130.0 / 255, green: 137.0 / 255, blue: 165.0 / 255)
    static let controlBorder = adaptive(light: 0x465966, dark: 0x637889)
    static let decorativeLine = adaptive(light: 0x8A989F, dark: 0x2D4354)
    static let primaryFill = adaptive(light: 0x061727, dark: 0x8289A5)
    static let onPrimary = adaptive(light: 0xCCD0D3, dark: 0x061727)
    static let artworkRadius: CGFloat = 12
    static let contentWidth: CGFloat = 1200

    private static func uiColor(_ hex: UInt32) -> UIColor {
        UIColor(red: CGFloat((hex >> 16) & 255) / 255,
                green: CGFloat((hex >> 8) & 255) / 255,
                blue: CGFloat(hex & 255) / 255, alpha: 1)
    }

    private static func adaptiveUIColor(light: UInt32, dark: UInt32) -> UIColor {
        UIColor { traits in uiColor(traits.userInterfaceStyle == .dark ? dark : light) }
    }

    private static func adaptive(light: UInt32, dark: UInt32) -> Color {
        Color(uiColor: adaptiveUIColor(light: light, dark: dark))
    }

    /// Keep system list containers in the same palette as custom library views.
    @MainActor static func configureChrome() {
        let background = adaptiveUIColor(light: 0xA3ACB1, dark: 0x061727)
        UITableView.appearance().backgroundColor = background
        UITableViewCell.appearance().backgroundColor = .clear
        UICollectionView.appearance().backgroundColor = background
    }
}

struct BluefinBackground: View {
    var body: some View { BluefinStyle.background.ignoresSafeArea() }
}

/// Glass belongs to the floating controls; the media canvas stays quiet and solid.
struct BluefinGlassSurface: ViewModifier {
    var enabled = true
    var cornerRadius: CGFloat = 24
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @ViewBuilder func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        if !enabled { content }
        else if reduceTransparency || reduceMotion {
            content.background(BluefinStyle.secondaryBackground, in: shape)
                .overlay(shape.strokeBorder(BluefinStyle.controlBorder.opacity(0.3), lineWidth: 0.5))
        } else {
            #if !os(visionOS)
            if #available(iOS 26.0, tvOS 26.0, macOS 26.0, watchOS 26.0, *) {
                content.glassEffect(.regular, in: shape)
            } else {
                content.background(.regularMaterial, in: shape)
            }
            #else
            content.background(.regularMaterial, in: shape)
            #endif
        }
    }
}

struct BluefinMark: View {
    var body: some View {
        Canvas { context, size in
            context.translateBy(x: 0, y: size.height)
            context.scaleBy(x: size.width / 1024, y: -size.height / 1024)
            var fin = Path()
            fin.move(to: CGPoint(x: 225, y: 420))
            fin.addCurve(to: CGPoint(x: 620, y: 785),
                         control1: CGPoint(x: 388, y: 490), control2: CGPoint(x: 410, y: 680))
            fin.addCurve(to: CGPoint(x: 790, y: 420),
                         control1: CGPoint(x: 580, y: 625), control2: CGPoint(x: 670, y: 470))
            fin.addCurve(to: CGPoint(x: 225, y: 420),
                         control1: CGPoint(x: 560, y: 470), control2: CGPoint(x: 420, y: 337))
            fin.closeSubpath()
            context.fill(fin, with: .color(BluefinStyle.text))
            var waves = Path()
            for baseline in [300.0, 240.0] {
                waves.move(to: CGPoint(x: 235, y: baseline))
                waves.addCurve(to: CGPoint(x: 790, y: baseline + 12),
                               control1: CGPoint(x: 430, y: baseline - 65),
                               control2: CGPoint(x: 575, y: baseline + 80))
            }
            context.stroke(waves, with: .color(BluefinStyle.brand),
                           style: StrokeStyle(lineWidth: 20, lineCap: .round))
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }
}


private struct BluefinCategoryActiveKey: EnvironmentKey { static let defaultValue = true }
extension EnvironmentValues {
    var bluefinCategoryIsActive: Bool {
        get { self[BluefinCategoryActiveKey.self] }
        set { self[BluefinCategoryActiveKey.self] = newValue }
    }
}
