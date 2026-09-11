import Foundation

// MARK: Rekordbox

struct Library: Codable, Equatable {
  var masterDbPath: String = ""
  var tracks: [Track] = []
  var playlists: [Playlist] = []

  /// Tracks keyed by file path, for joining a dropped file to its metadata.
  func index() -> [String: Track] {
    var byPath: [String: Track] = [:]
    for track in tracks {
      byPath[track.filePath] = track
      let resolved = track.fileURL.resolvingSymlinksInPath().path
      if resolved != track.filePath { byPath[resolved] = track }
    }
    return byPath
  }
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

struct TrackAnalysis: Codable {
  var beatsMs: [Float] = []
  var cues: [RekordboxCue] = []
  /// RGB triplets, flattened.
  var waveformRgb: [UInt8] = []
}

struct RekordboxCue: Codable, Equatable {
  var hotCue: UInt32
  var timeMs: UInt32
  var loopTimeMs: UInt32
  var isLoop: Bool
  var colorRgb: [UInt8]?
  var comment: String?
}

struct AudioDevice: Codable, Equatable, Hashable {
  var name: String
  var maxOutputChannels: UInt16
}

// MARK: Decks

/// What the console shows for a loaded track, derived once at load time.
struct LoadedTrack {
  var title: String
  var bpm: Float?
  /// Beat positions in audio frames.
  var beats: [Float]
  var cues: [CuePoint]
  /// Peak per ~5 ms, 0...1.
  var waveform: [Float]
  /// RGB triplets aligned with the rekordbox waveform; empty without analysis.
  var waveformColors: [UInt8]
  var totalFrames: Int

  var bpmText: String { bpm.map { String(format: "%.1f", $0) } ?? "" }
}

struct CuePoint: Equatable, Identifiable {
  var label: String
  /// Fraction of the track (0...1).
  var position: Float
  /// Fraction of the track (0...1) for loop cues.
  var loopEnd: Float?
  var colorRgb: [UInt8]?

  var id: String { "\(label)@\(position)" }

  /// Hot cues carry their number; memory cues are numbered M1, M2, … in
  /// time order. Cues outside the track are dropped.
  static func from(_ cues: [RekordboxCue], totalFrames: Int, sampleRate: Double) -> [CuePoint] {
    let durationMs = Float(Double(totalFrames) / max(sampleRate, 1) * 1000)
    guard durationMs > 0 else { return [] }
    var memoryIndex = 0
    return cues.compactMap { cue in
      let position = Float(cue.timeMs) / durationMs
      guard (0...1).contains(position) else { return nil }
      var loopEnd: Float? = nil
      if cue.isLoop, cue.loopTimeMs > cue.timeMs {
        let end = Float(cue.loopTimeMs) / durationMs
        if end <= 1 { loopEnd = end }
      }
      let label: String
      if cue.hotCue > 0 {
        label = String(cue.hotCue)
      } else {
        memoryIndex += 1
        label = "M\(memoryIndex)"
      }
      return CuePoint(label: label, position: position, loopEnd: loopEnd, colorRgb: cue.colorRgb)
    }
  }
}

/// Per-deck state read from the engine every frame.
struct DeckState: Equatable {
  var positionFrames: Double = 0
  var totalFrames: Double = 0
  var peak: Float = 0
  var peakHold: Float = 0
  var gain: Float = 1
  var bpm: Float = 0
  var loopStart: Float = 0
  var loopEnd: Float = 0
  var playing = false
  var cueEnabled = false
  var eqLow = false
  var eqMid = false
  var eqHigh = false
  var loopEnabled = false
  var loaded = false

  /// Which standard pad the current loop length matches, or 0.
  var loopBeats: Float {
    guard loopEnabled, bpm > 0, sampleRate > 0 else { return 0 }
    let beatInterval = sampleRate * 60 / bpm
    let beats = (loopEnd - loopStart) / beatInterval
    let standards: [Float] = [0.25, 0.5, 1, 2, 4, 8, 16, 32]
    return standards.min { abs($0 - beats) < abs($1 - beats) } ?? 0
  }
  var sampleRate: Float = 0
}

struct ConsoleSnapshot: Equatable {
  var decks: [DeckState] = [DeckState(), DeckState()]
  var masterTempo: Float = 130
  var crossfader: Float = 0.5
  var micPeak: Float = 0
  var micAvailable = false
  var micEnabled = false
  var isRecording = false
  var recElapsedSecs: UInt32 = 0
  var cpuPercent: Double = 0
  var memoryBytes: UInt64 = 0
}
