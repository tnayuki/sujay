import Darwin
import Foundation

/// This process's CPU share and memory footprint, read from
/// `proc_pid_rusage`, the way hukan samples its own footprint. CPU is the
/// delta between two samples, so the first reading is 0.
struct SystemUsage {
  private var lastCPU: UInt64?
  private var lastSampleTime: UInt64?

  private static let ticksToNanos: Double = {
    var info = mach_timebase_info_data_t()
    mach_timebase_info(&info)
    return Double(info.numer) / Double(info.denom)
  }()

  mutating func sample() -> (cpuPercent: Double, memoryBytes: UInt64) {
    var info = rusage_info_v0()
    let result = withUnsafeMutablePointer(to: &info) { pointer -> Int32 in
      pointer.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { rebound in
        proc_pid_rusage(getpid(), RUSAGE_INFO_V0, rebound)
      }
    }
    guard result == 0 else { return (0, 0) }
    let now = DispatchTime.now().uptimeNanoseconds
    let cpu = info.ri_user_time + info.ri_system_time
    var percent = 0.0
    if let last = lastSampleTime, let previous = lastCPU, now > last, cpu >= previous {
      percent = Double(cpu - previous) * Self.ticksToNanos / Double(now - last) * 100
    }
    lastCPU = cpu
    lastSampleTime = now
    return (percent, info.ri_phys_footprint)
  }
}
