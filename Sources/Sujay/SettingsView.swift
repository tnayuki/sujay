import AppKit
import SwiftUI

/// Preferences window (⌘,): Audio / Recording / OSC as toolbar tabs, each a
/// grouped form sized to its own rows, so the window follows the tab the way a
/// Mac settings window does. Each control writes its setting through as it is
/// changed, so there is nothing to save. The text fields are the exception:
/// they commit on Return or when they lose focus, because a half-typed host or
/// an empty folder would be normalized away under the caret.
struct SettingsView: View {
  @Environment(ConsoleModel.self) private var model

  @State private var recordingDirectory = ""
  @State private var oscHost = ""
  @State private var oscPort = ""
  @FocusState private var focus: Field?

  private enum Field { case recordingDirectory, oscHost, oscPort }

  private static let width: CGFloat = 480

  /// Every control edits this: the setter normalizes, persists and applies.
  private var settings: Binding<Preferences> {
    Binding(get: { model.preferences }, set: { model.applyPreferences($0) })
  }

  var body: some View {
    TabView {
      audioTab
        .tabItem { Label("Audio", systemImage: "speaker.wave.2") }
      recordingTab
        .tabItem { Label("Recording", systemImage: "record.circle") }
      oscTab
        .tabItem { Label("OSC", systemImage: "antenna.radiowaves.left.and.right") }
    }
    .onAppear {
      model.refreshAudioDevices()
      revertFields()
    }
    .onChange(of: model.preferences) { _, _ in revertFields() }
    .onChange(of: focus) { previous, _ in commit(previous) }
    // A window closed with the caret still in a field commits what it holds.
    .onDisappear { commit(focus) }
  }

  // MARK: Audio

  private var selectedDeviceChannels: Int {
    let device = model.audioDevices.first { $0.name == model.preferences.audioDeviceId }
    return max(Int(device?.maxOutputChannels ?? 2), 2)
  }

  private var audioTab: some View {
    Form {
      Section("Output") {
        Picker("Device", selection: settings.audioDeviceId) {
          Text("System Default").tag(String?.none)
          ForEach(model.audioDevices, id: \.name) { device in
            Text(device.name).tag(String?.some(device.name))
          }
        }
      }
      Section("Channels") {
        channelPicker("Main Left", channel: settings.mainChannels[0])
        channelPicker("Main Right", channel: settings.mainChannels[1])
        channelPicker("Cue Left", channel: settings.cueChannels[0])
        channelPicker("Cue Right", channel: settings.cueChannels[1])
      }
    }
    .formStyle(.grouped)
    .frame(width: Self.width, height: 340)
  }

  private func channelPicker(_ label: String, channel: Binding<Int32?>) -> some View {
    Picker(label, selection: channel) {
      Text("None").tag(Int32?.none)
      ForEach(0..<selectedDeviceChannels, id: \.self) { index in
        Text("\(index + 1)").tag(Int32?.some(Int32(index)))
      }
    }
  }

  // MARK: Recording

  private var recordingTab: some View {
    Form {
      Section("Location") {
        LabeledContent("Folder") {
          HStack {
            TextField("Folder", text: $recordingDirectory)
              .labelsHidden()
              .autocorrectionDisabled()
              .focused($focus, equals: .recordingDirectory)
              .onSubmit { commit(.recordingDirectory) }
            Button("Choose…", action: chooseRecordingDirectory)
          }
        }
        Toggle("Create the folder if it is missing", isOn: settings.recordingAutoCreateDirectory)
      }
      Section("Files") {
        Picker("Name", selection: settings.recordingNamingStrategy) {
          Text("Timestamp").tag("timestamp")
          Text("Sequential").tag("sequential")
        }
        Picker("Format", selection: settings.recordingFormat) {
          Text("WAV").tag("wav")
          Text("AAC (.m4a)").tag("m4a")
        }
      }
    }
    .formStyle(.grouped)
    .frame(width: Self.width, height: 300)
  }

  /// The standard folder panel, starting where the setting points if that
  /// folder exists.
  private func chooseRecordingDirectory() {
    commit(focus)
    let panel = NSOpenPanel()
    panel.canChooseFiles = false
    panel.canChooseDirectories = true
    panel.canCreateDirectories = true
    panel.allowsMultipleSelection = false
    panel.prompt = "Choose"
    panel.message = "Choose the folder recordings are saved to."
    let current = URL(fileURLWithPath: model.preferences.recordingDirectory)
    if FileManager.default.fileExists(atPath: current.path) { panel.directoryURL = current }
    guard panel.runModal() == .OK, let url = panel.url else { return }
    var edited = model.preferences
    edited.recordingDirectory = url.path
    model.applyPreferences(edited)
    recordingDirectory = model.preferences.recordingDirectory
  }

  // MARK: OSC

  private var oscTab: some View {
    Form {
      Section {
        Toggle("Broadcast over OSC", isOn: settings.oscEnabled)
        TextField("Host", text: $oscHost)
          .autocorrectionDisabled()
          .focused($focus, equals: .oscHost)
          .onSubmit { commit(.oscHost) }
          .disabled(!model.preferences.oscEnabled)
        TextField("Port", text: $oscPort)
          .multilineTextAlignment(.trailing)
          .focused($focus, equals: .oscPort)
          .onSubmit { commit(.oscPort) }
          .disabled(!model.preferences.oscEnabled)
      }
    }
    .formStyle(.grouped)
    .frame(width: Self.width, height: 190)
  }

  // MARK: Text fields

  /// Show what the settings hold, leaving whatever is being typed alone.
  private func revertFields() {
    if focus != .recordingDirectory { recordingDirectory = model.preferences.recordingDirectory }
    if focus != .oscHost { oscHost = model.preferences.oscHost }
    if focus != .oscPort { oscPort = String(model.preferences.oscPort) }
  }

  /// Apply one text field, then show what normalizing made of it.
  private func commit(_ field: Field?) {
    var edited = model.preferences
    switch field {
    case .recordingDirectory:
      edited.recordingDirectory = recordingDirectory.trimmingCharacters(in: .whitespaces)
    case .oscHost:
      edited.oscHost = oscHost.trimmingCharacters(in: .whitespaces)
    case .oscPort:
      guard let port = Int(oscPort.trimmingCharacters(in: .whitespaces)) else { break }
      edited.oscPort = UInt16(clamping: port)
    case .none:
      return
    }
    model.applyPreferences(edited)
    // Not `revertFields`: the field may still hold the focus, and what it
    // shows now has to be what normalizing settled on.
    switch field {
    case .recordingDirectory: recordingDirectory = model.preferences.recordingDirectory
    case .oscHost: oscHost = model.preferences.oscHost
    case .oscPort: oscPort = String(model.preferences.oscPort)
    case .none: break
    }
  }
}
