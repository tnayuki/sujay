import AVFoundation
import Foundation
import os

/// Writes the main mix to a file. The render thread drops stereo frames into
/// a ring; a writer thread drains it into an `AVAudioFile`, which encodes to
/// 16-bit WAV or AAC in an .m4a.
final class Recorder {
  private let lock = OSAllocatedUnfairLock()
  private var ring: [Float]
  private var readIndex = 0
  private var writeIndex = 0
  private var count = 0
  private var file: AVAudioFile?
  private var writer: Thread?
  private var stopping = false
  private let format: AVAudioFormat
  private(set) var isRecording = false

  init(sampleRate: Double) {
    ring = [Float](repeating: 0, count: Int(sampleRate) * 10)  // 5 s of stereo
    format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 2)!
  }

  func start(path: String, recordingFormat: RecordingFormat) throws {
    stop()
    var settings: [String: Any] = [
      AVSampleRateKey: format.sampleRate,
      AVNumberOfChannelsKey: 2,
    ]
    switch recordingFormat {
    case .wav:
      settings[AVFormatIDKey] = kAudioFormatLinearPCM
      settings[AVLinearPCMBitDepthKey] = 16
      settings[AVLinearPCMIsFloatKey] = false
      settings[AVLinearPCMIsBigEndianKey] = false
      settings[AVLinearPCMIsNonInterleaved] = false
    case .m4a:
      settings[AVFormatIDKey] = kAudioFormatMPEG4AAC
      settings[AVEncoderBitRateKey] = 256_000
    }
    let file = try AVAudioFile(
      forWriting: URL(fileURLWithPath: path), settings: settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    lock.withLock {
      self.file = file
      readIndex = 0
      writeIndex = 0
      count = 0
      stopping = false
      isRecording = true
    }
    let thread = Thread { [weak self] in self?.writeLoop() }
    thread.name = "sujay.recorder"
    thread.qualityOfService = .userInitiated
    writer = thread
    thread.start()
  }

  func stop() {
    lock.withLock { stopping = true }
    writer = nil
  }

  /// Render thread: interleaved stereo frames.
  func push(left: UnsafePointer<Float>, right: UnsafePointer<Float>, frames: Int) {
    guard lock.lockIfAvailable() else { return }
    defer { lock.unlock() }
    guard isRecording, !stopping else { return }
    for i in 0..<frames {
      ring[writeIndex] = left[i]
      ring[(writeIndex + 1) % ring.count] = right[i]
      writeIndex = (writeIndex + 2) % ring.count
      count = min(count + 2, ring.count)
    }
  }

  private func writeLoop() {
    guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 8192) else { return }
    while true {
      var frames = 0
      var finished = false
      lock.withLock {
        frames = min(count / 2, 8192)
        if frames > 0, let left = buffer.floatChannelData?[0],
          let right = buffer.floatChannelData?[1]
        {
          for i in 0..<frames {
            left[i] = ring[readIndex]
            right[i] = ring[(readIndex + 1) % ring.count]
            readIndex = (readIndex + 2) % ring.count
          }
          count -= frames * 2
        }
        finished = stopping && count == 0
      }
      if frames > 0 {
        buffer.frameLength = AVAudioFrameCount(frames)
        do {
          try file?.write(from: buffer)
        } catch {
          NSLog("sujay: recording write failed: \(error)")
          finished = true
        }
      }
      if finished {
        lock.withLock {
          file = nil
          isRecording = false
        }
        return
      }
      if frames == 0 { Thread.sleep(forTimeInterval: 0.05) }
    }
  }
}
