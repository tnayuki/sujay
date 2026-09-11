/* C ABI over the Rust rekordbox reader (crates/ffi). Hand-written; keep in
 * step with crates/ffi/src/lib.rs. Strings returned as char * are freed with
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

/* ── Rekordbox ──────────────────────────────────────────────────────────── */

/* {master_db_path, tracks[], playlists[]} or {"error": "..."}; master_db NULL = newest under ~/Library/Pioneer */
char *sujay_library_load_json(const char *master_db);
/* {beats_ms[], cues[], waveform_rgb[]} or {"error": "..."} */
char *sujay_library_track_analysis_json(const char *master_db, const char *content_id);

#ifdef __cplusplus
}
#endif

#endif /* SUJAY_H */
