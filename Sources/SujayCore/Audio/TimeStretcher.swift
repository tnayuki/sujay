import AVFoundation
import Foundation

/// Apple's time-pitch unit driven in pull mode: the engine's render asks for
/// output frames and the unit pulls source frames from the caller's closure
/// at `rate`, keeping pitch. Not attached to an AVAudioEngine; rendered by
/// hand through the AUv2 API — the AUv3 `renderBlock` of this bridged unit
/// refuses to render without a connection (kAudioUnitErr_NoConnection).
final class TimeStretcher {
  private let timePitch = AVAudioUnitTimePitch()
  private let unit: AudioUnit
  private var sampleTime: Double = 0
  private var pull: ((UnsafeMutablePointer<AudioBufferList>, Int) -> Void)?
  let sampleRate: Double
  /// Last non-zero render status, for diagnostics.
  private(set) var lastStatus: OSStatus = noErr

  /// Playback rate (0.5...2), 1 = original tempo. Pitch stays put.
  var rate: Float {
    get { timePitch.rate }
    set { timePitch.rate = newValue }
  }

  /// Output-referred latency in frames.
  var latencyFrames: Int {
    var latency: Float64 = 0
    var size = UInt32(MemoryLayout<Float64>.size)
    AudioUnitGetProperty(
      unit, kAudioUnitProperty_Latency, kAudioUnitScope_Global, 0, &latency, &size)
    return Int(latency * sampleRate)
  }

  init?(sampleRate: Double, maximumFrames: Int) {
    self.sampleRate = sampleRate
    unit = timePitch.audioUnit
    timePitch.pitch = 0
    timePitch.rate = 1
    var format = AudioStreamBasicDescription(
      mSampleRate: sampleRate, mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kAudioFormatFlagsNativeFloatPacked | kAudioFormatFlagIsNonInterleaved,
      mBytesPerPacket: 4, mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 2,
      mBitsPerChannel: 32, mReserved: 0)
    let formatSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    var maxFrames = UInt32(maximumFrames)
    var callback = AURenderCallbackStruct(
      inputProc: { refCon, _, _, _, count, ioData -> OSStatus in
        guard let ioData else { return kAudioUnitErr_NoConnection }
        let stretcher = Unmanaged<TimeStretcher>.fromOpaque(refCon).takeUnretainedValue()
        stretcher.pull?(ioData, Int(count))
        return noErr
      }, inputProcRefCon: nil)
    // Set after `self` exists; the callback needs it as its refCon.
    callback.inputProcRefCon = Unmanaged.passUnretained(self).toOpaque()
    let steps: [OSStatus] = [
      AudioUnitSetProperty(
        unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &format, formatSize),
      AudioUnitSetProperty(
        unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 0, &format, formatSize),
      AudioUnitSetProperty(
        unit, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, &maxFrames,
        UInt32(MemoryLayout<UInt32>.size)),
      AudioUnitSetProperty(
        unit, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &callback,
        UInt32(MemoryLayout<AURenderCallbackStruct>.size)),
      AudioUnitInitialize(unit),
    ]
    if let failure = steps.first(where: { $0 != noErr }) {
      NSLog("sujay: time-pitch unit setup failed: \(failure)")
      return nil
    }
  }

  deinit {
    AudioUnitUninitialize(unit)
  }

  /// Forget the history after a seek or loop jump.
  func reset() {
    AudioUnitReset(unit, kAudioUnitScope_Global, 0)
  }

  /// Render `frames` output frames into `output` (two non-interleaved
  /// channels). `pull` fills the given buffer list with source frames; it is
  /// called from inside the unit as it needs input.
  func render(
    frames: Int, into output: UnsafeMutablePointer<AudioBufferList>,
    pull: (UnsafeMutablePointer<AudioBufferList>, Int) -> Void
  ) -> Bool {
    var flags = AudioUnitRenderActionFlags()
    var timestamp = AudioTimeStamp()
    timestamp.mSampleTime = sampleTime
    timestamp.mFlags = .sampleTimeValid
    sampleTime += Double(frames)
    let status = withoutActuallyEscaping(pull) { escapable -> OSStatus in
      self.pull = escapable
      defer { self.pull = nil }
      return AudioUnitRender(unit, &flags, &timestamp, 0, UInt32(frames), output)
    }
    if status != noErr { lastStatus = status }
    return status == noErr
  }
}
