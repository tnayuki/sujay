import Foundation

/// The slow textual console state: titles, BPM text and cue points.
struct ConsoleText: Codable, Equatable {
  var deckA: DeckText = DeckText()
  var deckB: DeckText = DeckText()

  func deck(_ index: Int) -> DeckText { index == 0 ? deckA : deckB }
}

struct DeckText: Codable, Equatable {
  var title: String = ""
  var bpmText: String = ""
  var cues: [CuePoint] = []
}

struct CuePoint: Codable, Equatable, Identifiable {
  var label: String
  /// Fraction of the track (0...1).
  var position: Float
  /// Fraction of the track (0...1) for loop cues.
  var loopEnd: Float?
  var colorRgb: [UInt8]?

  var id: String { "\(label)@\(position)" }
}

struct Library: Codable, Equatable {
  var sourceLabel: String = ""
  var tracks: [Track] = []
  var playlists: [Playlist] = []
}

struct Track: Codable, Equatable, Identifiable, Hashable {
  var id: String
  var title: String
  var artist: String
  var album: String
  var bpm: Float?
  var durationSeconds: Float?
  var rating: Int32?
  var tags: String?
  var releaseDate: String?
  var filePath: String

  var fileURL: URL { URL(fileURLWithPath: filePath) }

  /// Rekordbox also lists streaming entries (`spotify:track:…`, Beatport
  /// links); only an absolute local path can be decoded.
  var isLocalFile: Bool { filePath.hasPrefix("/") }
}

struct Playlist: Codable, Equatable, Identifiable, Hashable {
  var id: String
  var name: String
  var parentId: String
  var isFolder: Bool
  var trackIds: [String]
}

struct AudioDevice: Codable, Equatable, Hashable {
  var name: String
  var maxOutputChannels: UInt16
}

/// Mirrors `sujay_core::state::PreferencesState`.
struct Preferences: Codable, Equatable {
  var audioDeviceId: String?
  var audioDevices: [AudioDevice] = []
  var mainChannels: [Int32?] = [0, 1]
  var cueChannels: [Int32?] = [nil, nil]
  var recordingDirectory: String = ""
  var recordingAutoCreateDirectory: Bool = true
  var recordingNamingStrategy: String = "timestamp"
  var recordingFormat: String = "wav"
  var oscEnabled: Bool = false
  var oscHost: String = "127.0.0.1"
  var oscPort: UInt16 = 9000
}
