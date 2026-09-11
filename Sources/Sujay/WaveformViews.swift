import AppKit
import SwiftUI

/// The waveforms draw in an NSView with CoreGraphics and redraw on the
/// model's frame tick, outside SwiftUI's layout: a Canvas re-evaluated every
/// frame invalidated its size and forced a layout pass to the root.
final class WaveformNSView: NSView {
  enum Mode {
    /// The 8-second window around the playhead, scaled by the tempo ratio.
    case zoom
    /// The whole track; click to seek.
    case full
  }

  var model: ConsoleModel? {
    didSet { subscribe() }
  }
  var index = 0
  var mode = Mode.zoom
  private var listener: UUID?
  private var colorCache: [UInt16: CGColor] = [:]
  /// What the last draw showed; a frame that would draw the same is skipped.
  private var lastDrawKey: (Double, Double, Bool, UUID?, CGSize, Float)?

  override var isFlipped: Bool { true }
  override var isOpaque: Bool { true }

  deinit {
    if let listener, let model { model.removeFrameListener(listener) }
  }

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    subscribe()
  }

  private func subscribe() {
    if let listener, let model { model.removeFrameListener(listener) }
    listener = nil
    guard let model, window != nil else { return }
    listener = model.addFrameListener { [weak self] in self?.needsDisplay = true }
  }

  /// The mean rekordbox colour over the stretch of track a drawn column covers,
  /// quantised to 4 bits per channel so columns of one colour fill as one path.
  ///
  /// Averaging rather than sampling is what keeps the full view from aliasing:
  /// it draws a few hundred columns over tens of thousands of rekordbox ones,
  /// so taking the colour at each column's midpoint would pick an arbitrary one
  /// in a hundred and shimmer instead of reading as the colour of that passage.
  /// It is the box filter the max-pooled height already has. In the zoom view
  /// the two resolutions are close, so this averages about one column.
  private static func colorKey(
    _ rgb: [UInt8], columns: Int, from frameLeft: Float, to frameRight: Float, of total: Float
  ) -> UInt16? {
    guard columns > 0, total > 0 else { return nil }
    let scale = Float(columns) / total
    let lo = min(max(Int(frameLeft * scale), 0), columns - 1)
    let hi = min(max(Int((frameRight * scale).rounded(.up)), lo + 1), columns)
    var red = 0
    var green = 0
    var blue = 0
    for column in lo..<hi {
      red += Int(rgb[column * 3])
      green += Int(rgb[column * 3 + 1])
      blue += Int(rgb[column * 3 + 2])
    }
    let count = hi - lo
    return (UInt16(red / count) >> 4) << 8 | (UInt16(green / count) >> 4) << 4
      | (UInt16(blue / count) >> 4)
  }

  private static let playedKey: UInt16 = 0xF000
  private static let unplayedKey: UInt16 = 0xF001

  private func color(for key: UInt16) -> CGColor {
    if let cached = colorCache[key] { return cached }
    let color: CGColor
    switch key {
    case Self.playedKey: color = Self.played
    case Self.unplayedKey: color = Self.unplayed
    default:
      color = CGColor(
        red: CGFloat((key >> 8) & 0xF) / 15, green: CGFloat((key >> 4) & 0xF) / 15,
        blue: CGFloat(key & 0xF) / 15, alpha: 1)
    }
    colorCache[key] = color
    return color
  }

  private static let played = NSColor.controlAccentColor.cgColor
  private static let unplayed = NSColor.secondaryLabelColor.cgColor
  private static let background = NSColor.controlBackgroundColor.cgColor
  private static let beat = NSColor.systemRed.withAlphaComponent(0.6).cgColor
  private static let loopFill = NSColor.systemGreen.withAlphaComponent(0.18).cgColor
  private static let loopEdge = NSColor.systemGreen.withAlphaComponent(0.7).cgColor
  private static let playhead = NSColor.labelColor.cgColor
  private static let border = NSColor.separatorColor.cgColor

  override func draw(_ dirtyRect: NSRect) {
    guard let context = NSGraphicsContext.current?.cgContext else { return }
    let size = bounds.size
    // Pixel-aligned rectangles: anti-aliasing only costs (it was most of the
    // frame in the profile) and softens nothing worth keeping.
    context.setShouldAntialias(false)
    context.setFillColor(Self.background)
    context.fill(bounds)
    defer {
      context.setStrokeColor(Self.border)
      context.setLineWidth(1)
      context.stroke(bounds.insetBy(dx: 0.5, dy: 0.5))
    }
    guard let model, let track = model.deck(index).track, !track.waveform.isEmpty else { return }
    let deck = model.deck(index)
    let total = Float(deck.totalFrames)
    guard total > 0 else { return }
    let current = Float(deck.positionFrames)
    let samples = track.waveform
    let count = Float(samples.count)
    let cy = size.height / 2

    var viewStart: Float = 0
    var viewEnd: Float = total
    if mode == .zoom {
      guard deck.sampleRate > 0 else { return }
      let masterTempo = model.masterTempo
      let rate: Float =
        deck.bpm > 0 && masterTempo > 0 ? min(max(masterTempo / deck.bpm, 0.5), 2) : 1
      let visible = min(8 * deck.sampleRate * rate, total)
      viewStart = current - visible * 0.3
      viewEnd = viewStart + visible
      if viewStart < 0 {
        viewStart = 0
        viewEnd = visible
      }
      if viewEnd > total {
        viewEnd = total
        viewStart = max(total - visible, 0)
      }
    }
    let span = max(viewEnd - viewStart, 1)
    func toX(_ pos: Float) -> CGFloat {
      CGFloat(min(max((pos - viewStart) / span, 0), 1)) * size.width
    }
    let progressX = toX(current)

    // Columns: the peak over the samples under each pixel (zoom) or bar (full),
    // grouped by colour so each colour is one fill.
    let columns = mode == .zoom ? Int(size.width) : max(min(Int(size.width), 512), 1)
    let columnWidth = size.width / CGFloat(columns)
    let heightScale = mode == .zoom ? size.height * 0.5 : size.height * 0.5 * 0.9
    let colors = track.waveformColors
    let colorColumns = colors.count / 3
    var paths: [UInt16: CGMutablePath] = [:]
    let inset: CGFloat = mode == .full ? 0.5 : 0
    let barWidth = max(columnWidth - inset * 2, 1)
    samples.withUnsafeBufferPointer { buffer in
      for column in 0..<columns {
        let frameLeft = viewStart + (Float(column) / Float(columns)) * span
        let frameRight = viewStart + (Float(column + 1) / Float(columns)) * span
        var lo = Int((frameLeft / total * count).rounded(.down))
        var hi = Int((frameRight / total * count).rounded(.up))
        lo = min(max(lo, 0), buffer.count - 1)
        hi = min(max(hi, lo + 1), buffer.count)
        var maxAmp: Float = 0
        for j in lo..<hi where abs(buffer[j]) > maxAmp { maxAmp = abs(buffer[j]) }
        guard maxAmp > 0 else { continue }
        let x = CGFloat(column) * columnWidth
        let h = max(CGFloat(maxAmp) * heightScale, 0.5)
        // rekordbox's colour columns are its own resolution, not the decoded
        // waveform's, so they are matched by position in the track.
        let key =
          Self.colorKey(colors, columns: colorColumns, from: frameLeft, to: frameRight, of: total)
          ?? (x < progressX ? Self.playedKey : Self.unplayedKey)
        let path = paths[key] ?? CGMutablePath()
        path.addRect(CGRect(x: x + inset, y: cy - h, width: barWidth, height: h * 2))
        paths[key] = path
      }
    }
    for (key, path) in paths {
      context.setFillColor(color(for: key))
      context.addPath(path)
      context.fillPath()
    }
    lastDrawKey = (
      Double(current), Double(deck.loopStart) * 1e6 + Double(deck.loopEnd), deck.loopEnabled,
      track.id, size, model.masterTempo
    )

    if mode == .zoom {
      context.setStrokeColor(Self.beat)
      context.setLineWidth(1)
      for beat in track.beats where beat >= viewStart && beat <= viewEnd {
        let x = toX(beat)
        context.move(to: CGPoint(x: x, y: 0))
        context.addLine(to: CGPoint(x: x, y: size.height))
      }
      context.strokePath()
    }

    if deck.loopEnabled, deck.loopStart < deck.loopEnd {
      let x1 = toX(deck.loopStart)
      let x2 = toX(deck.loopEnd)
      if x2 > x1 {
        context.setFillColor(Self.loopFill)
        context.fill(CGRect(x: x1, y: 0, width: x2 - x1, height: size.height))
        context.setStrokeColor(Self.loopEdge)
        context.setLineWidth(1)
        for x in [x1, x2] {
          context.move(to: CGPoint(x: x, y: 0))
          context.addLine(to: CGPoint(x: x, y: size.height))
        }
        context.strokePath()
      }
    }

    context.setStrokeColor(Self.playhead)
    context.setLineWidth(2)
    context.move(to: CGPoint(x: progressX, y: 0))
    context.addLine(to: CGPoint(x: progressX, y: size.height))
    context.strokePath()
  }

  override func mouseDown(with event: NSEvent) {
    guard mode == .full, let model, bounds.width > 0 else { return }
    let x = convert(event.locationInWindow, from: nil).x
    model.seek(index, Float(min(max(x / bounds.width, 0), 1)))
  }
}

private struct WaveformRepresentable: NSViewRepresentable {
  @Environment(ConsoleModel.self) private var model
  let index: Int
  let mode: WaveformNSView.Mode

  func makeNSView(context: Context) -> WaveformNSView {
    let view = WaveformNSView()
    view.index = index
    view.mode = mode
    view.model = model
    view.wantsLayer = true
    return view
  }

  func updateNSView(_ view: WaveformNSView, context: Context) {
    view.index = index
    view.mode = mode
    if view.model !== model { view.model = model }
  }
}

/// The 8-second window around the playhead, scaled by the tempo ratio.
struct ZoomWaveformView: View {
  let index: Int

  var body: some View {
    WaveformRepresentable(index: index, mode: .zoom)
      .frame(height: 56)
  }
}

/// The whole track; click to seek.
struct FullWaveformView: View {
  let index: Int
  var height: CGFloat = 50

  var body: some View {
    WaveformRepresentable(index: index, mode: .full)
      .frame(height: height)
  }
}
