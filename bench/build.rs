const CRATES: [&str; 8] = ["ahash", "foldhash", "gxhash", "rapidhash", "rustc-hash", "hothash", "wyhash", "xxhash-rust"];

fn main() {
    let rustc = std::process::Command::new(std::env::var("RUSTC").unwrap()).arg("-V").output().unwrap();
    println!("cargo:rustc-env=BENCH_RUSTC={}", String::from_utf8(rustc.stdout).unwrap().trim());
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default().replace('\x1f', " ");
    println!("cargo:rustc-env=BENCH_RUSTFLAGS={flags}");

    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let mut versions = Vec::new();
    for package in lock.split("[[package]]") {
        let field = |key: &str| package.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(" = \"")?.strip_suffix('"')).unwrap_or("");
        let (name, version, source) = (field("name"), field("version"), field("source"));
        if CRATES.contains(&name) {
            let git = source.rsplit_once('#').filter(|_| source.starts_with("git+")).map(|(_, c)| format!(" git {}", &c[..7]));
            versions.push(format!("{name} {version}{}", git.unwrap_or_default()));
        }
    }
    println!("cargo:rustc-env=BENCH_CRATES={}", versions.join(", "));
    println!("cargo:rerun-if-changed=Cargo.lock");
}
