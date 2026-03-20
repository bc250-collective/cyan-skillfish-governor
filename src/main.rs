mod config;
mod gpu;
use config::Config;
use gpu::GPU;
use std::io::{Error as IoError, ErrorKind};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct CoreSnapshot {
    total: u64,
    idle: u64,
}

struct CpuSnapshot {
    cores: Vec<CoreSnapshot>,
}

fn read_cpu_snapshot() -> Result<CpuSnapshot, IoError> {
    let stat = std::fs::read_to_string("/proc/stat")?;
    let mut cores = Vec::new();

    for line in stat.lines() {
        let Some(cpu_label) = line.split_whitespace().next() else {
            continue;
        };
        let is_per_core = cpu_label
            .strip_prefix("cpu")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()));
        if !is_per_core {
            continue;
        }

        let mut total: u64 = 0;
        let mut idle: u64 = 0;
        for (i, field) in line.split_whitespace().skip(1).enumerate() {
            let value = field.parse::<u64>().map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidData,
                    "failed to parse /proc/stat per-core counter",
                )
            })?;
            total = total.saturating_add(value);
            if i == 3 || i == 4 {
                idle = idle.saturating_add(value);
            }
        }

        if total > 0 {
            cores.push(CoreSnapshot { total, idle });
        }
    }

    if cores.is_empty() {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "no per-core CPU counters found in /proc/stat",
        ));
    }

    Ok(CpuSnapshot { cores })
}

fn max_core_load_fraction(prev: &CpuSnapshot, current: &CpuSnapshot) -> f32 {
    let mut max_load = 0.0_f32;
    for (prev_core, curr_core) in prev.cores.iter().zip(current.cores.iter()) {
        let total_delta = curr_core.total.saturating_sub(prev_core.total);
        if total_delta == 0 {
            continue;
        }

        let idle_delta = curr_core
            .idle
            .saturating_sub(prev_core.idle)
            .min(total_delta);
        let busy_delta = total_delta.saturating_sub(idle_delta);
        let core_load = busy_delta as f32 / total_delta as f32;
        max_load = max_load.max(core_load);
    }
    max_load
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::new(
        std::env::args()
            .nth(1)
            .map(std::fs::read_to_string)
            .unwrap_or(Ok("".to_string())),
    )?;

    let mut gpu = GPU::new(config.safe_points)?;

    let mut curr_freq: u32 = gpu.get_freq()?;
    let mut target_freq = gpu.min_freq;
    let mut status: i16 = 0;
    const UP_EVENTS: i16 = 2;
    gpu.change_freq(target_freq)?;
    let mut max_freq = gpu.max_freq;
    let mut cpu_snapshot = read_cpu_snapshot()?;
    let mut cpu_load = 0.0_f32;
    let mut cpu_profile_boost = false;
    const CPU_CHECK_INTERVAL: Duration = Duration::from_millis(500);
    let mut last_cpu_check = Instant::now();

    let burst_freq_step =
        (config.ramp_rate_burst * config.adjustment_interval.as_millis() as f32) as u32;
    let freq_step = (config.ramp_rate * config.adjustment_interval.as_millis() as f32) as u32;
    println!("freq min {} max {} ", gpu.min_freq, max_freq);
    loop {
        let mut average_load: f32 = 0.0;
        let mut burst_length: u32 = 0;

        //fill the sample buffer
        for _ in 0..65 {
            (average_load, burst_length) = gpu.poll_and_get_load()?;
            std::thread::sleep(config.sampling_interval);
        }
        if last_cpu_check.elapsed() >= CPU_CHECK_INTERVAL {
            let next_cpu_snapshot = read_cpu_snapshot()?;
            cpu_load = max_core_load_fraction(&cpu_snapshot, &next_cpu_snapshot);
            cpu_snapshot = next_cpu_snapshot;
            last_cpu_check = Instant::now();

            if cpu_load > config.cpu_up_thresh {
                cpu_profile_boost = true;
            } else if cpu_load < config.cpu_down_thresh {
                cpu_profile_boost = false;
            }
        }

        let burst = config
            .burst_samples
            .map_or(false, |burst_samples| burst_length >= burst_samples);

        let safe_point_perf_profile = gpu.safe_point_perf_profile(curr_freq)?;
        let perf_profile_target = if cpu_profile_boost {
            3
        } else {
            safe_point_perf_profile
        };
        gpu.set_perf_profile(perf_profile_target)?;

        //Temperature Management
        let temp = gpu.read_temperature()?;
        if let Some(max_temp) = config.throttling_temp {
            if (temp > max_temp) && (max_freq >= gpu.min_freq + freq_step) {
                max_freq -= config.significant_change;
                println!("throttling temp {temp} freq {max_freq}");
            } else if let Some(recovery_temp) = config.throttling_recovery_temp
                && temp < recovery_temp
                && max_freq != gpu.max_freq
            {
                max_freq = gpu.max_freq;
                println!("recover throttling temp {temp} freq {max_freq}");
            }
        }

        if burst {
            target_freq += burst_freq_step;
        } else {
            if average_load > config.up_thresh && status <= UP_EVENTS {
                status += UP_EVENTS;
            } else if average_load < config.down_thresh && curr_freq > gpu.min_freq {
                status -= 1;
            } else if status < 0 {
                status += 1;
            } else if status > 0 {
                status -= 1;
            }

            if status <= -config.down_events {
                target_freq -= freq_step;
            } else if status >= UP_EVENTS {
                target_freq += freq_step;
            }
        }

        target_freq = target_freq.clamp(gpu.min_freq, max_freq);
        let hit_bounds = target_freq == gpu.min_freq || target_freq == max_freq;
        let big_change = curr_freq.abs_diff(target_freq) >= config.significant_change;

        if curr_freq != target_freq && (burst || hit_bounds || big_change) {
            let de = config.down_events;
            println!(
                "freq curr {curr_freq} target {target_freq} temp {temp} status {status} de {de} gpu_load {average_load} cpu_load {cpu_load} bl {burst_length} perf_profile {perf_profile_target}"
            );
            gpu.change_freq(target_freq)?;
            status = 0;
            curr_freq = target_freq;
        }

        std::thread::sleep(config.adjustment_interval - 64 * config.sampling_interval);
    }
}
