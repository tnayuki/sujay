import SwiftUI

/// 38 pt custom titlebar: title on the left after the traffic lights, then
/// REC, MIC, CPU, MEM and the clock from the right.
struct TitlebarView: View {
  @Environment(ConsoleModel.self) private var model

  private var recLabel: String {
    let secs = model.snapshot.rec_elapsed_secs
    guard model.snapshot.is_recording != 0 else { return "REC" }
    let h = secs / 3600
    let m = (secs % 3600) / 60
    let s = secs % 60
    return h > 0
      ? String(format: "REC %02d:%02d:%02d", h, m, s)
      : String(format: "REC %02d:%02d", m, s)
  }

  private var memText: String {
    let mb = model.snapshot.mem_mb
    return mb >= 1024 ? String(format: "%.1fG", Double(mb) / 1024) : "\(mb)M"
  }

  var body: some View {
    let recording = model.snapshot.is_recording != 0
    let micOn = model.snapshot.mic_enabled != 0
    HStack(spacing: 8) {
      Spacer().frame(width: 80)
      Text("Sujay")
        .font(Theme.pixel(13)).bold()
        .foregroundStyle(Theme.cyan)
      Spacer()

      PillButton(
        label: recLabel, active: recording, palette: recording ? .recActive : .recIdle,
        enabled: true
      ) {
        model.toggleRecording()
      }
      sectionSeparator
      PillButton(
        label: "MIC", active: micOn, palette: micOn ? .micActive : .micIdle,
        enabled: model.snapshot.mic_available != 0
      ) {
        model.toggleMic()
      }
      levelBar(
        value: CGFloat(model.snapshot.mic_peak), width: 56, height: 6,
        low: Theme.rgb(0x21, 0xd4, 0xfd), high: Theme.rgb(0xff, 0x6b, 0x6b))
      sectionSeparator
      Text("CPU").font(.system(size: 10)).foregroundStyle(Theme.textGray)
      levelBar(
        value: CGFloat(model.snapshot.cpu_percent / 100), width: 50, height: 8,
        low: Theme.rgb(0x4a, 0x9e, 0xff), high: Theme.rgb(0xff, 0x6b, 0x6b))
      Text(String(format: "%.1f%%", model.snapshot.cpu_percent))
        .font(.system(size: 10, design: .monospaced)).foregroundStyle(Theme.textGray)
      thinSeparator
      Text("MEM").font(.system(size: 10)).foregroundStyle(Theme.textGray)
      Text(memText)
        .font(.system(size: 10, design: .monospaced)).foregroundStyle(Theme.textGray)
      thinSeparator
      Text(model.clock)
        .font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.textGray)
      Spacer().frame(width: 12)
    }
    .frame(height: 38)
    .background(
      LinearGradient(
        colors: [Theme.rgb(0x1a, 0x1a, 0x1a), Theme.rgb(0x0f, 0x0f, 0x0f)],
        startPoint: .top, endPoint: .bottom)
    )
    .overlay(alignment: .bottom) {
      Rectangle().fill(Theme.cyan).frame(height: 1)
        .shadow(color: Theme.cyan.opacity(0.5), radius: 2, y: 1)
    }
    .contentShape(Rectangle())
    .gesture(WindowDragGesture())
  }

  private var thinSeparator: some View {
    Rectangle().fill(Theme.rgb(255, 255, 255, 0.12)).frame(width: 1, height: 14)
  }

  private var sectionSeparator: some View {
    Rectangle().fill(Theme.rgb(255, 255, 255, 0.25)).frame(width: 1, height: 20)
      .padding(.horizontal, 4)
  }

  private func levelBar(value: CGFloat, width: CGFloat, height: CGFloat, low: Color, high: Color)
    -> some View
  {
    let fill = width * min(max(value, 0), 1)
    return ZStack(alignment: .leading) {
      Capsule().fill(Theme.rgb(255, 255, 255, 0.12))
      if fill > 1 {
        LinearGradient(colors: [low, high], startPoint: .leading, endPoint: .trailing)
          .frame(width: fill)
      }
    }
    .frame(width: width, height: height)
    .clipShape(Capsule())
  }
}

struct PillPalette {
  var border: Color
  var dot: Color
  var text: Color
  var top: Color
  var bottom: Color

  static let recActive = PillPalette(
    border: Theme.rgb(0xff, 0x4d, 0x4d), dot: Theme.rgb(0xff, 0x3b, 0x3b),
    text: Theme.rgb(0xff, 0xd2, 0xd2), top: Theme.rgb(0x4a, 0x1e, 0x1e),
    bottom: Theme.rgb(0x2d, 0x0e, 0x0e))
  static let recIdle = PillPalette(
    border: Theme.rgb(0xb0, 0x30, 0x30), dot: Theme.rgb(0x55, 0x33, 0x33),
    text: Theme.rgb(0xff, 0x85, 0x85), top: Theme.rgb(0x36, 0x18, 0x18),
    bottom: Theme.rgb(0x22, 0x0b, 0x0b))
  static let micActive = PillPalette(
    border: Theme.rgb(0x4d, 0xff, 0x4d), dot: Theme.rgb(0x3b, 0xff, 0x3b),
    text: Theme.rgb(0xd2, 0xff, 0xd2), top: Theme.rgb(0x20, 0x3a, 0x26),
    bottom: Theme.rgb(0x12, 0x20, 0x1a))
  static let micIdle = PillPalette(
    border: Theme.rgb(0x2a, 0x60, 0x30), dot: Theme.rgb(0x33, 0x55, 0x33),
    text: Theme.rgb(0x70, 0xb0, 0x80), top: Theme.rgb(0x1a, 0x36, 0x20),
    bottom: Theme.rgb(0x0f, 0x1f, 0x12))
}

struct PillButton: View {
  let label: String
  let active: Bool
  let palette: PillPalette
  let enabled: Bool
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      HStack(spacing: 5) {
        Circle().fill(palette.dot).frame(width: 6, height: 6)
          .shadow(color: active ? palette.dot : .clear, radius: 3)
        Text(label)
          .font(Theme.pixel(10))
          .foregroundStyle(palette.text)
          .monospacedDigit()
      }
      .padding(.horizontal, 9)
      .frame(height: 20)
      .background(Theme.gradient135(palette.top, palette.bottom))
      .clipShape(Capsule())
      .overlay(Capsule().stroke(palette.border, lineWidth: 1))
      .shadow(color: active ? palette.border.opacity(0.5) : .clear, radius: 4)
    }
    .buttonStyle(.plain)
    .disabled(!enabled)
    .opacity(enabled ? 1 : 0.45)
  }
}
