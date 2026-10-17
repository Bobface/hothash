use std::hash::{BuildHasher, Hasher};

pub const SEED: u64 = 0x9e37_79b9_7f4a_7c15;

pub const NAMES: [&str; 9] = ["hothash", "foldhash", "fxhash", "xxh3", "rapidhash", "gxhash", "gxhash256", "wyhash", "ahash"];

pub trait Measure {
    type Out;
    fn measure<F: Fn(&[u8]) -> u64>(&self, f: &F) -> Self::Out;
}

pub struct Hashes {
    hothash: hothash::Keys,
    foldhash: foldhash::fast::FixedState,
    xxh3: [u8; 192],
    rapidhash: rapidhash::v3::RapidSecrets,
    wyhash: [u64; 4],
    ahash: ahash::RandomState,
}

impl Default for Hashes {
    fn default() -> Hashes {
        Hashes {
            hothash: hothash::Keys::new(SEED),
            foldhash: foldhash::fast::FixedState::with_seed(SEED),
            xxh3: xxhash_rust::const_xxh3::const_custom_default_secret(SEED),
            rapidhash: rapidhash::v3::RapidSecrets::seed(SEED),
            wyhash: wyhash::final3::make_secret(SEED),
            ahash: ahash::RandomState::with_seeds(SEED, !SEED, SEED.rotate_left(17), SEED.rotate_left(41)),
        }
    }
}

impl Hashes {
    pub fn with<M: Measure>(&self, name: &str, m: &M) -> M::Out {
        match name {
            "hothash" => m.measure(&|d: &[u8]| hothash::hash_with(&self.hothash, d)),
            "foldhash" => m.measure(&|d: &[u8]| {
                let mut h = self.foldhash.build_hasher();
                h.write(d);
                h.finish()
            }),
            "fxhash" => m.measure(&|d: &[u8]| {
                let mut h = rustc_hash::FxHasher::with_seed(SEED as usize);
                h.write(d);
                h.finish()
            }),
            "xxh3" => m.measure(&|d: &[u8]| xxhash_rust::xxh3::xxh3_64_with_secret(d, &self.xxh3)),
            "rapidhash" => m.measure(&|d: &[u8]| rapidhash::v3::rapidhash_v3_seeded(d, &self.rapidhash)),
            "gxhash" => m.measure(&|d: &[u8]| gxhash::gxhash64(d, SEED as i64)),
            "gxhash256" => m.measure(&|d: &[u8]| gxhash_hybrid::gxhash64(d, SEED as i64)),
            "wyhash" => m.measure(&|d: &[u8]| wyhash::final3::wyhash(d, SEED, self.wyhash)),
            "ahash" => m.measure(&|d: &[u8]| {
                let mut h = self.ahash.build_hasher();
                h.write(d);
                h.finish()
            }),
            _ => panic!("unknown hash {name}"),
        }
    }
}

const PHYS: usize = 32 << 10;
pub const SPREAD: usize = 0x3C0;

pub struct Input {
    base: *const u8,
    max_len: usize,
}

impl Input {
    pub fn new(max_len: usize) -> Input {
        let span = (max_len + SPREAD + 1).next_multiple_of(PHYS);
        unsafe {
            let fd = libc::memfd_create(c"hothash-bench".as_ptr(), 0);
            assert!(fd >= 0, "memfd_create: {}", std::io::Error::last_os_error());
            assert_eq!(libc::ftruncate(fd, PHYS as i64), 0);
            let w = libc::mmap(std::ptr::null_mut(), PHYS, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
            assert_ne!(w, libc::MAP_FAILED);
            let mut x = 0x1234_5678_9abc_def1u64;
            for i in 0..PHYS {
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                *w.cast::<u8>().add(i) = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8;
            }
            libc::munmap(w, PHYS);
            let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE;
            let base = libc::mmap(std::ptr::null_mut(), span, libc::PROT_NONE, flags, -1, 0).cast::<u8>();
            assert_ne!(base.cast(), libc::MAP_FAILED);
            for off in (0..span).step_by(PHYS) {
                let flags = libc::MAP_SHARED | libc::MAP_FIXED | libc::MAP_POPULATE;
                let p = libc::mmap(base.add(off).cast(), PHYS, libc::PROT_READ, flags, fd, 0);
                assert_eq!(p, base.add(off).cast(), "mmap: {}", std::io::Error::last_os_error());
            }
            libc::close(fd);
            Input { base, max_len }
        }
    }
}

#[inline(always)]
pub fn opaque(mut x: usize) -> usize {
    unsafe { std::arch::asm!("/* {0} */", inout(reg) x, options(nomem, nostack, preserves_flags)) };
    x
}

#[inline(never)]
pub fn chain<F: Fn(&[u8]) -> u64>(f: &F, input: &Input, len: usize, n: usize) -> u64 {
    assert!(0 < len && len <= input.max_len);
    let mut h = 0u64;
    for _ in 0..n {
        let len = opaque(len);
        h = f(unsafe { std::slice::from_raw_parts(input.base.add(h as usize & SPREAD), len) });
    }
    h
}
