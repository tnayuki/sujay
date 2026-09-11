import Foundation

/// Persisted settings, the same `settings.json` the Rust host wrote
/// (`~/Library/Application Support/Sujay/settings.json`, snake_case keys).
struct Preferences: Codable, Equatable {
  var audioDeviceId: String?
  var mainChannels: [Int32?] = [0, 1]
  var cueChannels: [Int32?] = [nil, nil]
  var recordingDirectory: String = Preferences.defaultRecordingDirectory
  var recordingAutoCreateDirectory = true
  var recordingNamingStrategy = "timestamp"
  var recordingFormat = "wav"
  var oscEnabled = false
  var oscHost = "127.0.0.1"
  var oscPort: UInt16 = 9000

  static let defaultRecordingDirectory: String = {
    let music =
      FileManager.default.urls(for: .musicDirectory, in: .userDomainMask).first
      ?? FileManager.default.homeDirectoryForCurrentUser
    return music.appendingPathComponent("Sujay Recordings").path
  }()

  static let fileURL: URL = {
    let support =
      FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
      ?? FileManager.default.homeDirectoryForCurrentUser
    return support.appendingPathComponent("Sujay/settings.json")
  }()

  var format: RecordingFormat { RecordingFormat(rawValue: recordingFormat) ?? .wav }

  /// The Rust engine wrote OGG Vorbis; AVFoundation writes AAC instead.
  private static func migrate(_ format: String) -> String { format == "ogg" ? "m4a" : format }

  static func load() -> Preferences {
    guard let data = try? Data(contentsOf: fileURL),
      let preferences = try? JSON.decoder.decode(Preferences.self, from: data)
    else { return Preferences() }
    return preferences
  }

  /// Write atomically next to the old file, then rename over it.
  func save() throws {
    let directory = Preferences.fileURL.deletingLastPathComponent()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let data = try JSON.encoder.encode(self)
    try data.write(to: Preferences.fileURL, options: .atomic)
  }

  /// Clamp channels to the selected device, drop duplicates, and fill any
  /// empty field with its default.
  mutating func normalize(devices: [AudioDevice]) {
    let selectedMax = max(
      Int32(devices.first { $0.name == audioDeviceId }?.maxOutputChannels ?? 2), 2)
    var used = Set<Int32>()
    for side in 0..<2 {
      if let channel = mainChannels[side],
        channel < 0 || channel >= selectedMax || !used.insert(channel).inserted
      {
        mainChannels[side] = nil
      }
    }
    for side in 0..<2 {
      if let channel = cueChannels[side],
        channel < 0 || channel >= selectedMax || !used.insert(channel).inserted
      {
        cueChannels[side] = nil
      }
    }
    if mainChannels[0] == nil, mainChannels[1] == nil {
      mainChannels[0] = 0
      if selectedMax > 1 { mainChannels[1] = 1 }
    }
    if recordingDirectory.trimmingCharacters(in: .whitespaces).isEmpty {
      recordingDirectory = Preferences.defaultRecordingDirectory
    }
    if recordingNamingStrategy != "timestamp", recordingNamingStrategy != "sequential" {
      recordingNamingStrategy = "timestamp"
    }
    recordingFormat = Preferences.migrate(recordingFormat)
    if RecordingFormat(rawValue: recordingFormat) == nil { recordingFormat = "wav" }
    if oscHost.trimmingCharacters(in: .whitespaces).isEmpty { oscHost = "127.0.0.1" }
    if oscPort == 0 { oscPort = 9000 }
  }

  /// The next free recording file under the configured directory.
  func recordingPath() throws -> String {
    let directory = URL(fileURLWithPath: recordingDirectory)
    guard directory.path.hasPrefix("/") else {
      throw PreferencesError.recording("recording directory must be an absolute path")
    }
    if !FileManager.default.fileExists(atPath: directory.path) {
      guard recordingAutoCreateDirectory else {
        throw PreferencesError.recording("recording directory not found: \(directory.path)")
      }
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    let ext = format.fileExtension
    if recordingNamingStrategy == "sequential" {
      for index in 1...9999 {
        let candidate = directory.appendingPathComponent(String(format: "%04d.%@", index, ext))
        if !FileManager.default.fileExists(atPath: candidate.path) { return candidate.path }
      }
      throw PreferencesError.recording("unable to allocate recording filename")
    }
    let base = "sujay_\(Int(Date().timeIntervalSince1970))"
    for suffix in 0...999 {
      let name = suffix == 0 ? "\(base).\(ext)" : "\(base)_\(suffix).\(ext)"
      let candidate = directory.appendingPathComponent(name)
      if !FileManager.default.fileExists(atPath: candidate.path) { return candidate.path }
    }
    throw PreferencesError.recording("unable to allocate timestamp recording filename")
  }
}

enum PreferencesError: Error, CustomStringConvertible {
  case recording(String)

  var description: String {
    switch self {
    case .recording(let message): message
    }
  }
}
