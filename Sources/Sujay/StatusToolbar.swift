import SwiftUI

/// Window toolbar. Readings true of the whole window — the app's CPU and
/// memory footprint, the clock — sit at the trailing edge in the same
/// icon-and-digits idiom hukan uses, with the two actions after them.
struct StatusToolbar: ToolbarContent {
  @Environment(ConsoleModel.self) private var model

  private var recLabel: String {
    let secs = model.snapshot.recElapsedSecs
    guard model.snapshot.isRecording else { return "REC" }
    let h = secs / 3600
    let m = (secs % 3600) / 60
    let s = secs % 60
    return h > 0
      ? String(format: "%02d:%02d:%02d", h, m, s) : String(format: "%02d:%02d", m, s)
  }

  var body: some ToolbarContent {
    // macOS 26 wraps every item in a glass capsule; a readout is not a control
    // and goes without one, the way hukan's unbordered items do.
    if #available(macOS 26, *) {
      ToolbarItem(placement: .automatic) {
        FootprintReadout()
      }
      .sharedBackgroundVisibility(.hidden)
    } else {
      ToolbarItem(placement: .automatic) {
        FootprintReadout()
      }
    }
    ToolbarItemGroup(placement: .primaryAction) {
      ActiveButton(
        active: model.snapshot.micEnabled, tint: .green, action: { model.toggleMic() }
      ) {
        Label("Mic", systemImage: "mic.fill")
      }
      .disabled(!model.snapshot.micAvailable)
      .help("Microphone talkover")

      ActiveButton(
        active: model.snapshot.isRecording, tint: .red, action: { model.toggleRecording() }
      ) {
        Label(recLabel, systemImage: "record.circle")
          .monospacedDigit()
      }
      .help("Record the session")
    }
  }
}

/// `cpu NN%   memorychip N.N GB   clock HH:MM:SS`, secondary-tinted, monospaced
/// digits, at a fixed width so the items beside it hold still as a reading
/// gains or loses a digit. The percent is left-padded with figure spaces to
/// three digits for the same reason.
private struct FootprintReadout: View {
  @Environment(ConsoleModel.self) private var model

  private static let memoryFormatter: ByteCountFormatter = {
    let formatter = ByteCountFormatter()
    formatter.countStyle = .memory
    formatter.allowedUnits = [.useMB, .useGB]
    return formatter
  }()

  private var cpuText: String {
    let percent = Int(model.snapshot.cpuPercent.rounded())
    let digits = String(percent)
    return String(repeating: "\u{2007}", count: max(0, 3 - digits.count)) + digits + "%"
  }

  private var memoryText: String {
    Self.memoryFormatter.string(fromByteCount: Int64(model.snapshot.memoryBytes))
  }

  var body: some View {
    // One Text, not a stack: the toolbar draws a bare text item plain, like the
    // title, while a stack of views is treated as a control and gets a capsule.
    Text(
      "\(Image(systemName: "cpu")) \(cpuText)   \(Image(systemName: "memorychip")) \(memoryText)   \(Image(systemName: "clock")) \(model.clock)"
    )
    .font(.system(size: 11, weight: .medium).monospacedDigit())
    .foregroundStyle(.secondary)
    .lineLimit(1)
    .frame(width: 230, alignment: .leading)
    .help("Sujay — CPU \(cpuText.trimmingCharacters(in: .whitespaces)), memory \(memoryText)")
  }
}
