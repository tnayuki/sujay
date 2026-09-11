import CoreAudio
import Foundation

/// Output devices through the HAL property API. No AudioUnit is created to
/// ask — that is what deadlocked inside CoreAudio when cpal did it.
enum AudioDevices {
  static func outputDevices() -> [AudioDevice] {
    allDeviceIDs()
      .compactMap { id -> AudioDevice? in
        let channels = outputChannelCount(id)
        guard channels > 0, let name = name(of: id),
          // AVAudioEngine's private aggregate for its own I/O; not a user choice.
          !name.hasPrefix("CADefaultDeviceAggregate")
        else { return nil }
        return AudioDevice(name: name, maxOutputChannels: UInt16(min(channels, Int(UInt16.max))))
      }
      .sorted { $0.name < $1.name }
  }

  static func deviceID(named name: String) -> AudioDeviceID? {
    allDeviceIDs().first { self.name(of: $0) == name && outputChannelCount($0) > 0 }
  }

  static func defaultOutputDeviceID() -> AudioDeviceID? {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioHardwarePropertyDefaultOutputDevice,
      mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var id = AudioDeviceID(0)
    var size = UInt32(MemoryLayout<AudioDeviceID>.size)
    let status = AudioObjectGetPropertyData(
      AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &id)
    return status == noErr && id != 0 ? id : nil
  }

  static func name(of id: AudioDeviceID) -> String? {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioObjectPropertyName, mScope: kAudioObjectPropertyScopeGlobal,
      mElement: kAudioObjectPropertyElementMain)
    var name: Unmanaged<CFString>?
    var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    let status = AudioObjectGetPropertyData(id, &address, 0, nil, &size, &name)
    guard status == noErr, let name else { return nil }
    return name.takeRetainedValue() as String
  }

  static func outputChannelCount(_ id: AudioDeviceID) -> Int {
    channelCount(id, scope: kAudioDevicePropertyScopeOutput)
  }

  static func inputChannelCount(_ id: AudioDeviceID) -> Int {
    channelCount(id, scope: kAudioDevicePropertyScopeInput)
  }

  private static func channelCount(_ id: AudioDeviceID, scope: AudioObjectPropertyScope) -> Int {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioDevicePropertyStreamConfiguration, mScope: scope,
      mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(id, &address, 0, nil, &size) == noErr, size > 0 else {
      return 0
    }
    let raw = UnsafeMutableRawPointer.allocate(byteCount: Int(size), alignment: 8)
    defer { raw.deallocate() }
    guard AudioObjectGetPropertyData(id, &address, 0, nil, &size, raw) == noErr else { return 0 }
    let list = UnsafeMutableAudioBufferListPointer(
      raw.assumingMemoryBound(to: AudioBufferList.self))
    return list.reduce(0) { $0 + Int($1.mNumberChannels) }
  }

  private static func allDeviceIDs() -> [AudioDeviceID] {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal,
      mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    guard
      AudioObjectGetPropertyDataSize(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr
    else { return [] }
    var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
    guard
      AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &ids) == noErr
    else { return [] }
    return ids
  }
}
