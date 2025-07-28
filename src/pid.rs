use humansize::{format_size, BINARY};
use humantime::format_duration;
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};

pub struct PidStats {
    pub start: Instant,
    pub mem: u64,
    pub cpu: f32,
    system: System,
}

impl Default for PidStats {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PidStats {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let elapsed = self.start;

        write!(
            f,
            "up: {} cpu: {:.1}% mem: {}",
            // hack to make the info! output not include ms/ns etc...
            format_duration(Duration::from_secs(
                Instant::now().duration_since(elapsed).as_secs()
            )),
            self.cpu,
            format_size(self.mem, BINARY)
        )
    }
}

impl PidStats {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            mem: 0,
            cpu: 0.0,
            system: System::new_all(),
        }
    }

    pub fn update(&mut self) {
        self.system.refresh_all();

        let pid = std::process::id();

        if let Some(process) = self.system.process(Pid::from_u32(pid)) {
            self.mem = process.memory() / 1024;
            self.cpu = process.cpu_usage();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pidstats_new() {
        let stats = PidStats::new();
        assert_eq!(stats.mem, 0);
        assert_eq!(stats.cpu, 0.0);
    }

    #[test]
    fn test_update() {
        let mut stats = PidStats::new();
        stats.update();
    }
}
