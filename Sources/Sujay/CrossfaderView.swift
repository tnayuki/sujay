import SwiftUI

struct CrossfaderView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    let binding = Binding<Double>(
      get: { Double(model.crossfader) },
      set: { model.setCrossfader(Float($0)) })
    Slider(value: binding, in: 0...1) {
      Text("Crossfader")
    } minimumValueLabel: {
      Text("A").font(.headline).foregroundStyle(Theme.deckA)
    } maximumValueLabel: {
      Text("B").font(.headline).foregroundStyle(Theme.deckB)
    }
    .labelsHidden()
    .controlSize(.large)
  }
}
