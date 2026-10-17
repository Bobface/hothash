use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

use hothash_bench::{Hashes, Input, Measure, NAMES, chain};

#[rustfmt::skip]
const SIZES: [usize; 34] = [
    1, 2, 3, 4, 7, 8, 12, 16, 20, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512,
    1 << 10, 2 << 10, 4 << 10, 16 << 10, 64 << 10, 256 << 10, 1 << 20, 4 << 20, 16 << 20, 64 << 20, 256 << 20,
    1 << 30, 4 << 30, 16 << 30, 32 << 30,
];
const BIG: usize = 256 << 20;

fn batch(size: usize) -> usize {
    ((16 << 20) / size).clamp(1, 8192)
}

struct Batch<'a> {
    input: &'a Input,
    len: usize,
    n: usize,
}

impl Measure for Batch<'_> {
    type Out = (u64, u64);
    fn measure<F: Fn(&[u8]) -> u64>(&self, f: &F) -> (u64, u64) {
        let c = thread_cpu_ns();
        let t = Instant::now();
        black_box(chain(f, self.input, self.len, self.n));
        let wall = t.elapsed().as_nanos() as u64;
        (wall, thread_cpu_ns() - c)
    }
}

fn thread_cpu_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

fn cpu_list(s: &str) -> Vec<usize> {
    s.split(',')
        .filter(|p| !p.is_empty())
        .flat_map(|p| {
            let (a, b) = p.split_once('-').unwrap_or((p, p));
            a.parse::<usize>().unwrap()..=b.parse().unwrap()
        })
        .collect()
}

fn affinity() -> Vec<usize> {
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &mut set) }, 0);
    (0..libc::CPU_SETSIZE as usize).filter(|&c| unsafe { libc::CPU_ISSET(c, &set) }).collect()
}

fn cgroups_on(cpu: usize) -> Vec<String> {
    let mut on = Vec::new();
    for entry in std::fs::read_dir("/sys/fs/cgroup").unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path();
        let cpus = read(dir.join("cpuset.cpus.effective").to_str().unwrap());
        let populated = read(dir.join("cgroup.events").to_str().unwrap()).contains("populated 1");
        if name != "hothashbench.slice" && populated && cpu_list(&cpus).contains(&cpu) {
            on.push(name);
        }
    }
    on.sort();
    on
}

fn interrupts(cpu: usize) -> (u64, u64) {
    let text = read("/proc/interrupts");
    let mut lines = text.lines();
    let col = lines.next().unwrap().split_whitespace().position(|c| c == format!("CPU{cpu}")).unwrap();
    let (mut all, mut timer) = (0, 0);
    for line in lines {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or("");
        if let Some(n) = fields.nth(col).and_then(|f| f.parse::<u64>().ok()) {
            all += n;
            if name == "LOC:" {
                timer += n;
            }
        }
    }
    (all, timer)
}

fn involuntary_switches() -> i64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut ru) };
    ru.ru_nivcsw
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).map(|i| args[i + 1].clone())
}

