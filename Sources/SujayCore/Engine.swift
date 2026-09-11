import CSujay
import Foundation

/// The Rust audio engine (`crates/audio`) through its C ABI. One instance,
/// used from the main thread except for `start`, which enumerates CoreAudio
/// devices and must not run inside a SwiftUI update.
final class Engine {
  private let handle: OpaquePointer
  let sampleRate: Double

  /// Fails if CoreAudio refuses an output stream.
  init?(sampleRate: UInt32 = 44100) {
    guard let handle = sujay_engine_new(sampleRate) else { return nil }
    self.handle = handle
    self.sampleRate = Double(sujay_engine_sample_rate(handle))
  }

  deinit {
    sujay_engine_free(handle)
  }

  // MARK: Device

  /// `main` / `cue` are two channel indices each; nil disables a side.
  @discardableResult
  func configureDevice(deviceID: String?, main: [Int32?], cue: [Int32?]) -> Bool {
    let mainPair: [Int32] = (0..<2).map { side in (side < main.count ? main[side] : nil) ?? -1 }
    let cuePair: [Int32] = (0..<2).map { side in (side < cue.count ? cue[side] : nil) ?? -1 }
    return mainPair.withUnsafeBufferPointer { mainPtr in
      cuePair.withUnsafeBufferPointer { cuePtr in
        if let deviceID {
          return deviceID.withCString { name in
            sujay_engine_configure_device(handle, name, mainPtr.baseAddress, cuePtr.baseAddress)
          }
        }
        return sujay_engine_configure_device(handle, nil, mainPtr.baseAddress, cuePtr.baseAddress)
      }
    } == 0
  }

  static func outputDevices() -> [AudioDevice] {
    guard let pointer = sujay_list_output_devices_json() else { return [] }
    defer { sujay_string_free(pointer) }
    let data = Data(bytes: pointer, count: strlen(pointer))
    return (try? JSON.decoder.decode([AudioDevice].self, from: data)) ?? []
  }

  // MARK: Tracks

  /// Hand interleaved stereo PCM at the engine's sample rate to a deck.
  /// `beats` are audio frame indices. Returns false if the processing thread
  /// kept the engine busy for the whole retry window.
  func loadTrack(
    deck: UInt8, pcm: [Float], bpm: Float?, beats: [Float], trackID: String
  ) -> Bool {
    let frames = pcm.count / 2
    return pcm.withUnsafeBufferPointer { pcmPtr in
      beats.withUnsafeBufferPointer { beatsPtr in
        trackID.withCString { id in
          sujay_engine_load_track(
            handle, deck, pcmPtr.baseAddress, frames, bpm ?? 0, beatsPtr.baseAddress,
            beats.count, id)
        }
      }
    } == 0
  }

  // MARK: Commands (deck 1 = A, 2 = B)

  func play(_ deck: UInt8) { sujay_engine_play(handle, deck) }
  func stop(_ deck: UInt8) { sujay_engine_stop(handle, deck) }
  /// `position` is a fraction of the track (0...1).
  func seek(_ deck: UInt8, _ position: Double) { sujay_engine_seek(handle, deck, position) }
  func setCrossfader(_ position: Double) { sujay_engine_set_crossfader(handle, position) }
  func setMasterTempo(_ bpm: Double) { sujay_engine_set_master_tempo(handle, bpm) }
  func setDeckGain(_ deck: UInt8, _ gain: Double) { sujay_engine_set_deck_gain(handle, deck, gain) }
  func setEQ(_ deck: UInt8, _ band: EQBand, kill: Bool) {
    sujay_engine_set_eq(handle, deck, band.rawValue, kill)
  }
  func setCue(_ deck: UInt8, _ enabled: Bool) { sujay_engine_set_cue(handle, deck, enabled) }
  func setMicEnabled(_ enabled: Bool) { sujay_engine_set_mic_enabled(handle, enabled) }
  /// Loop bounds as fractions of the track (0...1).
  func setLoop(_ deck: UInt8, start: Double, end: Double) {
    sujay_engine_set_loop(handle, deck, start, end, true)
  }
  /// Loop bounds in seconds; the playhead is moved inside the loop.
  func setBeatLoop(_ deck: UInt8, startSeconds: Double, endSeconds: Double) {
    sujay_engine_set_beat_loop(handle, deck, startSeconds, endSeconds)
  }
  func clearLoop(_ deck: UInt8) { sujay_engine_clear_loop(handle, deck) }
  @discardableResult
  func startRecording(path: String, format: RecordingFormat) -> Bool {
    path.withCString { sujay_engine_start_recording(handle, $0, format == .ogg ? 1 : 0) } == 0
  }
  func stopRecording() { sujay_engine_stop_recording(handle) }

  // MARK: State

  func state() -> SujayEngineState {
    var out = SujayEngineState()
    sujay_engine_state(handle, &out)
    return out
  }
}

enum EQBand: UInt8, CustomStringConvertible {
  case low = 0
  case mid = 1
  case high = 2

  var description: String {
    switch self {
    case .low: "low"
    case .mid: "mid"
    case .high: "high"
    }
  }
}

enum RecordingFormat: String, Codable, CaseIterable {
  case wav
  case ogg

  var fileExtension: String { rawValue }
}

/// Shared JSON coding for everything that crosses from Rust (snake_case keys).
enum JSON {
  static let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
  }()

  static let encoder: JSONEncoder = {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    return encoder
  }()
}

extension DeckState {
  /// One deck out of the engine's POD state.
  init(_ c: SujayDeckState, sampleRate: Float) {
    self.init()
    positionFrames = c.position_frames
    totalFrames = c.total_frames
    peak = c.peak
    peakHold = c.peak_hold
    gain = c.gain
    bpm = c.bpm
    loopStart = c.loop_start
    loopEnd = c.loop_end
    playing = c.playing != 0
    cueEnabled = c.cue_enabled != 0
    eqLow = c.eq_low != 0
    eqMid = c.eq_mid != 0
    eqHigh = c.eq_high != 0
    loopEnabled = c.loop_enabled != 0
    loaded = c.loaded != 0
    self.sampleRate = sampleRate
  }
}
