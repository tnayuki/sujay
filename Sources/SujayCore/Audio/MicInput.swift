import AVFoundation
import Foundation
import os

/// Microphone input for talkover: an AVAudioEngine of its own on the chosen
/// device, whose input tap fills a ring the mix render drains. Mono in,
/// duplicated to stereo, at most 100 ms buffered.
final class MicInput {
  private let engine = AVAudioEngine()
  private let lock = OSAllocatedUnfairLock()
  private var ring: [Float]
  private var readIndex = 0
  private var writeIndex = 0
  private var count = 0
  private(set) var peak: Float = 0
  let sampleRate: Double
  /// Touching `inputNode` at all creates the input unit and asks for the
  /// microphone, so it happens only once the device is known to have inputs,
  /// and `deinit` removes the tap only if one was installed.
  private var tapInstalled = false

  init?(deviceID: AudioDeviceID, sampleRate: Double) {
    self.sampleRate = sampleRate
    ring = [Float](repeating: 0, count: Int(sampleRate / 10) * 2)
    guard AudioDevices.inputChannelCount(deviceID) > 0 else { return nil }
    let input = engine.inputNode
    var device = deviceID
    guard let unit = input.audioUnit,
      AudioUnitSetProperty(
        unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &device,
        UInt32(MemoryLayout<AudioDeviceID>.size)) == noErr
    else { return nil }
    let inputFormat = input.outputFormat(forBus: 0)
    guard inputFormat.channelCount > 0 else { return nil }
    // The input must feed something for the engine to pull it; a silent
    // mixer into the output does that.
    engine.mainMixerNode.outputVolume = 0
    engine.connect(input, to: engine.mainMixerNode, format: inputFormat)
    input.installTap(onBus: 0, bufferSize: 512, format: inputFormat) { [weak self] buffer, _ in
      self?.push(buffer)
    }
    tapInstalled = true
    do {
      try engine.start()
    } catch {
      NSLog("sujay: microphone input failed: \(error)")
      return nil
    }
  }

  deinit {
    if tapInstalled {
      engine.inputNode.removeTap(onBus: 0)
      engine.stop()
    }
  }

  private func push(_ buffer: AVAudioPCMBuffer) {
    guard let data = buffer.floatChannelData?[0] else { return }
    let frames = Int(buffer.frameLength)
    var localPeak: Float = 0
    lock.withLock {
      for i in 0..<frames {
        let sample = data[i]
        localPeak = max(localPeak, abs(sample))
        for _ in 0..<2 {
          ring[writeIndex] = sample
          writeIndex = (writeIndex + 1) % ring.count
          if count < ring.count {
            count += 1
          } else {
            readIndex = (readIndex + 1) % ring.count
          }
        }
      }
      peak = peak * 0.9 + localPeak * 0.1
    }
  }

  /// Drain `frames` stereo frames into `left` / `right`; false if not enough
  /// is buffered yet. Render thread.
  func pull(frames: Int, left: UnsafeMutablePointer<Float>, right: UnsafeMutablePointer<Float>)
    -> Bool
  {
    guard lock.lockIfAvailable() else { return false }
    defer { lock.unlock() }
    guard count >= frames * 2 else { return false }
    for i in 0..<frames {
      left[i] = ring[readIndex]
      readIndex = (readIndex + 1) % ring.count
      right[i] = ring[readIndex]
      readIndex = (readIndex + 1) % ring.count
    }
    count -= frames * 2
    return true
  }
}
