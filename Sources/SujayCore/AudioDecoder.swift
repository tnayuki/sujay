import AVFoundation
import Foundation

/// Decodes any file AVFoundation reads into interleaved stereo Float32 at
/// the engine's sample rate, plus the peak-per-chunk waveform the console
/// draws. Blocking; run off the main thread.
enum AudioDecoder {
  struct Decoded {
    /// Interleaved stereo samples.
    var pcm: [Float]
    var frames: Int { pcm.count / 2 }
    /// One peak per `waveformStep` frames (~200 per second).
    var waveform: [Float]
  }

  enum Failure: Error, CustomStringConvertible {
    case format
    case convert(String)

    var description: String {
      switch self {
      case .format: "unsupported audio format"
      case .convert(let message): message
      }
    }
  }

  static func decode(_ url: URL, sampleRate: Double) throws -> Decoded {
    let file = try AVAudioFile(forReading: url)
    guard
      let target = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 2, interleaved: true),
      let converter = AVAudioConverter(from: file.processingFormat, to: target)
    else { throw Failure.format }

    let ratio = sampleRate / file.processingFormat.sampleRate
    var pcm: [Float] = []
    pcm.reserveCapacity(Int(Double(file.length) * ratio) * 2 + 4096)

    let chunk: AVAudioFrameCount = 32768
    var reachedEnd = false
    let input: AVAudioConverterInputBlock = { count, status in
      if reachedEnd {
        status.pointee = .endOfStream
        return nil
      }
      guard
        let buffer = AVAudioPCMBuffer(
          pcmFormat: file.processingFormat, frameCapacity: min(count, chunk))
      else {
        status.pointee = .endOfStream
        return nil
      }
      do {
        try file.read(into: buffer)
      } catch {
        reachedEnd = true
        status.pointee = .endOfStream
        return nil
      }
      if buffer.frameLength == 0 {
        reachedEnd = true
        status.pointee = .endOfStream
        return nil
      }
      status.pointee = .haveData
      return buffer
    }

    while true {
      guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: chunk) else {
        throw Failure.format
      }
      var error: NSError?
      let status = converter.convert(to: output, error: &error, withInputFrom: input)
      if let error { throw Failure.convert(error.localizedDescription) }
      if output.frameLength > 0, let data = output.floatChannelData?[0] {
        pcm.append(contentsOf: UnsafeBufferPointer(start: data, count: Int(output.frameLength) * 2))
      }
      if status == .endOfStream || status == .error { break }
      if status == .inputRanDry, reachedEnd { break }
    }

    return Decoded(pcm: pcm, waveform: waveform(of: pcm, step: Int(sampleRate) / 200))
  }

  /// Peak of the mono mix per `step` frames.
  static func waveform(of pcm: [Float], step: Int) -> [Float] {
    let step = max(step, 1)
    let frames = pcm.count / 2
    var peaks: [Float] = []
    peaks.reserveCapacity(frames / step + 1)
    pcm.withUnsafeBufferPointer { samples in
      var start = 0
      while start < frames {
        let end = min(start + step, frames)
        var peak: Float = 0
        for frame in start..<end {
          let mono = abs((samples[frame * 2] + samples[frame * 2 + 1]) * 0.5)
          if mono > peak { peak = mono }
        }
        peaks.append(peak)
        start = end
      }
    }
    return peaks
  }
}
