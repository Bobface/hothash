use crate::opaque;

#[derive(Clone, Copy)]
pub(crate) struct Keys {
    words: [u64; 24],
    empty: u64,
}

const KEY_BASE: [u64; 24] = [
    0x2d358dccaa6c78a5, 0x8bb84b93962eacc9, 0x4b33a62ed433d4a3, 0x4d5a2da51de1aa47,
    0xa0761d6478bd642f, 0xe7037ed1a0b428db, 0x8ebc6af09c88c6e3, 0x589965cc75374cc3,
    0x1d8e4e27c47d124f, 0xd6e8feb86659fd93, 0x9e3779b97f4a7c15, 0xbf58476d1ce4e5b9,
    0x94d049bb133111eb, 0xc2b2ae3d27d4eb4f, 0x165667b19e3779f9, 0x27d4eb2f165667c5,
    0x85ebca77c2b2ae63, 0xff51afd7ed558ccd, 0xc4ceb9fe1a85ec53, 0x87c37b91114253d5,
    0x4cf5ad432745937f, 0x52dce729da3ed4c7, 0x38495ab5f6b0cd2d, 0xaf251af3b0f025b5,
];
const LEN_MUL: u64 = 0xd6e8_feb8_6659_fd93;
const FIN_KEY: usize = 23;

impl Keys {
    pub(crate) fn new(seed_even: u64, seed_odd: u64) -> Keys {
        let mut words = [0u64; 24];
        for (i, word) in words.iter_mut().enumerate() {
            *word = KEY_BASE[i] ^ if i % 2 == 0 { seed_even.rotate_left(i as u32) } else { seed_odd.rotate_left(i as u32) };
        }
        let (lo, hi) = wide_mul(words[0], words[1]);
        Keys { words, empty: lo ^ hi }
    }
}

#[inline(always)]
unsafe fn load8(ptr: *const u8, offset: u64) -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!("mov {value}, qword ptr [{ptr} + {offset}]", value = lateout(reg) value, ptr = in(reg) ptr, offset = in(reg) offset, options(pure, readonly, nostack, preserves_flags))
    };
    value
}

#[inline(always)]
unsafe fn load4(ptr: *const u8, offset: u64) -> u64 {
    let value: u32;
    unsafe {
        core::arch::asm!("mov {value:e}, dword ptr [{ptr} + {offset}]", value = lateout(reg) value, ptr = in(reg) ptr, offset = in(reg) offset, options(pure, readonly, nostack, preserves_flags))
    };
    value as u64
}

#[inline(always)]
unsafe fn load2(ptr: *const u8, offset: u64) -> u64 {
    let value: u32;
    unsafe {
        core::arch::asm!("movzx {value:e}, word ptr [{ptr} + {offset}]", value = lateout(reg) value, ptr = in(reg) ptr, offset = in(reg) offset, options(pure, readonly, nostack, preserves_flags))
    };
    value as u64
}

#[inline(always)]
unsafe fn load1(ptr: *const u8, offset: u64) -> u64 {
    let value: u32;
    unsafe {
        core::arch::asm!("movzx {value:e}, byte ptr [{ptr} + {offset}]", value = lateout(reg) value, ptr = in(reg) ptr, offset = in(reg) offset, options(pure, readonly, nostack, preserves_flags))
    };
    value as u64
}

#[inline(always)]
fn wide_mul(a: u64, b: u64) -> (u64, u64) {
    let product = a as u128 * b as u128;
    (product as u64, (product >> 64) as u64)
}

#[inline(always)]
unsafe fn lane0(head: *const u8, tail: *const u8, key: *const u64, key_len: u64) -> (u64, u64) {
    let (lo, hi): (u64, u64);
    unsafe {
        core::arch::asm!(
            "mov {tmp}, qword ptr [{head}]",
            "xor {tmp}, qword ptr [{key}]",
            "mov rdx, qword ptr [{head} + 8]",
            "xor rdx, {key_len}",
            "mulx {hi}, {lo}, {tmp}",
            "mov {tmp}, qword ptr [{tail}]",
            "xor {tmp}, {lo}",
            "mov rdx, qword ptr [{tail} + 8]",
            "xor rdx, {hi}",
            "mulx {hi}, {lo}, {tmp}",
            head = in(reg) head, tail = in(reg) tail, key = in(reg) key, key_len = in(reg) key_len,
            tmp = out(reg) _, lo = out(reg) lo, hi = out(reg) hi, out("rdx") _,
            options(pure, readonly, nostack),
        )
    };
    (lo, hi)
}

