#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct DeckConsoleVisualState {
  pub title: String,
  pub time_text: String,
  pub bpm_text: String,
  pub bpm: f32,
  pub playing: bool,
  pub loop_enabled: bool,
  pub loop_beats: f32,
  pub loop_start: f32,
  pub loop_end: f32,
  pub cue_enabled: bool,
  pub eq_low: bool,
  pub eq_mid: bool,
  pub eq_high: bool,
  pub gain: f32,
  pub peak: f32,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct ConsoleVisualState {
  pub deck_a: DeckConsoleVisualState,
  pub deck_b: DeckConsoleVisualState,
  pub master_tempo: f32,
  pub crossfader: f32,
}

impl Default for ConsoleVisualState {
  fn default() -> Self {
    Self {
      deck_a: DeckConsoleVisualState::default(),
      deck_b: DeckConsoleVisualState::default(),
      master_tempo: 130.0,
      crossfader: 0.5,
    }
  }
}
