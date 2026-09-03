//! CPU and memory readouts, straight from `/proc`.
//!
//! Deliberately allocation-light and sampled once a second: a panel widget that
//! measures load must not be a noticeable part of it.

use std::time::Instant;

/// A snapshot of the counters `/proc/stat` reports for the whole machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuSample {
    pub idle: u64,
    pub total: u64,
}

impl CpuSample {
    /// Parse the aggregate `cpu` line of `/proc/stat`.
    pub fn parse(stat: &str) -> Option<Self> {
        let line = stat.lines().find(|l| l.starts_with("cpu "))?;
        let mut fields = line.split_whitespace().skip(1).filter_map(|f| f.parse::<u64>().ok());

        let user = fields.next()?;
        let nice = fields.next()?;
        let system = fields.next()?;
        let idle = fields.next()?;
        let iowait = fields.next().unwrap_or(0);
        let rest: u64 = fields.sum();

        let idle_total = idle + iowait;
        Some(Self { idle: idle_total, total: user + nice + system + idle_total + rest })
    }

    /// Busy fraction between two samples, in `0.0..=1.0`.
    ///
    /// Returns `0.0` when the counters did not move or went backwards, which
    /// happens across a suspend; reporting nonsense would be worse than
    /// reporting idle.
    pub fn usage_since(&self, previous: &CpuSample) -> f32 {
        let total = self.total.saturating_sub(previous.total);
        if total == 0 {
            return 0.0;
        }
        let idle = self.idle.saturating_sub(previous.idle);
        ((total.saturating_sub(idle)) as f32 / total as f32).clamp(0.0, 1.0)
    }
}

/// Memory in use, as a fraction and in kibibytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Memory {
    pub used_kib: u64,
    pub total_kib: u64,
}

impl Memory {
    /// Parse `/proc/meminfo`.
    ///
    /// "Used" is total minus available, the same definition `free` uses, so the
    /// number matches what the user sees elsewhere rather than counting cache
    /// as used.
    pub fn parse(meminfo: &str) -> Option<Self> {
        let field = |name: &str| -> Option<u64> {
            meminfo
                .lines()
                .find(|l| l.starts_with(name))?
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()
        };
        let total = field("MemTotal:")?;
        let available = field("MemAvailable:").unwrap_or(0);
        Some(Self { used_kib: total.saturating_sub(available), total_kib: total })
    }

    /// Used by the tests and by the forthcoming memory meter.
    #[allow(dead_code)]
    pub fn fraction(&self) -> f32 {
        if self.total_kib == 0 {
            return 0.0;
        }
        (self.used_kib as f32 / self.total_kib as f32).clamp(0.0, 1.0)
    }

    pub fn used_gib(&self) -> f32 {
        self.used_kib as f32 / 1024.0 / 1024.0
    }
}

/// Samples the readouts, no more often than once a second.
pub struct Readout {
    previous_cpu: CpuSample,
    cpu: f32,
    memory: Memory,
    last_sample: Option<Instant>,
}

impl Default for Readout {
    fn default() -> Self {
        Self::new()
    }
}

impl Readout {
    pub fn new() -> Self {
        Self {
            previous_cpu: CpuSample::default(),
            cpu: 0.0,
            memory: Memory::default(),
            last_sample: None,
        }
    }

    /// Re-read `/proc` if a second has passed. Returns `true` when the values
    /// changed enough to be worth a repaint.
    pub fn refresh(&mut self) -> bool {
        let now = Instant::now();
        if let Some(last) = self.last_sample {
            if now.duration_since(last).as_millis() < 900 {
                return false;
            }
        }
        self.last_sample = Some(now);

        let before = (self.cpu, self.memory);

        if let Some(sample) = std::fs::read_to_string("/proc/stat").ok().and_then(|s| CpuSample::parse(&s))
        {
            if self.previous_cpu.total != 0 {
                self.cpu = sample.usage_since(&self.previous_cpu);
            }
            self.previous_cpu = sample;
        }
        if let Some(memory) =
            std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| Memory::parse(&s))
        {
            self.memory = memory;
        }