#[inline(always)]
unsafe fn lane<const OFF: usize, const KEY_OFF: usize>(head: *const u8, tail: *const u8, key: *const u64, lo: &mut u64, hi: &mut u64) {
    unsafe {
        core::arch::asm!(
            "mov {tmp}, qword ptr [{head} + {off}]",
            "xor {tmp}, qword ptr [{key} + {key_off}]",
            "mov rdx, qword ptr [{head} + {off} + 8]",
            "xor rdx, qword ptr [{key} + {key_off} + 8]",
            "mulx {prod_hi}, {prod_lo}, {tmp}",
            "mov {tmp}, qword ptr [{tail} + {off}]",
            "xor {tmp}, {prod_lo}",
            "mov rdx, qword ptr [{tail} + {off} + 8]",
            "xor rdx, {prod_hi}",
            "mulx {prod_hi}, {prod_lo}, {tmp}",
            "xor {lo}, {prod_lo}",
            "xor {hi}, {prod_hi}",
            head = in(reg) head, tail = in(reg) tail, key = in(reg) key, off = const OFF, key_off = const KEY_OFF,
            tmp = out(reg) _, prod_lo = out(reg) _, prod_hi = out(reg) _, out("rdx") _,
            lo = inout(reg) *lo, hi = inout(reg) *hi,
            options(pure, readonly, nostack),
        )
    };
}

#[inline(always)]
unsafe fn level<const N: usize>(keys: &Keys, ptr: *const u8, len: usize, len_key: u64) -> (u64, u64) {
    let tail = len as u64 - 8 * N as u64;
    let (mut lo, mut hi) = (0u64, 0u64);
    for i in 0..N {
        let key_odd = if i == 0 { opaque(keys.words[2 * i + 1] ^ len_key) } else { keys.words[2 * i + 1] };
        let (prod_lo, prod_hi) = wide_mul(
            unsafe { load8(ptr, 8 * i as u64) } ^ keys.words[2 * i],
            unsafe { load8(ptr, opaque(tail + 8 * i as u64)) } ^ key_odd,
        );
        lo ^= prod_lo;
        hi ^= prod_hi;
    }
    
    (opaque(lo), opaque(hi))
}

macro_rules! lanes {
    ($keys:expr, $ptr:expr, $len:expr, $len_key:expr, $lanes:literal; $($i:literal)*) => {{
        let key = $keys.words.as_ptr();
        let tail = $ptr.add($len - 16 * $lanes);
        let (mut lo, mut hi) = lane0($ptr, tail, key, opaque($keys.words[1] ^ $len_key));
        $( lane::<{ 16 * $i }, { 16 * $i }>($ptr, tail, key, &mut lo, &mut hi); )*
        (lo, hi)
    }};
}

#[inline(always)]
unsafe fn lane_last<const OFF: usize, const KEY_OFF: usize>(head: *const u8, tail: *const u8, key: *const u64, lo: u64, hi: u64) -> u64 {
    let hash: u64;
    unsafe {
        core::arch::asm!(
            "mov {tmp}, qword ptr [{head} + {off}]",
            "xor {tmp}, qword ptr [{key} + {key_off}]",
            "mov rdx, qword ptr [{head} + {off} + 8]",
            "xor rdx, qword ptr [{key} + {key_off} + 8]",
            "mulx {prod_hi}, {prod_lo}, {tmp}",
            "mov {tmp}, qword ptr [{tail} + {off}]",
            "xor {tmp}, {prod_lo}",
            "xor {tmp}, {lo}",
            "mov rdx, qword ptr [{tail} + {off} + 8]",
            "xor rdx, {prod_hi}",
            "xor rdx, {hi}",
            "mulx {prod_hi}, {prod_lo}, {tmp}",
            "xor {prod_lo}, {prod_hi}",
            head = in(reg) head, tail = in(reg) tail, key = in(reg) key, off = const OFF, key_off = const KEY_OFF,
            lo = in(reg) lo, hi = in(reg) hi,
            tmp = out(reg) _, prod_lo = out(reg) hash, prod_hi = out(reg) _, out("rdx") _,
            options(pure, readonly, nostack),
        )
    };
    hash
}

macro_rules! lanes_fin {
    ($keys:expr, $ptr:expr, $len:expr, $len_key:expr, $lanes:literal; $($i:literal)*; $last:literal) => {{
        let key = $keys.words.as_ptr();
        let tail = $ptr.add($len - 16 * $lanes);
        let (mut lo, mut hi) = lane0($ptr, tail, key, opaque($keys.words[1] ^ $len_key));
        $( lane::<{ 16 * $i }, { 16 * $i }>($ptr, tail, key, &mut lo, &mut hi); )*
        lane_last::<{ 16 * $last }, { 16 * $last }>($ptr, tail, key, lo, hi)
    }};
}

#[inline(always)]
fn fin(keys: &Keys, lo: u64, hi: u64) -> u64 {
    let (prod_lo, prod_hi) = wide_mul(lo ^ keys.words[FIN_KEY], hi);
    prod_lo ^ prod_hi
}

#[inline(always)]
pub(crate) fn fold_in(keys: &Keys, word: u64, value: u64) -> u64 {
    word.wrapping_mul(keys.words[FIN_KEY] | 1) ^ value
}

