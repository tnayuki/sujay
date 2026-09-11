//! Diagnostic: does web-audio-api's device enumeration return on this machine?
//!
//! `enumerate_devices_sync` asks cpal about every device, and cpal answers by
//! creating an AudioUnit per device; on some machines (virtual or aggregate
//! devices present) that never returns. The engine avoids the call unless a
//! non-default output device is selected. Run with a timeout:
//! `perl -e 'alarm 20; exec @ARGV' target/debug/examples/enumerate_probe`
fn main() {
  let started = std::time::Instant::now();
  for device in web_audio_api::media_devices::enumerate_devices_sync() {
    println!(
      "{:?} {:?} id={}",
      device.kind(),
      device.label(),
      device.device_id()
    );
  }
  println!("enumerated in {:.2}s", started.elapsed().as_secs_f32());
}