        // A percentage point of CPU is below the noise floor of a 1 Hz sample.
        (self.cpu - before.0).abs() >= 0.01 || self.memory != before.1
    }

    /// The line the panel draws, e.g. `CPU  12%  MEM  2.1G`.
    ///
    /// Every field is padded to its widest form. A readout that changes width
    /// would shove the clock sideways once a second.
    pub fn label(&self) -> String {
        // Padded so the panel does not twitch as the numbers change width.
        format!(
            "CPU {:>3.0}%  MEM {:>4.1}G",
            (self.cpu * 100.0).min(100.0),
            self.memory.used_gib().min(999.9)
        )
    }

    /// The two readings on their own, for a panel too narrow for one line.
    pub fn parts(&self) -> (String, String) {
        (
            format!("{:.0}%", (self.cpu * 100.0).min(100.0)),
            format!("{:.1}G", self.memory.used_gib().min(999.9)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 10 40 800 20 0 5 0 0 0\ncpu0 50 5 20 400 10 0 2 0 0 0\n";

    #[test]
    fn the_aggregate_cpu_line_is_parsed() {
        let s = CpuSample::parse(STAT).unwrap();
        assert_eq!(s.idle, 820, "idle plus iowait");
        assert_eq!(s.total, 975);
    }

    #[test]
    fn a_proc_stat_without_a_cpu_line_is_rejected() {
        assert!(CpuSample::parse("intr 1 2 3\n").is_none());
        assert!(CpuSample::parse("").is_none());
    }

    #[test]
    fn usage_is_the_non_idle_share() {
        let a = CpuSample { idle: 100, total: 200 };
        let b = CpuSample { idle: 150, total: 300 };
        assert!((b.usage_since(&a) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn identical_samples_report_idle_rather_than_dividing_by_zero() {
        let a = CpuSample { idle: 100, total: 200 };
        assert_eq!(a.usage_since(&a), 0.0);
    }

    #[test]
    fn counters_going_backwards_report_idle() {
        let later = CpuSample { idle: 10, total: 20 };
        let earlier = CpuSample { idle: 1000, total: 2000 };
        assert_eq!(later.usage_since(&earlier), 0.0);
    }

    #[test]
    fn memory_uses_available_not_free() {
        let meminfo = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    8000000 kB\n";
        let m = Memory::parse(meminfo).unwrap();
        assert_eq!(m.used_kib, 8_000_000);
        assert!((m.fraction() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_meminfo_without_available_still_parses() {
        let m = Memory::parse("MemTotal:       1000 kB\n").unwrap();
        assert_eq!(m.total_kib, 1000);
        assert_eq!(m.used_kib, 1000);
    }

    #[test]
    fn a_meminfo_without_total_is_rejected() {
        assert!(Memory::parse("MemFree: 100 kB\n").is_none());
    }

    #[test]
    fn an_empty_memory_reading_does_not_divide_by_zero() {
        assert_eq!(Memory::default().fraction(), 0.0);
    }

    #[test]
    fn the_label_is_stable_in_width() {
        // A readout whose width changes shoves everything to its right along
        // once a second, which is exactly the kind of twitch a panel must not
        // have.
        let cases = [
            (0.0, 0),
            (0.05, 1_048_576),
            (0.5, 8_388_608),
            (1.0, 15_728_640),
        ];
        let widths: Vec<usize> = cases
            .iter()
            .map(|&(cpu, used_kib)| {
                Readout {
                    cpu,
                    memory: Memory { used_kib, total_kib: 16_777_216 },
                    ..Readout::new()
                }
                .label()
                .chars()
                .count()
            })
            .collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "widths differ: {widths:?}");
    }
}
