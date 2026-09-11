import SwiftUI

/// The centre column: master tempo readout and the mixer section (EQ kills,
/// deck gain, level meters, cue monitors).
struct TempoView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    VStack(spacing: 10) {
      HStack(spacing: 5) {
        tempoArrow("▲", delta: 1)
        tempoDisplay
        tempoArrow("▼", delta: -1)
      }
      .frame(height: 40)

      HStack(alignment: .top, spacing: 5) {
        EQKillColumn(index: 0)
        gainColumn(index: 0)
        meterColumn(index: 0)
        meterColumn(index: 1)
        gainColumn(index: 1)
        EQKillColumn(index: 1)
      }
      .frame(height: 100)
    }
    .frame(maxWidth: .infinity, alignment: .top)
  }

  private func tempoArrow(_ symbol: String, delta: Float) -> some View {
    Button {
      model.nudgeMasterTempo(delta)
    } label: {
      Text(symbol)
        .font(.system(size: 10))
        .foregroundStyle(Theme.cyan)
        .frame(width: 24, height: 24)
    }
    .buttonStyle(ConsoleButtonStyle(cornerRadius: 3))
  }

  private var tempoDisplay: some View {
    let tempo = Int(min(max(model.snapshot.master_tempo.rounded(), 0), 999))
    return Text("\(tempo)")
      .font(Theme.dseg7(24))
      .foregroundStyle(Theme.orange)
      .shadow(color: Theme.orange.opacity(0.6), radius: 4)
      .frame(width: 80, height: 40)
      .background(Color.black)
      .clipShape(RoundedRectangle(cornerRadius: 2))
      .overlay(RoundedRectangle(cornerRadius: 2).stroke(Theme.bgDark, lineWidth: 2))
  }

  private func gainColumn(index: Int) -> some View {
    let gain = model.deck(index).gain
    return VStack(spacing: 2) {
      GainSlider(index: index, gain: gain)
      Text("\(Int((gain * 100).rounded()))%")
        .font(Theme.pixel(8)).foregroundStyle(Theme.textDim)
    }
    .frame(width: 18)
  }

  private func meterColumn(index: Int) -> some View {
    VStack(spacing: 6) {
      LevelMeter(peak: model.deck(index).peak, hold: model.peakHold[index])
        .frame(width: 10, height: 60)
      CueButton(index: index)
    }
    .frame(width: 32)
  }
}

/// Pioneer-style 15-segment LED meter, -24 dB to +13 dB with an +8 dB offset.
struct LevelMeter: View {
  let peak: Float
  let hold: Float

  private static func db(_ peak: Float) -> Float {
    guard peak > 0 else { return -.infinity }
    return min(20 * log10(peak) + 8, 13)
  }

  var body: some View {
    Canvas { context, size in
      context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Theme.bgDeep))
      let minDb: Float = -24
      let maxDb: Float = 13
      let segments = 15
      let segmentHeight = size.height / CGFloat(segments)
      let stepDb = (maxDb - minDb) / Float(segments - 1)
      let current = Self.db(peak)
      let held = Self.db(hold)
      for i in 0..<segments {
        let segmentDb = minDb + Float(i) * stepDb
        let color: Color =
          i >= 13 ? Theme.meterRed : (i >= 9 ? Theme.meterOrange : Theme.meterGreen)
        let lit = current >= segmentDb || (held >= segmentDb && held < segmentDb + stepDb)
        guard lit else { continue }
        let y = size.height - CGFloat(i + 1) * segmentHeight
        context.fill(
          Path(CGRect(x: 0, y: y, width: size.width, height: segmentHeight - 1)),
          with: .color(color))
      }
    }
  }
}

/// Vertical deck gain fader, 18 × 70.
struct GainSlider: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int
  let gain: Float

  var body: some View {
    GeometryReader { geometry in
      let height = geometry.size.height
      let fill = CGFloat(min(max(gain, 0), 1)) * height
      ZStack(alignment: .bottom) {
        RoundedRectangle(cornerRadius: 2).fill(Theme.bgDark)
        if fill > 1 {
          RoundedRectangle(cornerRadius: 1)
            .fill(Theme.cyan.opacity(0.6))
            .frame(height: fill - 1)
            .padding(.horizontal, 1)
            .padding(.bottom, 1)
        }
        RoundedRectangle(cornerRadius: 1)
          .fill(Theme.gradient135(Theme.borderDim, Theme.panel))
          .overlay(RoundedRectangle(cornerRadius: 1).stroke(Theme.rgb(102, 102, 102), lineWidth: 1))
          .frame(height: 8)
          .offset(y: -(fill - 4))
      }
      .overlay(RoundedRectangle(cornerRadius: 2).stroke(Theme.borderMed, lineWidth: 1))
      .contentShape(Rectangle())
      .gesture(
        DragGesture(minimumDistance: 0).onChanged { value in
          model.setDeckGain(index, Float(1 - min(max(value.location.y / height, 0), 1)))
        })
    }
    .frame(width: 18, height: 70)
  }
}

struct EQKillColumn: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let deck = model.deck(index)
    VStack(spacing: 3) {
      killButton("H", .high, active: deck.eq_high != 0)
      killButton("M", .mid, active: deck.eq_mid != 0)
      killButton("L", .low, active: deck.eq_low != 0)
    }
  }

  private func killButton(_ label: String, _ band: EQBand, active: Bool) -> some View {
    Button {
      model.setEQ(index, band, kill: !active)
    } label: {
      Text(label)
        .font(Theme.pixel(10))
        .foregroundStyle(active ? .white : Theme.textDim)
        .frame(width: 32, height: 18)
    }
    .buttonStyle(ConsoleButtonStyle(active: active))
  }
}

struct CueButton: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let enabled = model.deck(index).cue_enabled != 0
    Button {
      model.toggleCue(index)
    } label: {
      Image(systemName: "headphones")
        .font(.system(size: 14, weight: .semibold))
        .foregroundStyle(enabled ? Theme.cyan : Theme.textDim)
        .frame(width: 32, height: 30)
    }
    .buttonStyle(
      ConsoleButtonStyle(
        active: enabled, activeTop: Theme.rgb(32, 42, 58), activeBottom: Theme.rgb(15, 22, 32),
        activeBorder: Theme.cyan, cornerRadius: 4, glow: Theme.cyan))
  }
}
