/* C ABI over the Rust audio engine and rekordbox reader (crates/ffi).
 * Hand-written; keep in step with crates/ffi/src/lib.rs. The engine handle
 * is used from one thread. Strings returned as char * are freed with
 * sujay_string_free. */
#ifndef SUJAY_H
#define SUJAY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

void sujay_string_free(char *s);

/* ── Engine ─────────────────────────────────────────────────────────────── */

typedef struct SujayEngine SujayEngine;

typedef struct SujayDeckState {
  double position_frames;   /* audio frames; meaningful when loaded */
  double total_frames;
  float peak;
  float peak_hold;
  float gain;
  float bpm;                /* 0 when unknown */
  float loop_start;         /* audio frames */
  float loop_end;
  uint8_t playing;
  uint8_t cue_enabled;
  uint8_t eq_low;
  uint8_t eq_mid;
  uint8_t eq_high;
  uint8_t loop_enabled;
  uint8_t loaded;
  uint8_t _pad;
} SujayDeckState;

typedef struct SujayEngineState {
  SujayDeckState deck[2];
  float master_tempo;
  float crossfader;
  float mic_peak;
  float sample_rate;
  uint8_t is_crossfading;
  uint8_t mic_available;
  uint8_t mic_enabled;
  uint8_t is_recording;
} SujayEngineState;

SujayEngine *sujay_engine_new(uint32_t sample_rate);      /* NULL on failure */
void sujay_engine_free(SujayEngine *engine);              /* closes the engine */
uint32_t sujay_engine_sample_rate(const SujayEngine *engine);
int32_t sujay_engine_configure_device(const SujayEngine *engine, const char *device_id,
                                      const int32_t main[2], const int32_t cue[2]);
int32_t sujay_engine_load_track(const SujayEngine *engine, uint8_t deck, const float *pcm_interleaved,
                                size_t frames, float bpm, const float *beats_frames, size_t beat_count,
                                const char *track_id);
void sujay_engine_play(const SujayEngine *engine, uint8_t deck);
void sujay_engine_stop(const SujayEngine *engine, uint8_t deck);
void sujay_engine_seek(const SujayEngine *engine, uint8_t deck, double position);   /* 0..1 */
void sujay_engine_set_crossfader(const SujayEngine *engine, double position);
void sujay_engine_set_master_tempo(const SujayEngine *engine, double bpm);
void sujay_engine_set_deck_gain(const SujayEngine *engine, uint8_t deck, double gain);
void sujay_engine_set_eq(const SujayEngine *engine, uint8_t deck, uint8_t band, bool kill); /* 0 low 1 mid 2 high */
void sujay_engine_set_cue(const SujayEngine *engine, uint8_t deck, bool enabled);
void sujay_engine_set_mic_enabled(const SujayEngine *engine, bool enabled);
void sujay_engine_set_loop(const SujayEngine *engine, uint8_t deck, double start, double end, bool enabled); /* 0..1 */
void sujay_engine_set_beat_loop(const SujayEngine *engine, uint8_t deck, double start_seconds, double end_seconds);
void sujay_engine_clear_loop(const SujayEngine *engine, uint8_t deck);
int32_t sujay_engine_start_recording(const SujayEngine *engine, const char *path, uint8_t format); /* 0 wav 1 ogg */
void sujay_engine_stop_recording(const SujayEngine *engine);
void sujay_engine_state(const SujayEngine *engine, SujayEngineState *out);

/* [{name, max_output_channels}] sorted by name; reads the HAL property API. */
char *sujay_list_output_devices_json(void);

/* ── Rekordbox ──────────────────────────────────────────────────────────── */

/* {master_db_path, tracks[], playlists[]} or {"error": "..."}; master_db NULL = newest under ~/Library/Pioneer */
char *sujay_library_load_json(const char *master_db);
/* {beats_ms[], cues[], waveform_rgb[]} or {"error": "..."} */
char *sujay_library_track_analysis_json(const char *master_db, const char *content_id);

#ifdef __cplusplus
}
#endif

#endif /* SUJAY_H */
