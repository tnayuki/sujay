import SwiftUI

/// Colour per waveform sample: rekordbox 3-band colour when present, else
/// the accent for the played part and secondary for the rest.
private func sampleColor(_ colors: [UInt8], _ index: Int, played: Bool) -> Color {
  if index * 3 + 2 < colors.count {
    return Color(
      red: Double(colors[index * 3]) / 255,
      green: Double(colors[index * 3 + 1]) / 255,
      blue: Double(colors[index * 3 + 2]) / 255)
  }
  return played ? Theme.waveformPlayed : Theme.waveformUnplayed
}

/// Batches rectangles by colour so a frame is a handful of fills, not a
/// thousand.
private struct ColumnBatch {
  private var paths: [Color: Path] = [:]

  mutating func add(_ rect: CGRect, _ color: Color) {
    paths[color, default: Path()].addRect(rect)
  }

  func draw(in context: inout GraphicsContext) {
    for (color, path) in paths {
      context.fill(path, with: .color(color))
    }
  }
}

private let waveformBackground = Color(nsColor: .controlBackgroundColor)

/// The 8-second window around the playhead, scaled by the tempo ratio.
struct ZoomWaveformView: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let deck = model.deck(index)
    let track = model.tracks[index]
    let masterTempo = model.snapshot.masterTempo
    Canvas(rendersAsynchronously: false) { context, size in
      guard let track, !track.waveform.isEmpty, deck.totalFrames > 0, deck.sampleRate > 0 else {
        return
      }
      let total = Float(deck.totalFrames)
      let current = Float(deck.positionFrames)
      let rate: Float =
        deck.bpm > 0 && masterTempo > 0 ? min(max(masterTempo / deck.bpm, 0.5), 2) : 1
      let visible = min(8 * deck.sampleRate * rate, total)
      var viewStart = current - visible * 0.3
      var viewEnd = viewStart + visible
      if viewStart < 0 {
        viewStart = 0
        viewEnd = visible
      }
      if viewEnd > total {
        viewEnd = total
        viewStart = max(total - visible, 0)
      }
      let span = max(viewEnd - viewStart, 1)
      let width = Float(size.width)
      let toX = { (pos: Float) -> CGFloat in
        CGFloat(min(max((pos - viewStart) / span, 0), 1)) * size.width
      }

      let samples = track.waveform
      let count = Float(samples.count)
      let progressX = toX(current)
      let cy = size.height / 2
      var batch = ColumnBatch()
      for px in 0..<Int(size.width) {
        let frameLeft = viewStart + (Float(px) / width) * span
        let frameRight = viewStart + (Float(px + 1) / width) * span
        var lo = Int((frameLeft / total * count).rounded(.down))
        var hi = Int((frameRight / total * count).rounded(.up))
        lo = min(max(lo, 0), samples.count - 1)
        hi = min(max(hi, lo + 1), samples.count)
        var maxAmp: Float = 0
        var peakIndex = lo
        for j in lo..<hi where abs(samples[j]) > maxAmp {
          maxAmp = abs(samples[j])
          peakIndex = j
        }
        guard maxAmp > 0 else { continue }
        let x = CGFloat(px)
        let h = max(CGFloat(maxAmp) * size.height * 0.5, 0.5)
        batch.add(
          CGRect(x: x, y: cy - h, width: 1, height: h * 2),
          sampleColor(track.waveformColors, peakIndex, played: x <= progressX))
      }
      batch.draw(in: &context)

      var beatPath = Path()
      for beat in track.beats where beat >= viewStart && beat <= viewEnd {
        let x = toX(beat)
        beatPath.move(to: CGPoint(x: x, y: 0))
        beatPath.addLine(to: CGPoint(x: x, y: size.height))
      }
      context.stroke(beatPath, with: .color(Theme.beatMarker), lineWidth: 1)

      drawMarkers(
        &context, size: size, toX: toX,
        loopEnabled: deck.loopEnabled, loopStart: deck.loopStart, loopEnd: deck.loopEnd)

      var playhead = Path()
      playhead.move(to: CGPoint(x: progressX, y: 0))
      playhead.addLine(to: CGPoint(x: progressX, y: size.height))
      context.stroke(playhead, with: .color(.primary), lineWidth: 2)
    }
    .frame(height: 56)
    .background(waveformBackground, in: RoundedRectangle(cornerRadius: 6))
    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator, lineWidth: 1))
  }
}

