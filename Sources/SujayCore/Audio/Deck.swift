import AVFoundation
import Foundation
import os

/// One deck: the track's PCM, the playhead, loop and transport state, and
/// the per-deck signal chain (time stretch, kill EQ, gain). Commands run on
/// the main thread under `lock`; the render thread takes the lock with
/// `trylock` and plays silence for the callback if it cannot.
final class Deck {
  let lock = OSAllocatedUnfairLock()

  // Track — replaced whole on load.
  private(set) var pcm: [Float] = []  // interleaved stereo
  private(set) var totalFrames = 0
  private(set) var bpm: Float?
  private(set) var beats: [Float] = []  // audio frames
  private(set) var trackID = ""

  // Transport.
  var position = 0  // source frames
  var playing = false
  var loopEnabled = false
  var loopStart = 0
  var loopEnd = 0
  /// Playback rate = master tempo / track BPM, 1 when the BPM is unknown.
  private(set) var rate: Float = 1

  // Mix.
  /// Fader value 0...1; the applied gain is its square.
  var faderGain: Float = 1
  var gain: Float { faderGain == 0 ? 0 : faderGain * faderGain }
  var cueEnabled = false
  var eq: DeckEQ

  // Meters, written by the render thread.
  var peak: Float = 0
  var peakHold: Float = 0
  private var peakHoldSince = Date()

  private let stretcher: TimeStretcher?
  private let sampleRate: Double
  private var flushOnNextRender = false

  init?(sampleRate: Double, maximumFrames: Int) {
    self.sampleRate = sampleRate
    stretcher = TimeStretcher(sampleRate: sampleRate, maximumFrames: maximumFrames)
    eq = DeckEQ(sampleRate: Float(sampleRate))
    if stretcher == nil { return nil }
  }

  var hasTrack: Bool { totalFrames > 0 }

  /// Replace the track. Caller holds `lock`.
  func load(pcm: [Float], bpm: Float?, beats: [Float], trackID: String, masterTempo: Float) {
    self.pcm = pcm
    totalFrames = pcm.count / 2
    self.bpm = bpm
    self.beats = beats
    self.trackID = trackID
    position = 0
    playing = false
    loopEnabled = false
    loopStart = 0
    loopEnd = 0
    updateRate(masterTempo: masterTempo)
    flushOnNextRender = true
  }

  func updateRate(masterTempo: Float) {
    if let bpm, bpm > 0 {
      rate = min(max(masterTempo / bpm, 0.5), 2)
    } else {
      rate = 1
    }
    stretcher?.rate = rate
  }

  /// Move the playhead; caller holds `lock`.
  func seek(toFrame frame: Int) {
    position = min(max(frame, 0), max(totalFrames - 1, 0))
    flushOnNextRender = true
  }

  // MARK: Render

  /// Output-referred playhead: source frames consumed minus what the
  /// stretcher still holds.
  var playheadFrames: Int {
    guard let stretcher, playing else { return position }
    let held = Int(Float(stretcher.latencyFrames) * rate)
    return max(position - held, 0)
  }

  /// Render `frames` frames of this deck (post-EQ, pre-gain) into two channel
  /// buffers. Returns false when the deck is silent. Caller holds `lock`.
  func render(
    frames: Int, left: UnsafeMutablePointer<Float>, right: UnsafeMutablePointer<Float>,
    scratch: UnsafeMutablePointer<AudioBufferList>
  ) -> Bool {
    guard playing, totalFrames > 0, let stretcher else { return false }
    if flushOnNextRender {
      stretcher.reset()
      flushOnNextRender = false
    }
    let output = UnsafeMutableAudioBufferListPointer(scratch)
    output[0].mData = UnsafeMutableRawPointer(left)
    output[0].mDataByteSize = UInt32(frames * MemoryLayout<Float>.size)
    output[0].mNumberChannels = 1
    output[1].mData = UnsafeMutableRawPointer(right)
    output[1].mDataByteSize = UInt32(frames * MemoryLayout<Float>.size)
    output[1].mNumberChannels = 1

    var reachedEnd = false
    let rendered = stretcher.render(frames: frames, into: scratch) { input, count in
      self.pull(into: input, count: count, reachedEnd: &reachedEnd)
    }
    if !rendered { return false }
    if reachedEnd {
      playing = false
      position = 0
      flushOnNextRender = true
    }
    eq.process(channel: 0, left, count: frames)
    eq.process(channel: 1, right, count: frames)
    return true
  }

  /// Fill `count` source frames from the playhead, wrapping at the loop end
  /// sample-accurately and padding with silence past the end of the track.
  private func pull(
    into input: UnsafeMutablePointer<AudioBufferList>, count: Int, reachedEnd: inout Bool
  ) {
    let buffers = UnsafeMutableAudioBufferListPointer(input)
    guard buffers.count >= 2,
      let left = buffers[0].mData?.assumingMemoryBound(to: Float.self),
      let right = buffers[1].mData?.assumingMemoryBound(to: Float.self)
    else { return }
    var written = 0
    pcm.withUnsafeBufferPointer { source in
      while written < count {
        let limit = loopEnabled && loopEnd > loopStart ? min(loopEnd, totalFrames) : totalFrames
        if position >= limit {
          if loopEnabled && loopEnd > loopStart {
            position = loopStart
            continue
          }
          reachedEnd = true
          break
        }
        let run = min(count - written, limit - position)
        for i in 0..<run {
          left[written + i] = source[(position + i) * 2]
          right[written + i] = source[(position + i) * 2 + 1]
        }
        written += run
        position += run
      }
    }
    if written < count {
      for i in written..<count {
        left[i] = 0
        right[i] = 0
      }
    }
  }

  // MARK: Meters (render thread)

  /// Peak-hold as the Rust engine had it: hold 1.5 s, then decay 6 dB/s.
  func updateMeters(peak: Float) {
    self.peak = peak
    let now = Date()
    if peak > peakHold {
      peakHold = peak
      peakHoldSince = now
    } else {
      let held = now.timeIntervalSince(peakHoldSince)
      if held > 1.5 {
        let decayDb = Float(6 * (held - 1.5))
        guard peakHold > 0 else { return }
        let db = 20 * log10(peakHold) - decayDb
        peakHold = max(powf(10, db / 20), peak)
      }
    }
  }
}
