import Foundation

/// A second-order IIR section (RBJ cookbook), one channel of state.
struct Biquad {
  private var b0: Float = 1
  private var b1: Float = 0
  private var b2: Float = 0
  private var a1: Float = 0
  private var a2: Float = 0
  private var z1: Float = 0
  private var z2: Float = 0

  enum Kind {
    case lowpass
    case highpass
  }

  init(_ kind: Kind, frequency: Float, sampleRate: Float, q: Float = 0.70710678) {
    let w0 = 2 * Float.pi * frequency / sampleRate
    let cosw = cos(w0)
    let alpha = sin(w0) / (2 * q)
    let a0 = 1 + alpha
    switch kind {
    case .lowpass:
      b0 = (1 - cosw) / 2 / a0
      b1 = (1 - cosw) / a0
      b2 = b0
    case .highpass:
      b0 = (1 + cosw) / 2 / a0
      b1 = -(1 + cosw) / a0
      b2 = b0
    }
    a1 = -2 * cosw / a0
    a2 = (1 - alpha) / a0
  }

  @inline(__always)
  mutating func process(_ x: Float) -> Float {
    // Transposed direct form II.
    let y = b0 * x + z1
    z1 = b1 * x - a1 * y + z2
    z2 = b2 * x - a2 * y
    return y
  }

  mutating func reset() {
    z1 = 0
    z2 = 0
  }
}

/// The deck's 3-band kill EQ. Every crossover is Linkwitz-Riley 4th order —
/// two cascaded Butterworth biquads — at 250 Hz and 5 kHz, so the three bands
/// sum flat and a killed band's neighbour does not let it through: the
/// web-audio graph this replaces gave the mid band single biquads, which left
/// a low kill at −15 dB by 100 Hz and a high kill at −7 dB by 8 kHz. The
/// filters always run so a kill engages without a click.
struct DeckEQ {
  static let lowFrequency: Float = 250
  static let highFrequency: Float = 5000

  var killLow = false
  var killMid = false
  var killHigh = false

  private var lowA: [Biquad]
  private var lowB: [Biquad]
  private var midHPA: [Biquad]
  private var midHPB: [Biquad]
  private var midLPA: [Biquad]
  private var midLPB: [Biquad]
  private var highA: [Biquad]
  private var highB: [Biquad]

  init(sampleRate: Float) {
    let low = Biquad(.lowpass, frequency: Self.lowFrequency, sampleRate: sampleRate)
    let midHigh = Biquad(.highpass, frequency: Self.lowFrequency, sampleRate: sampleRate)
    let midLow = Biquad(.lowpass, frequency: Self.highFrequency, sampleRate: sampleRate)
    let high = Biquad(.highpass, frequency: Self.highFrequency, sampleRate: sampleRate)
    lowA = [low, low]
    lowB = [low, low]
    midHPA = [midHigh, midHigh]
    midHPB = [midHigh, midHigh]
    midLPA = [midLow, midLow]
    midLPB = [midLow, midLow]
    highA = [high, high]
    highB = [high, high]
  }

  /// In place on one channel's samples.
  mutating func process(channel: Int, _ samples: UnsafeMutablePointer<Float>, count: Int) {
    for i in 0..<count {
      let x = samples[i]
      let low = lowB[channel].process(lowA[channel].process(x))
      let band = midHPB[channel].process(midHPA[channel].process(x))
      let mid = midLPB[channel].process(midLPA[channel].process(band))
      let high = highB[channel].process(highA[channel].process(x))
      samples[i] = (killLow ? 0 : low) + (killMid ? 0 : mid) + (killHigh ? 0 : high)
    }
  }
}
