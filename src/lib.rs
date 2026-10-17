#![no_std]
#![allow(unused_unsafe)]

#[cfg(feature = "std")]
extern crate std;

use core::borrow::Borrow;
use core::hash::{BuildHasher, Hash};

#[cfg(not(all(
    target_arch = "x86_64",
    target_feature = "avx512f",
    target_feature = "avx512bw",
    target_feature = "avx512vl",
    target_feature = "vaes",
    target_feature = "aes",
    target_feature = "bmi2"
)))]
compile_error!(
    "hothash needs AVX-512 F/BW/VL, VAES, AES and BMI2 at compile time: build with \
     RUSTFLAGS=\"-C target-cpu=native\" on Zen 4/5 or Ice Lake and newer"
);

mod aes;
mod mul;

#[derive(Clone, Copy)]
pub struct Keys {
    aes: aes::Keys,
    mul: mul::Keys,
}

impl Keys {
    pub fn new(seed: u64) -> Keys {
        let (aes, mul_seed_even, mul_seed_odd) = unsafe { aes::Keys::new(seed) };
        Keys { aes, mul: mul::Keys::new(mul_seed_even, mul_seed_odd) }
    }
}

#[cfg(feature = "std")]
impl Default for Keys {
    fn default() -> Keys {
        Keys::new(std::hash::RandomState::new().hash_one(()))
    }
}

impl BuildHasher for Keys {
    type Hasher = Hasher;

    #[inline]
    fn build_hasher(&self) -> Hasher {
        Hasher { keys: *self, state: State::new() }
    }

    #[inline(always)]
    fn hash_one<T: Hash>(&self, x: T) -> u64 {
        let mut hasher = Hasher { keys: self, state: State::new() };
        x.hash(&mut hasher);
        core::hash::Hasher::finish(&hasher)
    }
}

#[cfg(feature = "std")]
pub type HashMap<K, V> = std::collections::HashMap<K, V, Keys>;

#[cfg(feature = "std")]
pub type HashSet<T> = std::collections::HashSet<T, Keys>;

#[derive(Clone, Copy)]
pub struct Hasher<K = Keys> {
    keys: K,
    state: State,
}

impl<K: Borrow<Keys>> core::hash::Hasher for Hasher<K> {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        self.state.bytes(self.keys.borrow(), bytes);
    }

    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.state.int(self.keys.borrow(), i as u64, 1);
    }

    #[inline(always)]
    fn write_u16(&mut self, i: u16) {
        self.state.int(self.keys.borrow(), i as u64, 2);
    }

    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.state.int(self.keys.borrow(), i as u64, 4);
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.state.int(self.keys.borrow(), i, 8);
    }

    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.state.int(self.keys.borrow(), i as u64, 8);
    }

    #[inline(always)]
    fn write_u128(&mut self, i: u128) {
        self.state.int128(self.keys.borrow(), i);
    }

    #[inline(always)]
    fn finish(&self) -> u64 {
        self.state.finish(self.keys.borrow())
    }
}

#[derive(Clone, Copy)]
struct State {
    acc: u64,
    words: [u64; 4],
    count: usize,
    fill: usize,
    len: usize,
    absorbed: bool,
    after_bytes: bool,
}

impl State {
    #[inline(always)]
    fn new() -> State {
        State { acc: 0, words: [0; 4], count: 0, fill: 0, len: 0, absorbed: false, after_bytes: false }
    }

    #[inline(always)]
    fn clear(&mut self) {
        self.words = [0; 4];
        self.count = 0;
        self.fill = 0;
        self.len = 0;
    }

    #[inline(always)]
    fn terminator(&self) -> bool {
        self.len == 1 && self.after_bytes && self.words[0] == 0xff
    }

    #[inline(always)]
    fn flush(&mut self, keys: &Keys) {
        self.acc = absorb(keys, self.acc, self.len << 3 | self.count, self.words[0], self.words[1], self.words[2], self.words[3]);
        self.clear();
        self.absorbed = true;
        self.after_bytes = false;
    }

    #[inline(always)]
    fn int(&mut self, keys: &Keys, value: u64, size: usize) {
        if self.count > 0 && self.fill + size <= 8 {
            self.words[0] |= value << (8 * self.fill);
            self.fill += size;
        } else {
            if self.count == 4 {
                self.flush(keys);
            }
            self.words = [value, self.words[0], self.words[1], self.words[2]];
            self.count += 1;
            self.fill = size;
        }
        self.len += size;
    }

    #[inline(always)]
    fn int128(&mut self, keys: &Keys, value: u128) {
        self.int(keys, value as u64, 8);
        self.int(keys, (value >> 64) as u64, 8);
    }

    #[inline(always)]
    fn bytes(&mut self, keys: &Keys, data: &[u8]) {
        if self.count > 0 {
            if (self.count == 1 && self.len == 8 && self.words[0] == data.len() as u64) || self.terminator() {
                self.clear();
            } else {
                self.flush(keys);
            }
        }
        self.acc = if data.len() <= 16 { mul::hash_short(&keys.mul, data, self.acc) } else { hash_long(keys, data, self.acc) };
        self.absorbed = true;
        self.after_bytes = true;
    }

    #[inline(always)]
    fn finish(&self, keys: &Keys) -> u64 {
        if self.absorbed && (self.count == 0 || self.terminator()) {
            return self.acc;
        }
        mul::hash_ints(&keys.mul, self.words, self.count, self.len, self.acc)
    }
}

#[inline(never)]
fn absorb(keys: &Keys, acc: u64, shape: usize, w0: u64, w1: u64, w2: u64, w3: u64) -> u64 {
    mul::hash_ints(&keys.mul, [w0, w1, w2, w3], shape & 7, shape >> 3, acc)
}

#[inline(always)]
fn hash_tweaked(keys: &Keys, data: &[u8], tweak: u64) -> u64 {
    if data.len() <= 352 {
        mul::hash(&keys.mul, data, tweak)
    } else {
        unsafe { aes::hash(&keys.aes, data, tweak) }
    }
}

#[inline(never)]
fn hash_long(keys: &Keys, data: &[u8], tweak: u64) -> u64 {
    hash_tweaked(keys, data, tweak)
}

#[inline]
pub fn hash_with(keys: &Keys, data: &[u8]) -> u64 {
    hash_tweaked(keys, data, 0)
}

#[inline]
pub fn hash(data: &[u8], seed: u64) -> u64 {
    hash_with(&Keys::new(seed), data)
}

#[inline(always)]
pub(crate) fn opaque(x: u64) -> u64 {
    let out: u64;
    unsafe { core::arch::asm!("mov {out}, {x}", out = lateout(reg) out, x = in(reg) x, options(pure, nomem, nostack, preserves_flags)) };
    out
}
