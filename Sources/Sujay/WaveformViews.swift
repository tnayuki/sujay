import SwiftUI

/// Colour per waveform sample: rekordbox 3-band colour when present, else
/// played / unplayed blue-grey split at the playhead.
private func sampleColor(
  _ colors: [UInt8], _ index: Int, played: Bool
) -> Color {
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

/// The 8-second window around the playhead, scaled by the tempo ratio.
struct ZoomWaveformView: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let deck = model.deck(index)
    let buffers = model.decks[index]
    let masterTempo = model.snapshot.master_tempo
    Canvas(rendersAsynchronously: false) { context, size in
      context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Theme.waveformEmpty))
      let total = deck.total_frames
      guard !buffers.waveform.isEmpty, total > 0, deck.sample_rate > 0 else { return }

      let current = deck.position_frames
      let rate: Float =
        deck.bpm > 0 && masterTempo > 0 ? min(max(masterTempo / deck.bpm, 0.5), 2) : 1
      let visible = min(8 * deck.sample_rate * rate, total)
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

      let samples = buffers.waveform
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
          sampleColor(buffers.waveformColors, peakIndex, played: x <= progressX))
      }
      batch.draw(in: &context)

      var beatPath = Path()
      for beat in buffers.beats where beat >= viewStart && beat <= viewEnd {
        let x = toX(beat)
        beatPath.move(to: CGPoint(x: x, y: 0))
        beatPath.addLine(to: CGPoint(x: x, y: size.height))
      }
      context.stroke(beatPath, with: .color(Theme.rgb(255, 100, 100, 0.8)), lineWidth: 1)

      drawMarkers(
        &context, size: size, toX: toX, intro: buffers.intro, outro: buffers.outro,
        loopEnabled: deck.loop_enabled != 0, loopStart: deck.loop_start, loopEnd: deck.loop_end)

      var playhead = Path()
      playhead.move(to: CGPoint(x: progressX, y: 0))
      playhead.addLine(to: CGPoint(x: progressX, y: size.height))
      context.stroke(playhead, with: .color(.white), lineWidth: 2)
    }
    .frame(height: 56)
  }
}

/// The whole track; click to seek.
struct FullWaveformView: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int
  var height: CGFloat = 50

  var body: some View {
    let deck = model.deck(index)
    let buffers = model.decks[index]
    GeometryReader { geometry in
      Canvas(rendersAsynchronously: false) { context, size in
        context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Theme.bgDark))
        let total = deck.total_frames
        guard !buffers.waveform.isEmpty, total > 0 else { return }
        let toX = { (pos: Float) -> CGFloat in
          CGFloat(min(max(pos / total, 0), 1)) * size.width
        }
        let samples = buffers.waveform
        let barCount = max(min(Int(size.width), 512), 1)
        let step = max(Float(samples.count) / Float(barCount), 1)
        let barWidth = size.width / CGFloat(barCount)
        let cy = size.height / 2
        let progressX = toX(deck.position_frames)
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
            sampleColor(buffers.waveformColors, peakIndex, played: x < progressX))
        }
        batch.draw(in: &context)

        drawMarkers(
          &context, size: size, toX: toX, intro: buffers.intro, outro: buffers.outro,
          loopEnabled: deck.loop_enabled != 0, loopStart: deck.loop_start, loopEnd: deck.loop_end)

        var playhead = Path()
        playhead.move(to: CGPoint(x: progressX, y: 0))
        playhead.addLine(to: CGPoint(x: progressX, y: size.height))
        context.stroke(playhead, with: .color(.white), lineWidth: 2)
      }
      .contentShape(Rectangle())
      .gesture(
        DragGesture(minimumDistance: 0).onEnded { value in
          let position = Float(min(max(value.location.x / geometry.size.width, 0), 1))
          model.seek(index, position)
        })
    }
    .frame(height: height)
  }
}

private func drawMarkers(
  _ context: inout GraphicsContext, size: CGSize, toX: (Float) -> CGFloat,
  intro: Float?, outro: Float?, loopEnabled: Bool, loopStart: Float, loopEnd: Float
) {
  func vline(_ x: CGFloat, _ color: Color, _ width: CGFloat) {
    var path = Path()
    path.move(to: CGPoint(x: x, y: 0))
    path.addLine(to: CGPoint(x: x, y: size.height))
    context.stroke(path, with: .color(color), lineWidth: width)
  }
  if let intro { vline(toX(intro), Theme.rgb(100, 255, 100, 0.8), 2) }
  if let outro { vline(toX(outro), Theme.rgb(255, 255, 100, 0.8), 2) }
  if loopEnabled, loopStart < loopEnd {
    let x1 = toX(loopStart)
    let x2 = toX(loopEnd)
    if x2 > x1 {
      context.fill(
        Path(CGRect(x: x1, y: 0, width: x2 - x1, height: size.height)),
        with: .color(Theme.rgb(0, 204, 102, 0.16)))
      vline(x1, Theme.rgb(0, 204, 102, 0.7), 1)
      vline(x2, Theme.rgb(0, 204, 102, 0.7), 1)
    }
  }
}
