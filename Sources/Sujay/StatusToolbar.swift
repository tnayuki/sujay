import SwiftUI

/// Window toolbar: session recording, microphone talkover, and the host stats
/// that used to live in the custom titlebar.
struct StatusToolbar: ToolbarContent {
  @Environment(ConsoleModel.self) private var model

  private var recLabel: String {
    let secs = model.snapshot.rec_elapsed_secs
    guard model.snapshot.is_recording != 0 else { return "REC" }
    let h = secs / 3600
    let m = (secs % 3600) / 60
    let s = secs % 60
    return h > 0
      ? String(format: "%02d:%02d:%02d", h, m, s) : String(format: "%02d:%02d", m, s)
  }

  private var memText: String {
    let mb = model.snapshot.mem_mb
    return mb >= 1024 ? String(format: "%.1f GB", Double(mb) / 1024) : "\(mb) MB"
  }

  var body: some ToolbarContent {
    ToolbarItemGroup(placement: .automatic) {
      HStack(spacing: 12) {
        Label(String(format: "%.0f%%", model.snapshot.cpu_percent), systemImage: "cpu")
        Label(memText, systemImage: "memorychip")
        Text(model.clock)
      }
      .font(.callout.monospacedDigit())
      .foregroundStyle(.secondary)
    }
    ToolbarItemGroup(placement: .primaryAction) {
      ActiveButton(
        active: model.snapshot.mic_enabled != 0, tint: .green, action: { model.toggleMic() }
      ) {
        Label("MIC", systemImage: "mic.fill")
      }
      .disabled(model.snapshot.mic_available == 0)
      .help("Microphone talkover")

      ActiveButton(
        active: model.snapshot.is_recording != 0, tint: .red, action: { model.toggleRecording() }
      ) {
        Label(recLabel, systemImage: "record.circle")
          .monospacedDigit()
      }
      .help("Record the session")
    }
  }
}
