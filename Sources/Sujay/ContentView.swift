import SwiftUI

struct ContentView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    VStack(spacing: 12) {
      VStack(spacing: 6) {
        ZoomWaveformView(index: 0)
        ZoomWaveformView(index: 1)
      }

      HStack(alignment: .top, spacing: 12) {
        DeckView(index: 0)
        MixerView()
          .frame(width: 220)
        DeckView(index: 1)
      }
      .fixedSize(horizontal: false, vertical: true)

      CrossfaderView()

      LibraryView()
    }
    .padding(12)
    .toolbar { StatusToolbar() }
  }
}
