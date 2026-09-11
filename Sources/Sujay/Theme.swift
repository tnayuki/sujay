import SwiftUI

/// The console palette, carried over from the egui console (which carried it
/// over from the React CSS before that).
enum Theme {
  static func rgb(_ r: Double, _ g: Double, _ b: Double, _ alpha: Double = 1) -> Color {
    Color(red: r / 255, green: g / 255, blue: b / 255, opacity: alpha)
  }

  static let bgDark = rgb(26, 26, 26)
  static let bgDeep = rgb(15, 15, 15)
  static let panel = rgb(42, 42, 42)
  static let borderDim = rgb(68, 68, 68)
  static let borderMed = rgb(85, 85, 85)
  static let cyan = rgb(0, 212, 255)
  static let orange = rgb(255, 69, 0)
  static let orangeSec = rgb(255, 107, 53)
  static let green = rgb(0, 204, 102)
  static let textPrimary = rgb(224, 224, 224)
  static let textDim = rgb(208, 208, 208)
  static let textGray = rgb(170, 170, 170)
  static let waveformPlayed = rgb(74, 158, 255)
  static let waveformUnplayed = rgb(221, 221, 221)
  static let waveformEmpty = rgb(51, 51, 51)
  static let meterGreen = rgb(0, 255, 0)
  static let meterOrange = rgb(255, 136, 0)
  static let meterRed = rgb(255, 0, 0)
  static let buttonBG = rgb(51, 51, 51)
  static let buttonBGBottom = rgb(31, 31, 31)

  static func pixel(_ size: CGFloat) -> Font {
    .custom("PixelMplus12-Regular", size: size)
  }

  static func dseg7(_ size: CGFloat) -> Font {
    .custom("DSEG7Classic-Regular", size: size)
  }

  /// The 135° gradient the console uses for every raised surface.
  static func gradient135(_ top: Color, _ bottom: Color) -> LinearGradient {
    LinearGradient(colors: [top, bottom], startPoint: .topLeading, endPoint: .bottomTrailing)
  }
}

/// A raised console button: gradient face, 1 px border, nudged down while pressed.
struct ConsoleButtonStyle: ButtonStyle {
  var active = false
  var activeTop: Color = Theme.orange
  var activeBottom: Color = Theme.rgb(204, 55, 0)
  var activeBorder: Color = Theme.orange
  var cornerRadius: CGFloat = 3
  var glow: Color? = nil
  var glowRadius: CGFloat = 6

  func makeBody(configuration: Configuration) -> some View {
    let shape = RoundedRectangle(cornerRadius: cornerRadius)
    configuration.label
      .background(
        active
          ? Theme.gradient135(activeTop, activeBottom)
          : Theme.gradient135(Theme.buttonBG, Theme.buttonBGBottom)
      )
      .clipShape(shape)
      .overlay(shape.stroke(active ? activeBorder : Theme.borderMed, lineWidth: 1))
      .shadow(color: active ? (glow ?? .clear) : .clear, radius: glowRadius)
      .offset(y: configuration.isPressed ? 1 : 0)
      .contentShape(shape)
  }
}
