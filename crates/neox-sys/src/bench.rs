//! Short, repeatable micro-benchmarks (about 10 s): CPU, memory bandwidth and timer latency.
//! Each metric is the median of several runs after a warm-up. Results are noisy, so comparisons use a noise threshold.

use neox_core::autotune::BenchmarkResult;
use std::hint::black_box;
use std::time::{Duration, Instant};

pub fn run(progress: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Option<BenchmarkResult> {
    progress("Measuring: processor…");
    let cpu = median(5, cpu_once, cancelled)?;
    progress("Measuring: memory…");
    let mem = median(5, mem_once, cancelled)?;
    progress("Measuring: timer…");
    let (avg, p99) = timer(cancelled)?;
    Some(BenchmarkResult { cpu_ops_per_sec: cpu, memory_mbps: mem, timer_avg_ms: avg, timer_p99_ms: p99 })
}

fn median(runs: usize, f: fn() -> f64, cancelled: &dyn Fn() -> bool) -> Option<f64> {
    f(); // warm-up (caches, frequency ramp)
    let mut v = Vec::with_capacity(runs);
    for _ in 0..runs { if cancelled() { return None; } v.push(f()); }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

fn cpu_once() -> f64 {
    const N: u64 = 30_000_000;
    let (mut x, mut acc) = (88_172_645_463_325_252u64, 0u64);
    let t = Instant::now();
    for _ in 0..N {
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        acc = acc.wrapping_add(x.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    let secs = t.elapsed().as_secs_f64();
    black_box(acc);
    N as f64 / secs.max(1e-9)
}

fn mem_once() -> f64 {
    const SIZE: usize = 64 * 1024 * 1024;
    let mut a = vec![0u8; SIZE];
    let mut b = vec![0u8; SIZE];
    a[0] = 1;
    let t = Instant::now();
    for i in 0..6 { a[i] = i as u8; b.copy_from_slice(black_box(&a)); black_box(&b); }
    let secs = t.elapsed().as_secs_f64();
    6.0 * (SIZE as f64) / (1024.0 * 1024.0) / secs.max(1e-9)
}

fn timer(cancelled: &dyn Fn() -> bool) -> Option<(f64, f64)> {
    let mut s = Vec::with_capacity(300);
    for _ in 0..300 {
        if cancelled() { return None; }
        let t = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        s.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let avg = s.iter().sum::<f64>() / s.len() as f64;
    Some((avg, s[((s.len() as f64 * 0.99) as usize).saturating_sub(1)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_plausible_positive_numbers() {
        let r = run(&|_| {}, &|| false).unwrap();
        assert!(r.cpu_ops_per_sec > 1e6 && r.memory_mbps > 10.0);
        assert!(r.timer_avg_ms >= 0.9 && r.timer_p99_ms >= r.timer_avg_ms * 0.5);
    }

    #[test]
    fn can_be_cancelled() { assert!(run(&|_| {}, &|| true).is_none()); }
}