fn main() {
    let out = arg("--out").expect("--out samples.jsonl");
    let meta = arg("--meta").expect("--meta meta.json");
    let rounds: usize = arg("--rounds").map_or(1000, |r| r.parse().unwrap());
    let big_every: usize = arg("--big-every").map_or(8, |r| r.parse().unwrap());

    let pinned = affinity();
    assert_eq!(pinned.len(), 1, "pin the bench to one CPU (taskset -c N); affinity is {pinned:?}");
    let cpu = pinned[0];
    let input = Input::new(SIZES[SIZES.len() - 1]);
    let hashes = Hashes::default();
    let cpufreq = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq");
    let siblings = cpu_list(&read(&format!("/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list")));
    let shared = cgroups_on(cpu);
    let mut freq_mhz: Vec<u32> = Vec::with_capacity(rounds);
    let (irq0, timer0) = interrupts(cpu);
    let ivcsw0 = involuntary_switches();

    let mut w = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    let start = Instant::now();
    for round in 0..rounds {
        if let Ok(khz) = read(&format!("{cpufreq}/cpuinfo_avg_freq")).parse::<u32>() {
            freq_mhz.push(khz / 1000);
        }
        for &size in SIZES.iter().filter(|&&s| s < BIG || round % big_every == 0) {
            let b = Batch { input: &input, len: black_box(size), n: batch(size) };
            for k in 0..NAMES.len() {
                let name = NAMES[(k + round) % NAMES.len()];
                let (wall, cpu) = hashes.with(name, &b);
                let ns = wall as f64 / b.n as f64;
                writeln!(
                    w,
                    r#"{{"round":{round},"size":{size},"hash":"{name}","ns":{ns:.4},"n":{},"wall_ns":{wall},"cpu_ns":{cpu}}}"#,
                    b.n
                )
                .unwrap();
            }
        }
    }
    w.flush().unwrap();
    let seconds = start.elapsed().as_secs_f64();
    let (irq1, timer1) = interrupts(cpu);
    let ivcsw = involuntary_switches() - ivcsw0;

    let cpuinfo = read("/proc/cpuinfo");
    let model = cpuinfo.lines().find_map(|l| l.strip_prefix("model name")).map_or("?", |m| m.trim_start_matches([' ', '\t', ':']));
    freq_mhz.sort_unstable();
    let fq = |q: f64| freq_mhz.get(((freq_mhz.len().max(1) - 1) as f64 * q) as usize).copied().unwrap_or(0);
    let alone = siblings == [cpu];
    let shared_json = shared.iter().map(|c| format!(r#""{c}""#)).collect::<Vec<_>>().join(",");
    std::fs::write(
        &meta,
        format!(
            concat!(
                r#"{{"cpu_model":"{model}","cpu":{cpu},"kernel":"{kernel}","isolated":{isolated},"smt_sibling_offline":{alone},"#,
                r#""cgroups_on_cpu":[{shared_json}],"governor":"{gov}","epp":"{epp}","boost":"{boost}","max_freq_mhz":{max_freq},"#,
                r#""freq_mhz":{{"min":{fmin},"p1":{fp1},"p50":{fp50},"p99":{fp99},"max":{fmax}}},"#,
                r#""irqs":{irqs},"timer_irqs":{timers},"involuntary_switches":{ivcsw},"rounds":{rounds},"big_from":{big_from},"big_every":{big_every},"seconds":{seconds:.1},"#,
                r#""rustc":"{rustc}","rustflags":"{rustflags}","crates":"{crates}"}}"#
            ),
            model = model,
            cpu = cpu,
            kernel = read("/proc/sys/kernel/osrelease"),
            isolated = alone && shared.is_empty(),
            alone = alone,
            shared_json = shared_json,
            gov = read(&format!("{cpufreq}/scaling_governor")),
            epp = read(&format!("{cpufreq}/energy_performance_preference")),
            boost = read(&format!("{cpufreq}/boost")),
            max_freq = read(&format!("{cpufreq}/scaling_max_freq")).parse::<u32>().unwrap_or(0) / 1000,
            fmin = fq(0.0),
            fp1 = fq(0.01),
            fp50 = fq(0.5),
            fp99 = fq(0.99),
            fmax = fq(1.0),
            irqs = irq1 - irq0,
            timers = timer1 - timer0,
            ivcsw = ivcsw,
            rounds = rounds,
            big_from = BIG,
            big_every = big_every,
            seconds = seconds,
            rustc = env!("BENCH_RUSTC"),
            rustflags = env!("BENCH_RUSTFLAGS"),
            crates = env!("BENCH_CRATES"),
        ),
    )
    .unwrap();
    eprintln!(
        "{rounds} rounds in {seconds:.1}s on cpu{cpu} -> {out}\n  isolated={} (sibling offline: {alone}, other cgroups on cpu: {shared:?}), clock p50 {} MHz [{}, {}], {} IRQs ({} timer), {ivcsw} involuntary switches",
        alone && shared.is_empty(),
        fq(0.5),
        fq(0.0),
        fq(1.0),
        irq1 - irq0,
        timer1 - timer0
    );
}
