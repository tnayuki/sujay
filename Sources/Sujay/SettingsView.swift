import SwiftUI

/// Preferences window (⌘,): Audio / Recording / OSC. Edits are held locally
/// and applied to the core on Save.
struct SettingsView: View {
  @Environment(ConsoleModel.self) private var model
  @State private var draft = Preferences()
  @State private var loaded = false

  var body: some View {
    VStack(spacing: 12) {
      TabView {
        audioTab.tabItem { Text("Audio") }
        recordingTab.tabItem { Text("Recording") }
        oscTab.tabItem { Text("OSC") }
      }
      HStack {
        Spacer()
        Button("Revert") { draft = model.preferences }
        Button("Save") { model.applyPreferences(draft) }
          .keyboardShortcut(.defaultAction)
      }
    }
    .padding(16)
    .frame(width: 520, height: 380)
    .onAppear {
      model.refreshAudioDevices()
      draft = model.preferences
      loaded = true
    }
    .onChange(of: model.preferences) { _, next in
      if !loaded { draft = next }
    }
  }

  private var selectedDeviceChannels: Int {
    let device = model.audioDevices.first { $0.name == draft.audioDeviceId }
    return max(Int(device?.maxOutputChannels ?? 2), 2)
  }

  private var audioTab: some View {
    Form {
      Picker("Output device", selection: $draft.audioDeviceId) {
        Text("System default").tag(String?.none)
        ForEach(model.audioDevices, id: \.name) { device in
          Text(device.name).tag(String?.some(device.name))
        }
      }
      channelPair("Main", channels: $draft.mainChannels)
      channelPair("Cue", channels: $draft.cueChannels)
    }
    .formStyle(.grouped)
  }

  private func channelPair(_ label: String, channels: Binding<[Int32?]>) -> some View {
    HStack {
      Text(label)
      ForEach(0..<2, id: \.self) { side in
        Picker(side == 0 ? "L" : "R", selection: channels[side]) {
          Text("-").tag(Int32?.none)
          ForEach(0..<selectedDeviceChannels, id: \.self) { channel in
            Text("\(channel + 1)").tag(Int32?.some(Int32(channel)))
          }
        }
        .frame(width: 90)
      }
    }
  }

  private var recordingTab: some View {
    Form {
      TextField("Directory", text: $draft.recordingDirectory)
        .autocorrectionDisabled()
      Toggle("Create directory if missing", isOn: $draft.recordingAutoCreateDirectory)
      Picker("File names", selection: $draft.recordingNamingStrategy) {
        Text("Timestamp").tag("timestamp")
        Text("Sequential").tag("sequential")
      }
      Picker("Format", selection: $draft.recordingFormat) {
        Text("WAV").tag("wav")
        Text("AAC (.m4a)").tag("m4a")
      }
    }
    .formStyle(.grouped)
  }

  private var oscTab: some View {
    Form {
      Toggle("Enable OSC broadcasting", isOn: $draft.oscEnabled)
      TextField("Host", text: $draft.oscHost)
      TextField(
        "Port",
        value: Binding(
          get: { Int(draft.oscPort) },
          set: { draft.oscPort = UInt16(min(max($0, 0), 65535)) }),
        format: .number)
    }
    .formStyle(.grouped)
  }
}
