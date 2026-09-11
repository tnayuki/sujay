import Foundation

/// A cursor over big-endian bytes that never reads past the end it was given.
///
/// Every read returns nil rather than trapping, so a truncated or unexpected ANLZ file
/// costs the sections after the damage and nothing else.
private struct Reader {
  private let data: Data
  private var offset: Int
  private let end: Int

  init(_ data: Data, offset: Int = 0, end: Int? = nil) {
    self.data = data
    self.offset = data.startIndex + offset
    self.end = end.map { data.startIndex + $0 } ?? data.endIndex
  }

  private mutating func take(_ count: Int) -> Range<Int>? {
    guard offset + count <= end else { return nil }
    defer { offset += count }
    return offset..<(offset + count)
  }

  mutating func uint8() -> UInt8? { take(1).map { data[$0.lowerBound] } }

  mutating func uint16() -> UInt16? {
    guard let range = take(2) else { return nil }
    return UInt16(data[range.lowerBound]) << 8 | UInt16(data[range.lowerBound + 1])
  }

  mutating func uint32() -> UInt32? {
    guard let range = take(4) else { return nil }
    return data[range].reduce(UInt32(0)) { $0 << 8 | UInt32($1) }
  }

  mutating func tag() -> String? {
    guard let range = take(4) else { return nil }
    return String(decoding: data[range], as: UTF8.self)
  }

  /// A UTF-16BE string of `bytes` bytes including its trailing NUL.
  mutating func utf16BE(bytes: Int) -> String? {
    guard bytes >= 2, let range = take(bytes) else { return nil }
    var scalars = String.UnicodeScalarView()
    var index = range.lowerBound
    while index + 1 < range.upperBound {
      let unit = UInt16(data[index]) << 8 | UInt16(data[index + 1])
      index += 2
      guard unit != 0, let scalar = Unicode.Scalar(unit) else { break }
      scalars.append(scalar)
    }
    return String(scalars)
  }

  /// Reads a nested `tag / header size / total size` block, advancing this reader past the
  /// whole block and returning a reader over the part after those twelve bytes.
  mutating func section() -> Reader? {
    let start = offset
    guard tag() != nil, let size = uint32(), let total = uint32(), size >= 12, total >= size,
      start + Int(total) <= end
    else { return nil }
    offset = start + Int(total)
    return Reader(
      data, offset: start + 12 - data.startIndex, end: start + Int(total) - data.startIndex)
  }
}

/// A rekordbox ANLZ analysis file set: big-endian, a `PMAI` header followed by tagged sections.
///
/// rekordbox splits one track's analysis over three files that sit in the same directory —
/// `.DAT` holds the beat grid and the original cue lists, `.EXT` the extended cues and the
/// colour waveforms, `.2EX` the three-band waveforms — so all three are parsed together and
/// sections are looked up across the set.
struct ANLZ {
  /// Section bodies by tag, in file order, each positioned just after the twelve bytes of
  /// `tag / header size / total size` so a section's own header fields are still readable.
  /// A tag can repeat: cue lists appear twice, once for memory cues and once for hot cues.
  private var sections: [String: [Reader]] = [:]

