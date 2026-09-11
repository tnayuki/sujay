/* C ABI of the Sujay core (crates/ffi). Hand-written; keep in step with
 * crates/ffi/src/lib.rs. Every function must be called from one thread. */
#ifndef SUJAY_H
#define SUJAY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SujayCore SujayCore;

typedef struct SujayTick {
  uint8_t console;      /* titles / bpm text / cues changed: re-read console json */
  uint8_t library;      /* library list changed: re-read library json */
  uint8_t preferences;  /* preferences changed: re-read preferences json */
  uint8_t retry_soon;   /* a decoded track is waiting for the engine; tick again ~1 ms */
  uint8_t deck[2];      /* waveform / colours / beat grid replaced for deck A / B */
  uint8_t _pad[2];
} SujayTick;

typedef struct SujayDeckSnapshot {
  float position_frames;
  float total_frames;
  float sample_rate;
  float peak;
  float gain;
  float bpm;
  float loop_start;   /* audio frames */
  float loop_end;     /* audio frames */
  float loop_beats;   /* 0 when no standard loop length is active */
  uint8_t playing;
  uint8_t cue_enabled;
  uint8_t eq_low;
  uint8_t eq_mid;
  uint8_t eq_high;
  uint8_t loop_enabled;
  uint8_t loaded;
  uint8_t _pad;
} SujayDeckSnapshot;

typedef struct SujaySnapshot {
  SujayDeckSnapshot deck[2];
  float master_tempo;
  float crossfader;
  float cpu_percent;
  float mic_peak;
  uint64_t mem_mb;
  uint32_t rec_elapsed_secs;
  uint8_t mic_available;
  uint8_t mic_enabled;
  uint8_t is_recording;
  uint8_t _pad;
} SujaySnapshot;

/* Lifecycle */
SujayCore *sujay_core_new(void);
void sujay_core_free(SujayCore *core);
int32_t sujay_core_start(SujayCore *core);
void sujay_core_shutdown(SujayCore *core);

/* Per frame */
SujayTick sujay_core_tick(SujayCore *core);
void sujay_core_snapshot(const SujayCore *core, SujaySnapshot *out);

/* Slow state as JSON; free the returned string with sujay_string_free. */
char *sujay_core_console_json(const SujayCore *core);
char *sujay_core_library_json(const SujayCore *core);
char *sujay_core_preferences_json(const SujayCore *core);
void sujay_string_free(char *s);

/* Bulk buffers; deck is 1 = A, 2 = B. */
size_t sujay_core_waveform_len(const SujayCore *core, uint8_t deck);
size_t sujay_core_copy_waveform(const SujayCore *core, uint8_t deck, float *out, size_t cap);
size_t sujay_core_waveform_colors_len(const SujayCore *core, uint8_t deck);
size_t sujay_core_copy_waveform_colors(const SujayCore *core, uint8_t deck, uint8_t *out_rgb, size_t cap_triplets);
size_t sujay_core_beats_len(const SujayCore *core, uint8_t deck);
size_t sujay_core_copy_beats(const SujayCore *core, uint8_t deck, float *out, size_t cap);
uint8_t sujay_core_deck_markers(const SujayCore *core, uint8_t deck, float *intro, float *outro);

/* Commands */
void sujay_core_play(const SujayCore *core, uint8_t deck);
void sujay_core_stop(const SujayCore *core, uint8_t deck);
void sujay_core_set_crossfader(const SujayCore *core, float position);
void sujay_core_set_master_tempo(const SujayCore *core, float bpm);
void sujay_core_set_deck_gain(const SujayCore *core, uint8_t deck, float gain);
void sujay_core_set_cue(const SujayCore *core, uint8_t deck, bool enabled);
void sujay_core_set_eq(const SujayCore *core, uint8_t deck, uint8_t band, bool kill);
void sujay_core_seek(const SujayCore *core, uint8_t deck, float position);
void sujay_core_recall_cue(const SujayCore *core, uint8_t deck, float position, float loop_end);
void sujay_core_toggle_loop(const SujayCore *core, uint8_t deck, float beats);
void sujay_core_set_mic_enabled(const SujayCore *core, bool enabled);
void sujay_core_start_recording(const SujayCore *core);
void sujay_core_stop_recording(const SujayCore *core);
void sujay_core_load_file(const SujayCore *core, uint8_t deck, const char *path);
void sujay_core_refresh_audio_devices(SujayCore *core);
int32_t sujay_core_apply_preferences_json(SujayCore *core, const char *json);

#ifdef __cplusplus
}
#endif

#endif /* SUJAY_H */
