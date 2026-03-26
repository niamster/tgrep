use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Default)]
struct PhaseStat {
    calls: u64,
    total: Duration,
}

impl PhaseStat {
    fn record(&mut self, elapsed: Duration) {
        self.calls += 1;
        self.total += elapsed;
    }
}

#[derive(Default)]
struct PhaseRegistry {
    phases: Mutex<BTreeMap<&'static str, PhaseStat>>,
}

static ENABLED: OnceLock<bool> = OnceLock::new();
static REGISTRY: OnceLock<PhaseRegistry> = OnceLock::new();

fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("TGREP_PROFILE")
            .ok()
            .is_some_and(|value| value != "0" && !value.is_empty())
    })
}

fn registry() -> &'static PhaseRegistry {
    REGISTRY.get_or_init(PhaseRegistry::default)
}

pub fn record(name: &'static str, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let mut phases = registry().phases.lock().unwrap();
    phases.entry(name).or_default().record(elapsed);
}

pub fn time<T, F>(name: &'static str, f: F) -> T
where
    F: FnOnce() -> T,
{
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    record(name, start.elapsed());
    result
}

pub fn report() {
    if !enabled() {
        return;
    }
    eprintln!("tgrep phase timings:");
    let phases = registry().phases.lock().unwrap();
    for (name, stat) in phases.iter() {
        eprintln!(
            "  {name:<22} calls={:<8} total={:.3}s",
            stat.calls,
            stat.total.as_secs_f64()
        );
    }
}
