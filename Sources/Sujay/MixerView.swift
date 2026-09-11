import SwiftUI

/// The centre column: master tempo and the mixer section (EQ kills, deck gain,
/// level meters, cue monitors).
struct MixerView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    GroupBox {
      VStack(spacing: 12) {
        tempo
        Divider()
        // Two rows so the kills, faders and meters share one height, with the
        // gain readouts and cue monitors on a row of their own beneath.
        Grid(horizontalSpacing: 8, verticalSpacing: 6) {
          GridRow {
            EQKillColumn(index: 0)
            GainSlider(index: 0, gain: model.deck(0).gain)
            LevelMeter(index: 0)
              .frame(width: 10)
            LevelMeter(index: 1)
              .frame(width: 10)
            GainSlider(index: 1, gain: model.deck(1).gain)
            EQKillColumn(index: 1)
          }
          .frame(height: Self.columnHeight)
          GridRow {
            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
            gainReadout(index: 0)
            CueButton(index: 0)
            CueButton(index: 1)
            gainReadout(index: 1)
            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
          }
        }
        Spacer(minLength: 0)
      }
      .padding(4)
      // Fill the row so the box is as tall as the decks beside it.
      .frame(maxHeight: .infinity)
    } label: {
      Label("Mixer", systemImage: "slider.horizontal.3")
    }
  }

  private var tempo: some View {
    let binding = Binding<Int>(
      get: { Int(model.masterTempo.rounded()) },
      set: { model.setMasterTempo(Float($0)) })
    return LabeledContent {
      Stepper(value: binding, in: 60...200) {
        Text("\(binding.wrappedValue) BPM")
          .font(.title3.monospacedDigit().weight(.semibold))
      }
    } label: {
      Text("Tempo")
    }
  }

  /// Shared height of the kills, faders and meters.
  static let columnHeight: CGFloat = 72

  private func gainReadout(index: Int) -> some View {
    Text("\(Int((model.deck(index).gain * 100).rounded()))%")
      .font(.caption2.monospacedDigit())
      .foregroundStyle(.secondary)
      .lineLimit(1)
      .fixedSize()
  }
}

/// Pioneer-style 15-segment LED meter, -24 dB to +13 dB with an +8 dB offset,
/// drawn in an NSView on the frame tick so SwiftUI never lays it out per frame.
final class LevelMeterNSView: NSView {
  var model: ConsoleModel? {
    didSet { subscribe() }
  }
  var index = 0
  private var listener: UUID?

  override var isFlipped: Bool { true }

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

  private static func db(_ peak: Float) -> Float {
    guard peak > 0 else { return -.infinity }
    return min(20 * log10(peak) + 8, 13)
  }

  override func draw(_ dirtyRect: NSRect) {
    guard let context = NSGraphicsContext.current?.cgContext, let model else { return }
    context.setShouldAntialias(false)
    let deck = model.deck(index)
    let size = bounds.size
    let minDb: Float = -24
    let maxDb: Float = 13
    let segments = 15
    let segmentHeight = size.height / CGFloat(segments)
    let stepDb = (maxDb - minDb) / Float(segments - 1)
    let current = Self.db(deck.peak)
    let held = Self.db(deck.peakHold)
    for i in 0..<segments {
      let segmentDb = minDb + Float(i) * stepDb
      let color: NSColor = i >= 13 ? .systemRed : (i >= 9 ? .systemOrange : .systemGreen)
      let lit = current >= segmentDb || (held >= segmentDb && held < segmentDb + stepDb)
      let y = size.height - CGFloat(i + 1) * segmentHeight
      context.setFillColor((lit ? color : color.withAlphaComponent(0.15)).cgColor)
      context.fill(CGRect(x: 0, y: y, width: size.width, height: segmentHeight - 1))
    }
  }
}

struct LevelMeter: NSViewRepresentable {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  func makeNSView(context: Context) -> LevelMeterNSView {
    let view = LevelMeterNSView()
    view.index = index
    view.model = model
    return view
  }

  func updateNSView(_ view: LevelMeterNSView, context: Context) {
    view.index = index
    if view.model !== model { view.model = model }
  }
}

/// Deck gain as a standard slider, stood on end.
struct GainSlider: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int
  let gain: Float

  var body: some View {
    let binding = Binding<Double>(
      get: { Double(gain) },
      set: { model.setDeckGain(index, Float($0)) })
    Slider(value: binding, in: 0...1)
      .labelsHidden()
      .controlSize(.small)
      .frame(width: MixerView.columnHeight)
      .rotationEffect(.degrees(-90))
      .frame(width: 20, height: MixerView.columnHeight)
  }
}

struct EQKillColumn: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let deck = model.deck(index)
    VStack(spacing: 0) {
      killButton("H", .high, active: deck.eqHigh)
      Spacer(minLength: 2)
      killButton("M", .mid, active: deck.eqMid)
      Spacer(minLength: 2)
      killButton("L", .low, active: deck.eqLow)
    }
    .frame(height: MixerView.columnHeight)
  }

  private func killButton(_ label: String, _ band: EQBand, active: Bool) -> some View {
    ActiveButton(
      active: active, tint: Theme.kill, action: { model.setEQ(index, band, kill: !active) }
    ) {
      Text(label).font(.caption.weight(.semibold)).frame(width: 18)
    }
    .controlSize(.small)
    .help("Kill \(band)")
  }
}

struct CueButton: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let enabled = model.deck(index).cueEnabled
    ActiveButton(active: enabled, tint: Theme.cue, action: { model.toggleCue(index) }) {
      Image(systemName: "headphones").frame(width: 18)
    }
    .help("Cue monitor")
  }
}