/// The whole track; click to seek.
struct FullWaveformView: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int
  var height: CGFloat = 50

  var body: some View {
    let deck = model.deck(index)
    let track = model.tracks[index]
    GeometryReader { geometry in
      Canvas(rendersAsynchronously: false) { context, size in
        guard let track, !track.waveform.isEmpty, deck.totalFrames > 0 else { return }
        let total = Float(deck.totalFrames)
        let toX = { (pos: Float) -> CGFloat in
          CGFloat(min(max(pos / total, 0), 1)) * size.width
        }
        let samples = track.waveform
        let barCount = max(min(Int(size.width), 512), 1)
        let step = max(Float(samples.count) / Float(barCount), 1)
        let barWidth = size.width / CGFloat(barCount)
        let cy = size.height / 2
        let progressX = toX(Float(deck.positionFrames))
        var batch = ColumnBatch()
        for i in 0..<barCount {
          let start = Int((Float(i) * step).rounded(.down))
          let end = min(Int((Float(i + 1) * step).rounded(.down)), samples.count)
          var maxAmp: Float = 0
          var peakIndex = min(start, samples.count - 1)
          for j in start..<end where abs(samples[j]) > maxAmp {
            maxAmp = abs(samples[j])
            peakIndex = j
          }
          let x = CGFloat(i) * barWidth
          let bh = CGFloat(maxAmp) * (size.height * 0.5) * 0.9
          batch.add(
            CGRect(x: x + 0.5, y: cy - bh, width: max(barWidth - 1, 1), height: bh * 2),
            sampleColor(track.waveformColors, peakIndex, played: x < progressX))
        }
        batch.draw(in: &context)

        drawMarkers(
          &context, size: size, toX: toX,
          loopEnabled: deck.loopEnabled, loopStart: deck.loopStart, loopEnd: deck.loopEnd)

        var playhead = Path()
        playhead.move(to: CGPoint(x: progressX, y: 0))
        playhead.addLine(to: CGPoint(x: progressX, y: size.height))
        context.stroke(playhead, with: .color(.primary), lineWidth: 2)
      }
      .contentShape(Rectangle())
      .gesture(
        DragGesture(minimumDistance: 0).onEnded { value in
          let position = Float(min(max(value.location.x / geometry.size.width, 0), 1))
          model.seek(index, position)
        })
    }
    .frame(height: height)
    .background(waveformBackground, in: RoundedRectangle(cornerRadius: 6))
    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator, lineWidth: 1))
  }
}

private func drawMarkers(
  _ context: inout GraphicsContext, size: CGSize, toX: (Float) -> CGFloat,
  loopEnabled: Bool, loopStart: Float, loopEnd: Float
) {
  func vline(_ x: CGFloat, _ color: Color, _ width: CGFloat) {
    var path = Path()
    path.move(to: CGPoint(x: x, y: 0))
    path.addLine(to: CGPoint(x: x, y: size.height))
    context.stroke(path, with: .color(color), lineWidth: width)
  }
  if loopEnabled, loopStart < loopEnd {
    let x1 = toX(loopStart)
    let x2 = toX(loopEnd)
    if x2 > x1 {
      context.fill(
        Path(CGRect(x: x1, y: 0, width: x2 - x1, height: size.height)),
        with: .color(Theme.loop.opacity(0.18)))
      vline(x1, Theme.loop.opacity(0.7), 1)
      vline(x2, Theme.loop.opacity(0.7), 1)
    }
  }
}