#[inline(always)]
pub(crate) fn hash_ints(keys: &Keys, words: &[u64; 8], count: usize, len: usize, tweak: u64) -> u64 {
    let len_key = (len as u64).wrapping_mul(LEN_MUL);
    let (first, last) = match count {
        0 if tweak == 0 => return keys.empty,
        0 => (0, 0),
        1 if len <= 3 => (words[0], words[0].rotate_right(22)),
        1 => (words[0], words[0]),
        2 => (words[0], words[1]),
        3 | 4 => {
            let (lo0, hi0) = wide_mul(words[0] ^ keys.words[0], words[(count - 2) & 7] ^ opaque(keys.words[1] ^ len_key) ^ tweak);
            let (lo1, hi1) = wide_mul(words[1] ^ keys.words[2], words[(count - 1) & 7] ^ keys.words[3]);
            return opaque(lo0 ^ lo1) ^ opaque(hi0 ^ hi1);
        }
        _ => {
            let tail = count - 4;
            let (lo0, hi0) = wide_mul(words[0] ^ keys.words[0], words[1] ^ opaque(keys.words[1] ^ len_key) ^ tweak);
            let (lo0, hi0) = wide_mul(words[tail & 7] ^ lo0, words[(tail + 1) & 7] ^ hi0);
            let (lo1, hi1) = wide_mul(words[2] ^ keys.words[2], words[3] ^ keys.words[3]);
            let (lo1, hi1) = wide_mul(words[(tail + 2) & 7] ^ lo1, words[(tail + 3) & 7] ^ hi1);
            return opaque(lo0 ^ lo1) ^ opaque(hi0 ^ hi1);
        }
    };
    let (prod_lo, prod_hi) = wide_mul(first ^ keys.words[0], last ^ opaque(keys.words[1] ^ len_key) ^ tweak);
    prod_lo ^ prod_hi
}

#[inline(always)]
unsafe fn short(keys: &Keys, ptr: *const u8, len: usize, tweak: u64) -> u64 {
    let (first, last) = unsafe {
        if len >= 8 {
            (load8(ptr, 0), load8(ptr, opaque(len as u64 - 8)))
        } else if len >= 4 {
            (load4(ptr, 0), load4(ptr, opaque(len as u64 - 4)))
        } else if len >= 2 {
            (load2(ptr, 0), load2(ptr, opaque(len as u64 - 2)).rotate_right(22))
        } else if len == 1 {
            let byte = load1(ptr, 0);
            (byte, byte.rotate_right(22))
        } else if tweak == 0 {
            return keys.empty;
        } else {
            (0, 0)
        }
    };
    let (prod_lo, prod_hi) = wide_mul(first ^ keys.words[0], last ^ opaque(keys.words[1] ^ (len as u64).wrapping_mul(LEN_MUL)) ^ tweak);
    prod_lo ^ prod_hi
}

#[inline(always)]
pub(crate) fn hash_short(keys: &Keys, data: &[u8], tweak: u64) -> u64 {
    let len = data.len();
    debug_assert!(len <= 16);
    unsafe { short(keys, data.as_ptr(), len, tweak) }
}

#[inline(always)]
pub(crate) fn hash(keys: &Keys, data: &[u8], tweak: u64) -> u64 {
    let len = data.len();
    debug_assert!(len <= 352);
    let ptr = data.as_ptr();
    let len_key = (len as u64).wrapping_mul(LEN_MUL) ^ tweak;
    unsafe {
        if len <= 16 {
            return short(keys, ptr, len, tweak);
        }
        if len <= 32 {
            let (lo, hi) = level::<2>(keys, ptr, len, len_key);
            return lo ^ hi;
        }
        if len <= 64 {
            let (lo, hi) = lanes!(keys, ptr, len, len_key, 2; 1);
            return lo ^ hi;
        }
        if len <= 128 {
            let (lo, hi) = match (len - 1) >> 4 {
                4 => level::<5>(keys, ptr, len, len_key),
                5 => level::<6>(keys, ptr, len, len_key),
                6 => level::<7>(keys, ptr, len, len_key),
                _ => level::<8>(keys, ptr, len, len_key),
            };
            return fin(keys, lo, hi);
        }
        match (len - 1) >> 5 {
            4 => lanes_fin!(keys, ptr, len, len_key, 5; 1 2 3; 4),
            5 => lanes_fin!(keys, ptr, len, len_key, 6; 1 2 3 4; 5),
            6 => lanes_fin!(keys, ptr, len, len_key, 7; 1 2 3 4 5; 6),
            7 => lanes_fin!(keys, ptr, len, len_key, 8; 1 2 3 4 5 6; 7),
            8 => lanes_fin!(keys, ptr, len, len_key, 9; 1 2 3 4 5 6 7; 8),
            9 => lanes_fin!(keys, ptr, len, len_key, 10; 1 2 3 4 5 6 7 8; 9),
            _ => lanes_fin!(keys, ptr, len, len_key, 11; 1 2 3 4 5 6 7 8 9; 10),
        }
    }
}
