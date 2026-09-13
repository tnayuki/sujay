import Foundation

/// Persisted settings: one `UserDefaults` key each, in the app's own domain,
/// with the whole default set handed to `register(defaults:)` before the first
/// read. A setting added later reads its registered default, and a value that
/// is missing or of the wrong type costs only itself — where the
/// `settings.json` this replaces was read whole and fell back to the whole
/// default on any error. `defaults read com.tnayuki.sujay` shows them all.
struct Preferences: Equatable {
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

  /// The keys are the settings' public surface — what `defaults read` prints
  /// and what `defaults write` sets — so they are named, not snake_cased the
  /// way the file's JSON was.
  enum Key {
    static let audioDevice = "AudioDevice"
    static let mainChannels = "MainChannels"
    static let cueChannels = "CueChannels"
    static let recordingDirectory = "RecordingDirectory"
    static let recordingAutoCreateDirectory = "RecordingAutoCreateDirectory"
    static let recordingNamingStrategy = "RecordingNamingStrategy"
    static let recordingFormat = "RecordingFormat"
    static let oscEnabled = "OSCEnabled"
    static let oscHost = "OSCHost"
    static let oscPort = "OSCPort"
    static let importedSettingsFile = "ImportedSettingsFile"
  }

  static let defaultRecordingDirectory: String = {
    let music =
      FileManager.default.urls(for: .musicDirectory, in: .userDomainMask).first
      ?? FileManager.default.homeDirectoryForCurrentUser
    return music.appendingPathComponent("Sujay Recordings").path
  }()

  /// The settings file the Rust host wrote, and the Swift console after it.
  /// Imported once into the defaults domain, then left where it is.
  static let settingsFileURL: URL = {
    let support =
      FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
      ?? FileManager.default.homeDirectoryForCurrentUser
    return support.appendingPathComponent("Sujay/settings.json")
  }()

  /// A side of a channel pair that is off. `[Int32?]` is not a property-list
  /// type, so a pair is stored as two `Int`s with this for "none".
  static let unassignedChannel = -1

  var format: RecordingFormat { RecordingFormat(rawValue: recordingFormat) ?? .wav }

  /// The Rust engine wrote OGG Vorbis; AVFoundation writes AAC instead.
  private static func migrate(_ format: String) -> String { format == "ogg" ? "m4a" : format }

  // MARK: Storage

  static func load() -> Preferences {
    let defaults = UserDefaults.standard
    register(in: defaults)
    importSettingsFile(into: defaults)

    var preferences = Preferences()
    preferences.audioDeviceId = defaults.string(forKey: Key.audioDevice)
    preferences.mainChannels = channels(in: defaults, forKey: Key.mainChannels, fallback: [0, 1])
    preferences.cueChannels = channels(in: defaults, forKey: Key.cueChannels, fallback: [nil, nil])
    preferences.recordingDirectory =
      defaults.string(forKey: Key.recordingDirectory) ?? defaultRecordingDirectory
    preferences.recordingAutoCreateDirectory = defaults.bool(
      forKey: Key.recordingAutoCreateDirectory)
    preferences.recordingNamingStrategy =
      defaults.string(forKey: Key.recordingNamingStrategy) ?? "timestamp"
    preferences.recordingFormat = defaults.string(forKey: Key.recordingFormat) ?? "wav"
    preferences.oscEnabled = defaults.bool(forKey: Key.oscEnabled)
    preferences.oscHost = defaults.string(forKey: Key.oscHost) ?? "127.0.0.1"
    preferences.oscPort = UInt16(clamping: defaults.integer(forKey: Key.oscPort))
    return preferences
  }

