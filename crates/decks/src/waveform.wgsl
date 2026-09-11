struct ComputeParams {
  sample_count: u32,
  peak_bins: u32,
  _pad0: u32,
  _pad1: u32,
};

@group(0) @binding(0)
var<storage, read> samples: array<f32>;

@group(0) @binding(1)
var<storage, read_write> peaks: array<f32>;

@group(0) @binding(2)
var<uniform> compute_params: ComputeParams;

@compute @workgroup_size(64)
fn cs_peaks(@builtin(global_invocation_id) gid: vec3<u32>) {
  let bin = gid.x;
  if (bin >= compute_params.peak_bins) {
    return;
  }

  let sample_count = compute_params.sample_count;
  if (sample_count == 0u) {
    peaks[bin] = 0.0;
    return;
  }

  let start_idx = (bin * sample_count) / compute_params.peak_bins;
  let end_idx = ((bin + 1u) * sample_count) / compute_params.peak_bins;

  var peak = 0.0;
  var i = start_idx;
  loop {
    if (i >= end_idx || i >= sample_count) {
      break;
    }
    peak = max(peak, abs(samples[i]));
    i = i + 1u;
  }

  peaks[bin] = peak;
}

struct RenderParams {
  peak_bins: u32,
  width: f32,
  height: f32,
  time: f32,
};

@group(0) @binding(0)
var<storage, read> peaks_a: array<f32>;

@group(0) @binding(1)
var<storage, read> peaks_b: array<f32>;

@group(0) @binding(2)
var<uniform> render_params: RenderParams;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) idx: u32) -> VsOut {
  var positions = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
  );
  var uvs = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(1.0, 0.0),
  );

  let i = min(idx, 5u);
  let p = positions[i];
  var out: VsOut;
  out.pos = vec4<f32>(p, 0.0, 1.0);
  out.uv = uvs[i];
  return out;
}

fn sample_peak(uv_x: f32, bins: u32, peaks_buffer: ptr<storage, array<f32>, read>) -> f32 {
  if (bins == 0u) {
    return 0.0;
  }
  let clamped = clamp(uv_x, 0.0, 0.999999);
  let idx = min(u32(clamped * f32(bins)), bins - 1u);
  return clamp((*peaks_buffer)[idx], 0.0, 1.0);
}

@fragment
fn fs_waveform(in: VsOut) -> @location(0) vec4<f32> {
  let uv = vec2<f32>(
    clamp(in.uv.x, 0.0, 0.999999),
    clamp(in.uv.y, 0.0, 0.999999),
  );

  let bins = render_params.peak_bins;
  let amp_a = sample_peak(uv.x, bins, &peaks_a);
  let amp_b = sample_peak(uv.x, bins, &peaks_b);

  var color = vec3<f32>(0.06, 0.08, 0.12);

  let y = uv.y;
  let h = max(render_params.height, 1.0);

  // Mirror Web console layout: two zoom waveforms live only in the top section.
  // CSS reference: top padding 10px, waveform height 80px, gap 5px.
  let top_padding_px = 10.0;
  let waveform_h_px = 80.0;
  let waveform_gap_px = 5.0;

  let center_a = (top_padding_px + waveform_h_px * 0.5) / h;
  let center_b = (top_padding_px + waveform_h_px + waveform_gap_px + waveform_h_px * 0.5) / h;

  // Keep vertical thickness bounded by the waveform lane height.
  let min_half_px = 2.0;
  let max_half_px = waveform_h_px * 0.42;
  let half_a = max(min_half_px / h, (amp_a * max_half_px) / h);
  let dist_a = abs(y - center_a);
  if (dist_a <= half_a) {
    let edge = 1.0 - smoothstep(half_a - 0.01, half_a + 0.01, dist_a);
    color = mix(color, vec3<f32>(0.20, 0.72, 0.98), edge);
  }

  let half_b = max(min_half_px / h, (amp_b * max_half_px) / h);
  let dist_b = abs(y - center_b);
  if (dist_b <= half_b) {
    let edge = 1.0 - smoothstep(half_b - 0.01, half_b + 0.01, dist_b);
    color = mix(color, vec3<f32>(0.98, 0.56, 0.18), edge);
  }

  // Separator between zoom waveforms.
  let separator_y = (top_padding_px + waveform_h_px + waveform_gap_px * 0.5) / h;
  let separator = abs(y - separator_y);
  if (separator < (1.0 / h)) {
    color = mix(color, vec3<f32>(0.18, 0.20, 0.24), 0.9);
  }

  return vec4<f32>(color, 1.0);
}
