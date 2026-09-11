import Foundation
import Observation

/// One deck's state for the views. What changes every frame — playhead and
/// meters — is deliberately not observable: a SwiftUI view that read it
/// re-evaluated sixty times a second, and each re-evaluation invalidated its
/// size, which sent a layout pass through `.fixedSize` parents all the way to
/// the root. That alone cost most of a core. The waveform and meter views are
/// NSViews that redraw on `ConsoleModel`'s frame tick instead.
@Observable
final class DeckModel {
  // Every frame while playing; read by the NSViews, never by SwiftUI bodies.
  @ObservationIgnored var positionFrames: Double = 0
  @ObservationIgnored var peak: Float = 0
  @ObservationIgnored var peakHold: Float = 0

  /// "M:SS" of the playhead; observable, changes once a second.
  var timeText = "0:00"

  // On load or a command.
  var track: LoadedTrack?
  var totalFrames: Double = 0
  var sampleRate: Float = 0
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

  var hasTrack: Bool { track != nil }

  /// Which standard pad the current loop length matches, or 0.
  var loopBeats: Float {
    guard loopEnabled, bpm > 0, sampleRate > 0 else { return 0 }
    let beatInterval = sampleRate * 60 / bpm
    let beats = (loopEnd - loopStart) / beatInterval
    let standards: [Float] = [0.25, 0.5, 1, 2, 4, 8, 16, 32]
    return standards.min { abs($0 - beats) < abs($1 - beats) } ?? 0
  }

  /// Copy what changed from the engine's state; untouched properties do not
  /// notify their observers.
  func apply(_ state: DeckState) {
    positionFrames = state.positionFrames
    peak = state.peak
    peakHold = state.peakHold
    let text: String
    if state.loaded, state.sampleRate > 0 {
      let seconds = Int(state.positionFrames / Double(state.sampleRate))
      text = String(format: "%d:%02d", seconds / 60, seconds % 60)
    } else {
      text = "0:00"
    }
    if timeText != text { timeText = text }
    if totalFrames != state.totalFrames { totalFrames = state.totalFrames }
    if sampleRate != state.sampleRate { sampleRate = state.sampleRate }
    if gain != state.gain { gain = state.gain }
    if bpm != state.bpm { bpm = state.bpm }
    if loopStart != state.loopStart { loopStart = state.loopStart }
    if loopEnd != state.loopEnd { loopEnd = state.loopEnd }
    if playing != state.playing { playing = state.playing }
    if cueEnabled != state.cueEnabled { cueEnabled = state.cueEnabled }
    if eqLow != state.eqLow { eqLow = state.eqLow }
    if eqMid != state.eqMid { eqMid = state.eqMid }
    if eqHigh != state.eqHigh { eqHigh = state.eqHigh }
    if loopEnabled != state.loopEnabled { loopEnabled = state.loopEnabled }
    if loaded != state.loaded { loaded = state.loaded }
  }
}