  func save() {
    let defaults = UserDefaults.standard
    if let audioDeviceId {
      defaults.set(audioDeviceId, forKey: Key.audioDevice)
    } else {
      defaults.removeObject(forKey: Key.audioDevice)
    }
    defaults.set(Preferences.stored(mainChannels), forKey: Key.mainChannels)
    defaults.set(Preferences.stored(cueChannels), forKey: Key.cueChannels)
    defaults.set(recordingDirectory, forKey: Key.recordingDirectory)
    defaults.set(recordingAutoCreateDirectory, forKey: Key.recordingAutoCreateDirectory)
    defaults.set(recordingNamingStrategy, forKey: Key.recordingNamingStrategy)
    defaults.set(recordingFormat, forKey: Key.recordingFormat)
    defaults.set(oscEnabled, forKey: Key.oscEnabled)
    defaults.set(oscHost, forKey: Key.oscHost)
    defaults.set(Int(oscPort), forKey: Key.oscPort)
  }

  /// Every default in one place, so a setting the domain has never seen reads
  /// as if it had been written with its default.
  private static func register(in defaults: UserDefaults) {
    let initial = Preferences()
    defaults.register(defaults: [
      Key.mainChannels: stored(initial.mainChannels),
      Key.cueChannels: stored(initial.cueChannels),
      Key.recordingDirectory: initial.recordingDirectory,
      Key.recordingAutoCreateDirectory: initial.recordingAutoCreateDirectory,
      Key.recordingNamingStrategy: initial.recordingNamingStrategy,
      Key.recordingFormat: initial.recordingFormat,
      Key.oscEnabled: initial.oscEnabled,
      Key.oscHost: initial.oscHost,
      Key.oscPort: Int(initial.oscPort),
    ])
  }

  private static func stored(_ channels: [Int32?]) -> [Int] {
    channels.map { $0.map(Int.init) ?? unassignedChannel }
  }

  private static func channels(
    in defaults: UserDefaults, forKey key: String, fallback: [Int32?]
  ) -> [Int32?] {
    guard let raw = defaults.array(forKey: key) as? [Int], raw.count == fallback.count else {
      return fallback
    }
    return raw.map { $0 == unassignedChannel ? nil : Int32(exactly: $0) }
  }

  // MARK: Migration

  /// Read `settings.json` once into the defaults domain. The marker is written
  /// first: a file that cannot be read is still an import that has happened,
  /// and the file is never read again either way.
  private static func importSettingsFile(into defaults: UserDefaults) {
    guard !defaults.bool(forKey: Key.importedSettingsFile) else { return }
    defaults.set(true, forKey: Key.importedSettingsFile)
    guard let data = try? Data(contentsOf: settingsFileURL),
      let file = try? JSON.decoder.decode(SettingsFile.self, from: data)
    else { return }
    file.preferences.save()
  }

  /// The settings file's shape. Every property is Optional, so a file written
  /// by any past version imports what it does have.
  private struct SettingsFile: Decodable {
    var audioDeviceId: String?
    var mainChannels: [Int32?]?
    var cueChannels: [Int32?]?
    var recordingDirectory: String?
    var recordingAutoCreateDirectory: Bool?
    var recordingNamingStrategy: String?
    var recordingFormat: String?
    var oscEnabled: Bool?
    var oscHost: String?
    var oscPort: UInt16?

    var preferences: Preferences {
      var preferences = Preferences()
      preferences.audioDeviceId = audioDeviceId
      if let mainChannels, mainChannels.count == 2 { preferences.mainChannels = mainChannels }
      if let cueChannels, cueChannels.count == 2 { preferences.cueChannels = cueChannels }
      if let recordingDirectory { preferences.recordingDirectory = recordingDirectory }
      if let recordingAutoCreateDirectory {
        preferences.recordingAutoCreateDirectory = recordingAutoCreateDirectory
      }
      if let recordingNamingStrategy {
        preferences.recordingNamingStrategy = recordingNamingStrategy
      }
      if let recordingFormat { preferences.recordingFormat = recordingFormat }
      if let oscEnabled { preferences.oscEnabled = oscEnabled }
      if let oscHost { preferences.oscHost = oscHost }
      if let oscPort { preferences.oscPort = oscPort }
      return preferences
    }
  }

  // MARK: Values

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
