use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuSample {
    pub idle: u64,
    pub total: u64,
}

impl CpuSample {
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

    pub fn usage_since(&self, previous: &CpuSample) -> f32 {
        let total = self.total.saturating_sub(previous.total);
        if total == 0 {
            return 0.0;
        }
        let idle = self.idle.saturating_sub(previous.idle);
        ((total.saturating_sub(idle)) as f32 / total as f32).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Memory {
    pub used_kib: u64,
    pub total_kib: u64,
}

impl Memory {
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

pub struct Readout {
    previous_cpu: CpuSample,
    cpu: f32,
    memory: Memory,
}

impl Default for Readout {
    fn default() -> Self {
        Self::new()
    }
}

impl Readout {
    pub fn new() -> Self {
        Self { previous_cpu: CpuSample::default(), cpu: 0.0, memory: Memory::default() }
    }

    pub fn refresh(&mut self) -> bool {
        let before = self.label();

        if let Some(sample) =
            std::fs::read_to_string("/proc/stat").ok().and_then(|s| CpuSample::parse(&s))
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

        self.label() != before
    }

    pub fn label(&self) -> String {
        format!(
            "CPU {:>3.0}%  MEM {:>4.1}G",
            (self.cpu * 100.0).min(100.0),
            self.memory.used_gib().min(999.9)
        )
    }

    pub fn parts(&self) -> (String, String) {
        (
            format!("{:.0}%", (self.cpu * 100.0).min(100.0)),
            format!("{:.1}G", self.memory.used_gib().min(999.9)),
        )
    }
}

const MONITORS: &[&str] = &[
    "plasma-systemmonitor",
    "gnome-system-monitor",
    "missioncenter",
    "xfce4-taskmanager",
];

const TERMINALS: &[&str] = &["foot", "alacritty", "kitty", "wezterm", "konsole", "xterm"];

const TOPS: &[&str] = &["btop", "htop"];

pub fn system_monitor() -> Option<String> {
    system_monitor_in(&path_dirs())
}

fn system_monitor_in(dirs: &[PathBuf]) -> Option<String> {
    if let Some(monitor) = MONITORS.iter().find(|name| runnable(dirs, name)) {
        return Some((*monitor).to_owned());
    }
    let terminal = TERMINALS.iter().find(|name| runnable(dirs, name))?;
    let top = TOPS.iter().find(|name| runnable(dirs, name))?;
    Some(format!("{terminal} -e {top}"))
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

fn runnable(dirs: &[PathBuf], name: &str) -> bool {
    dirs.iter().any(|dir| is_executable(&dir.join(name)))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
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
    fn the_totals_add_up() {
        let s = CpuSample::parse("cpu  100 0 50 850 0 0 0 0 0 0\ncpu0 1 2 3 4 0 0 0 0 0 0\n")
            .unwrap();
        assert_eq!(s.total, 1000);
        assert_eq!(s.idle, 850);
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
    fn nonsense_never_panics() {
        for text in ["", "cpu", "cpu  \n", "cpu  x y z\n", "\0\0", "MemTotal: kB\n"] {
            let _ = CpuSample::parse(text);
            let _ = Memory::parse(text);
        }
    }

    #[test]
    fn an_empty_memory_reading_does_not_divide_by_zero() {
        assert_eq!(Memory::default().fraction(), 0.0);
    }

    #[test]
    fn the_label_is_stable_in_width() {
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

    fn fake_binaries(names: &[&str]) -> tempdir::TempDir {
        let dir = tempdir::TempDir::new();
        for name in names {
            let path = dir.path().join(name);
            std::fs::write(&path, b"#!/bin/sh\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    #[test]
    fn without_a_path_there_is_no_system_monitor() {
        assert_eq!(system_monitor_in(&[]), None);
    }

    #[test]
    fn the_first_monitor_on_the_list_wins() {
        let dir = fake_binaries(&["gnome-system-monitor", "plasma-systemmonitor"]);
        assert_eq!(
            system_monitor_in(&[dir.path().to_owned()]).as_deref(),
            Some("plasma-systemmonitor")
        );
    }

    #[test]
    fn a_machine_with_no_monitor_falls_back_to_a_terminal() {
        let dir = fake_binaries(&["foot", "btop"]);
        assert_eq!(system_monitor_in(&[dir.path().to_owned()]).as_deref(), Some("foot -e btop"));
    }

    #[test]
    fn a_terminal_with_nothing_to_run_in_it_is_not_a_monitor() {
        let dir = fake_binaries(&["foot"]);
        assert_eq!(system_monitor_in(&[dir.path().to_owned()]), None);
    }

    #[test]
    fn a_file_that_is_not_executable_does_not_count() {
        let dir = tempdir::TempDir::new();
        std::fs::write(dir.path().join("plasma-systemmonitor"), b"not a program").unwrap();
        assert_eq!(system_monitor_in(&[dir.path().to_owned()]), None);
    }

    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                let unique = format!(
                    "spectre-readout-{}-{:?}",
                    std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap()
                );
                let path = std::env::temp_dir().join(unique);
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