  init(directory: URL) {
    let contents =
      (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil))
      ?? []
    for url in contents.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
      switch url.pathExtension.uppercased() {
      case "DAT", "EXT", "2EX": parse(url)
      default: continue
      }
    }
  }

  private mutating func parse(_ url: URL) {
    guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return }
    var file = Reader(data)
    guard file.tag() == "PMAI", let headerSize = file.uint32(), let totalSize = file.uint32(),
      headerSize >= 12
    else { return }
    var offset = Int(headerSize)
    let end = min(Int(totalSize), data.count)
    while offset + 12 <= end {
      var header = Reader(data, offset: offset, end: end)
      guard let tag = header.tag(), let size = header.uint32(), let total = header.uint32(),
        size >= 12, total >= size, offset + Int(total) <= end
      else { return }
      sections[tag, default: []].append(
        Reader(data, offset: offset + 12, end: offset + Int(total)))
      offset += Int(total)
    }
  }

  // MARK: Beat grid

  /// Beat positions in milliseconds, from the `PQTZ` grid in the `.DAT` file.
  func beatsMs() -> [Float] {
    guard var reader = sections["PQTZ"]?.first else { return [] }
    _ = reader.uint32()  // unknown, always 0
    _ = reader.uint32()  // unknown, always 0x80000
    guard let count = reader.uint32() else { return [] }
    var beats: [Float] = []
    beats.reserveCapacity(Int(count))
    for _ in 0..<count {
      guard reader.uint16() != nil, reader.uint16() != nil, let time = reader.uint32() else {
        break
      }
      beats.append(Float(time))
    }
    return beats
  }

  // MARK: Cues

  /// Cues from the ANLZ files, preferring the extended `PCO2` lists — which carry colours and
  /// comments — and falling back to the original `PCPT` entries inside `PCOB`.
  func cues() -> [RekordboxCue] {
    let extended = (sections["PCO2"] ?? []).flatMap(extendedCues)
    if !extended.isEmpty { return extended }
    return (sections["PCOB"] ?? []).flatMap(legacyCues)
  }

  private func extendedCues(_ list: Reader) -> [RekordboxCue] {
    var reader = list
    guard reader.uint32() != nil, let count = reader.uint16() else { return [] }
    _ = reader.uint16()
    var cues: [RekordboxCue] = []
    for _ in 0..<count {
      guard var entry = reader.section(), let hotCue = entry.uint32(), let type = entry.uint8()
      else { break }
      _ = entry.uint8()
      _ = entry.uint16()
      guard let time = entry.uint32(), let loopTime = entry.uint32() else { break }
      _ = entry.uint8()  // colour table index, memory cues only
      _ = entry.uint8()
      _ = entry.uint16()
      _ = entry.uint32()
      _ = entry.uint16()  // loop size numerator
      _ = entry.uint16()  // loop size denominator
      var comment: String?
      if let length = entry.uint32(), let text = entry.utf16BE(bytes: Int(length)), !text.isEmpty {
        comment = text
      }
      var color: [UInt8]?
      if entry.uint8() != nil, let red = entry.uint8(), let green = entry.uint8(),
        let blue = entry.uint8(), (red, green, blue) != (0, 0, 0)
      {
        color = [red, green, blue]
      }
      cues.append(
        RekordboxCue(
          hotCue: hotCue, timeMs: time, loopTimeMs: loopTime, isLoop: type == 2, colorRgb: color,
          comment: comment))
    }
    return cues
  }

  private func legacyCues(_ list: Reader) -> [RekordboxCue] {
    var reader = list
    guard reader.uint32() != nil, reader.uint16() != nil, let count = reader.uint16() else {
      return []
    }
    _ = reader.uint32()
    var cues: [RekordboxCue] = []
    for _ in 0..<count {
      guard var entry = reader.section(), let hotCue = entry.uint32(), entry.uint32() != nil,
        entry.uint32() != nil, entry.uint16() != nil, entry.uint16() != nil,
        let type = entry.uint8()
      else { break }
      _ = entry.uint8()
      _ = entry.uint16()
      guard let time = entry.uint32(), let loopTime = entry.uint32() else { break }
      cues.append(
        RekordboxCue(
          hotCue: hotCue, timeMs: time, loopTimeMs: loopTime, isLoop: type == 2, colorRgb: nil,
          comment: nil))
    }
    return cues
  }

  // MARK: Waveform

  /// The best colour waveform the analysis offers, in descending order of fidelity: `PWV5`
  /// true colour, then `PWV4` band energies, then the `PWV7`/`PWV6` three-band data that is
  /// all a CDJ-3000-era analysis writes. Returned as flattened RGB triplets.
  func waveformRGB() -> [UInt8] {
    if let reader = sections["PWV5"]?.first { return colorDetail(reader) }
    if let reader = sections["PWV4"]?.first { return colorPreview(reader) }
    if let reader = sections["PWV7"]?.first { return threeBand(reader, hasUnknownField: true) }
    if let reader = sections["PWV6"]?.first { return threeBand(reader, hasUnknownField: false) }
    return []
  }

  /// `PWV5`: two bytes per column, read as one big-endian 16-bit value holding red, green
  /// and blue in the top three bits each, then five bits of height and two unused bits. The
  /// channels are widened to the full 0...255 range rather than left at 0...7, which would
  /// render every column all but black. Height is ignored — the deck draws its own envelope
  /// from the decoded audio and takes only the colour from here.
  private func colorDetail(_ section: Reader) -> [UInt8] {
    var reader = section
    guard reader.uint32() == 2, let count = reader.uint32(), reader.uint32() != nil else {
      return []
    }
    var rgb: [UInt8] = []
    rgb.reserveCapacity(Int(count) * 3)
    for _ in 0..<count {
      guard let value = reader.uint16() else { break }
      rgb.append(Self.widen3(UInt8((value >> 13) & 0x7)))
      rgb.append(Self.widen3(UInt8((value >> 10) & 0x7)))
      rgb.append(Self.widen3(UInt8((value >> 7) & 0x7)))
    }
    return rgb
  }

  /// `PWV4`: six bytes per column, of which the three band energies are already 0...255.
  private func colorPreview(_ section: Reader) -> [UInt8] {
    var reader = section
    guard reader.uint32() == 6, let count = reader.uint32(), reader.uint32() != nil else {
      return []
    }
    var rgb: [UInt8] = []
    rgb.reserveCapacity(Int(count) * 3)
    for _ in 0..<count {
      guard reader.uint8() != nil, reader.uint8() != nil, reader.uint8() != nil,
        let bottom = reader.uint8(), let mid = reader.uint8(), let top = reader.uint8()
      else { break }
      rgb.append(contentsOf: [bottom, mid, top])
    }
    return rgb
  }

  /// `PWV6`/`PWV7`: three bytes per column holding mid, high and low band energy rather than
  /// a colour. They are mapped onto the blue → amber → white palette rekordbox draws them
  /// with, so a track analysed only by a CDJ-3000 still gets a coloured waveform.
  private func threeBand(_ section: Reader, hasUnknownField: Bool) -> [UInt8] {
    var reader = section
    guard reader.uint32() == 3, let count = reader.uint32() else { return [] }
    if hasUnknownField, reader.uint32() == nil { return [] }
    var rgb: [UInt8] = []
    rgb.reserveCapacity(Int(count) * 3)
    for _ in 0..<count {
      guard let mid = reader.uint8(), let high = reader.uint8(), let low = reader.uint8() else {
        break
      }
      rgb.append(contentsOf: [mid &+ high / 2, mid &+ high / 3, low &+ high / 2])
    }
    return rgb
  }

  /// 0...7 to 0...255, keeping 0 at black and 7 at full scale.
  private static func widen3(_ value: UInt8) -> UInt8 { value << 5 | value << 2 | value >> 1 }
}
