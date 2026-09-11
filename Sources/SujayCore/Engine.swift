import CSujay
import Foundation

/// Main-thread wrapper over the Rust core (`crates/ffi`). Every call goes
/// through the C ABI in `Vendor/SujayCore/include/sujay.h`; nothing here is
/// thread-safe by design — the core is single-threaded by contract.
final class Engine {
  private let handle: OpaquePointer

  init() {
    handle = sujay_core_new()
  }

  deinit {
    sujay_core_free(handle)
  }

  /// Start the audio engine and the background library load.
  func start() -> Bool {
    sujay_core_start(handle) == 0
  }

  func shutdown() {
    sujay_core_shutdown(handle)
  }

  // MARK: Per frame

  func tick() -> SujayTick {
    sujay_core_tick(handle)
  }

  func snapshot() -> SujaySnapshot {
    var out = SujaySnapshot()
    sujay_core_snapshot(handle, &out)
    return out
  }

  // MARK: Slow state

  private static let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
  }()

  private static let encoder: JSONEncoder = {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    return encoder
  }()

  private func decode<T: Decodable>(
    _ type: T.Type, from producer: (OpaquePointer) -> UnsafeMutablePointer<CChar>?
  ) -> T? {
    guard let pointer = producer(handle) else { return nil }
    defer { sujay_string_free(pointer) }
    let data = Data(bytes: pointer, count: strlen(pointer))
    do {
      return try Engine.decoder.decode(type, from: data)
    } catch {
      NSLog("sujay: json decode failed for \(type): \(error)")
      return nil
    }
  }

  func consoleText() -> ConsoleText? {
    decode(ConsoleText.self, from: sujay_core_console_json)
  }

  func library() -> Library? {
    decode(Library.self, from: sujay_core_library_json)
  }

  func preferences() -> Preferences? {
    decode(Preferences.self, from: sujay_core_preferences_json)
  }

  /// Persist and apply edited preferences.
  @discardableResult
  func apply(_ preferences: Preferences) -> Bool {
    guard let data = try? Engine.encoder.encode(preferences),
      let json = String(data: data, encoding: .utf8)
    else { return false }
    return sujay_core_apply_preferences_json(handle, json) == 0
  }

  func refreshAudioDevices() {
    sujay_core_refresh_audio_devices(handle)
  }

  // MARK: Bulk buffers

  func deckBuffers(_ deck: UInt8) -> DeckBuffers {
    var buffers = DeckBuffers()
    let waveformCount = sujay_core_waveform_len(handle, deck)
    if waveformCount > 0 {
      buffers.waveform = [Float](unsafeUninitializedCapacity: waveformCount) { buffer, count in
        count = sujay_core_copy_waveform(handle, deck, buffer.baseAddress, waveformCount)
      }
    }
    let colorCount = sujay_core_waveform_colors_len(handle, deck)
    if colorCount > 0 {
      buffers.waveformColors = [UInt8](unsafeUninitializedCapacity: colorCount * 3) {
        buffer, count in
        count = sujay_core_copy_waveform_colors(handle, deck, buffer.baseAddress, colorCount) * 3
      }
    }
    let beatCount = sujay_core_beats_len(handle, deck)
    if beatCount > 0 {
      buffers.beats = [Float](unsafeUninitializedCapacity: beatCount) { buffer, count in
        count = sujay_core_copy_beats(handle, deck, buffer.baseAddress, beatCount)
      }
    }
    var intro: Float = 0
    var outro: Float = 0
    let flags = sujay_core_deck_markers(handle, deck, &intro, &outro)
    buffers.intro = flags & 1 != 0 ? intro : nil
    buffers.outro = flags & 2 != 0 ? outro : nil
    return buffers
  }

  // MARK: Commands

  func play(_ deck: UInt8) { sujay_core_play(handle, deck) }
  func stop(_ deck: UInt8) { sujay_core_stop(handle, deck) }
  func setCrossfader(_ position: Float) { sujay_core_set_crossfader(handle, position) }
  func setMasterTempo(_ bpm: Float) { sujay_core_set_master_tempo(handle, bpm) }
  func setDeckGain(_ deck: UInt8, _ gain: Float) { sujay_core_set_deck_gain(handle, deck, gain) }
  func setCue(_ deck: UInt8, _ enabled: Bool) { sujay_core_set_cue(handle, deck, enabled) }
  func setEQ(_ deck: UInt8, _ band: EQBand, kill: Bool) {
    sujay_core_set_eq(handle, deck, band.rawValue, kill)
  }
  /// `position` is a fraction of the track (0...1).
  func seek(_ deck: UInt8, _ position: Float) { sujay_core_seek(handle, deck, position) }
  func recallCue(_ deck: UInt8, position: Float, loopEnd: Float?) {
    sujay_core_recall_cue(handle, deck, position, loopEnd ?? -1)
  }
  /// `beats <= 0` clears the loop.
  func toggleLoop(_ deck: UInt8, beats: Float) { sujay_core_toggle_loop(handle, deck, beats) }
  func setMicEnabled(_ enabled: Bool) { sujay_core_set_mic_enabled(handle, enabled) }
  func startRecording() { sujay_core_start_recording(handle) }
  func stopRecording() { sujay_core_stop_recording(handle) }
  func loadFile(_ deck: UInt8, _ url: URL) {
    url.withUnsafeFileSystemRepresentation { path in
      guard let path else { return }
      sujay_core_load_file(handle, deck, path)
    }
  }
}

enum EQBand: UInt8 {
  case low = 0
  case mid = 1
  case high = 2
}

/// Per-deck buffers copied out of the core when a track loads. Positions are
/// audio frame indices.
struct DeckBuffers {
  var waveform: [Float] = []
  /// RGB triplets aligned with `waveform`; empty when the track has no
  /// rekordbox analysis.
  var waveformColors: [UInt8] = []
  var beats: [Float] = []
  var intro: Float?
  var outro: Float?
}
