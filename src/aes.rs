use crate::opaque;
use core::arch::x86_64::*;

#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub(crate) struct Keys {
    first_key: [u64; 8],
}

const STATE_KEYS: [[u64; 8]; 8] = [
    [0x7e2a1c9fd3b84e61, 0x2c95f04a8d17b3e2, 0xb3d9475e0c6a2f18, 0x91f6c8a35e7d0b24, 0x48ec13b7a90f65d3, 0xe5207f9cb6d84a1e, 0x1fa7d26e4c3b9580, 0x6c03b9e8f1a7542d],
    [0xd84f2a6c1e93b705, 0x37b1e05f92c6da48, 0xa6c38d71f4b5290e, 0x0e9b54fa63d18c27, 0xf2186dcb3a4e70b9, 0x5b7ac9e1068f2d34, 0xc41e7b3092d5af68, 0x29d6f8854bc31e7a],
    [0x8a3f61d72cb9e540, 0x4de2a90b7f6c1835, 0x13c8f5e26ad04b97, 0xbf5d1c4893e27a06, 0x6fa4e3270b8dc951, 0xd2079ebc45f13a68, 0x7c61b8053ae4d92f, 0x05bc4fd9e1726a83],
    [0xe91c37a54f80d26b, 0x3a6fd2e8b1c59407, 0xc7b209f4d36e185a, 0x548de67a2f1bc093, 0x1be5a4c99d6237f0, 0xa04f7b1ec8d95326, 0x6e98c35d07a4fb12, 0xf32d06b8e5c17a49],
    [0x2fd8b46ce7051a93, 0x96a1e3f05b2d7c48, 0x4c370f9ad8e6b215, 0xdb64a2153f9ce087, 0x71fe5c8b406d39a2, 0x08b39d6e2ca7f5c1, 0xb5c2f8a7613e04d9, 0x5e4a71d3f908bc26],
    [0xc63e2b819a7fd450, 0x1d97c5ea30b6482f, 0x8b05f7364ed1ac93, 0x45e8bd0279c3f61a, 0xf9a1640cd52e87b3, 0x3276d9fb18a4e05c, 0xa8db037e6c915f24, 0x6f1c49a5b3e2d708],
    [0x13f9e6a4c25d0b87, 0xe47c2b913fa865d0, 0x5ab3d87061ce4f29, 0x9d2601fbe84a73c5, 0x2681ac5f7d39e0b4, 0xc05f93e2b67d1a48, 0x7be41d0c95a2f863, 0xb8a067d34f1c2e95],
    [0x4a17d8e63bc90f52, 0xf3b0629dc4e51a87, 0x86e94c17a02f5d3b, 0x1d5cb3f8e97064a2, 0xd72e05a16b8cf349, 0x69f8a42c13d7be05, 0xa25b7e90f6c1384d, 0x3ec416bd5a092fe7],
];

const FINISH_KEY: [u64; 8] = [0x5f3a9c71e2d4086b, 0xa18e6b2d47f9c350, 0x2c7d04f8b9a6e31d, 0xe6b1539a0d2c784f, 0x93d0e74a5bf1268c, 0x084fa2c6d73e95b1, 0xc95b1e3f06a8d274, 0x76e8cd0b9124af3e];

const FINAL_KEY: [u64; 2] = [0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1];

const SEED_KEYS: [[u64; 2]; 5] = [
    [0x243f6a8885a308d3, 0x13198a2e03707344],
    [0xa4093822299f31d0, 0x082efa98ec4e6c89],
    [0x452821e638d01377, 0xbe5466cf34e90c6c],
    [0xc0ac29b7c97c50dd, 0x3f84d5b5b5470917],
    [0x9216d5d98979fb1b, 0xd1310ba698dfb5ac],
];

const LEN_MUL: u64 = 0x9e37_79b9_7f4a_7c15;

#[inline(always)]
unsafe fn key128(key: &[u64; 2]) -> __m128i {
    unsafe { _mm_loadu_si128(key.as_ptr().cast()) }
}

#[inline(always)]
unsafe fn key512(key: &[u64; 8]) -> __m512i {
    unsafe { _mm512_loadu_si512(key.as_ptr().cast()) }
}

#[inline(always)]
unsafe fn load512(ptr: *const u8) -> __m512i {
    unsafe { _mm512_loadu_si512(ptr.cast()) }
}

#[inline(always)]
unsafe fn aesenc512(state: __m512i, key: __m512i) -> __m512i {
    unsafe { _mm512_aesenc_epi128(state, key) }
}

impl Keys {
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,vaes,aes")]
    pub(crate) unsafe fn new(seed: u64) -> (Keys, u64, u64) {
        unsafe {
            let seed_block = _mm_set_epi64x(!seed as i64, seed as i64);
            let first_key = _mm_aesenc_si128(_mm_aesenc_si128(_mm_xor_si128(seed_block, key128(&SEED_KEYS[0])), key128(&SEED_KEYS[1])), key128(&SEED_KEYS[2]));
            let mul_seed = _mm_aesenc_si128(_mm_aesenc_si128(first_key, key128(&SEED_KEYS[3])), key128(&SEED_KEYS[4]));
            let mut keys = Keys { first_key: [0; 8] };
            _mm512_storeu_si512(keys.first_key.as_mut_ptr().cast(), _mm512_broadcast_i32x4(first_key));
            (keys, _mm_cvtsi128_si64(mul_seed) as u64, _mm_extract_epi64::<1>(mul_seed) as u64)
        }
    }
}

#[inline(always)]
unsafe fn finish(state: __m128i) -> u64 {
    unsafe {
        let state = _mm_aesenc_si128(state, key128(&FINAL_KEY));
        let hi: u64;
        core::arch::asm!("vpextrq {hi}, {state}, 1", hi = out(reg) hi, state = in(xmm_reg) state, options(pure, nomem, nostack, preserves_flags));
        _mm_cvtsi128_si64(state) as u64 ^ hi
    }
}

#[inline(always)]
unsafe fn fold(state: __m512i) -> __m128i {
    unsafe {
        let lane01 = _mm_xor_si128(_mm512_castsi512_si128(state), _mm512_extracti32x4_epi32::<1>(state));
        let lane2 = _mm512_extracti32x4_epi32::<2>(state);
        let lane3 = _mm512_extracti32x4_epi32::<3>(state);
        let folded: __m128i;
        core::arch::asm!("vpternlogq {acc}, {lane2}, {lane3}, 0x96", acc = inout(xmm_reg) lane01 => folded, lane2 = in(xmm_reg) lane2, lane3 = in(xmm_reg) lane3, options(pure, nomem, nostack, preserves_flags));
        folded
    }
}

#[inline(always)]
unsafe fn xor3(a: __m512i, b: __m512i, c: __m512i) -> __m512i {
    unsafe { _mm512_ternarylogic_epi64::<0x96>(a, b, c) }
}

#[target_feature(enable = "avx512f,avx512bw,avx512vl,vaes,aes")]
#[inline]
unsafe fn leaves<const N: usize>(keys: &Keys, ptr: *const u8, len: usize, tweak: u64) -> u64 {
    unsafe {
        let len_key = _mm512_zextsi128_si512(_mm_cvtsi64_si128(((len as u64).wrapping_mul(LEN_MUL) ^ tweak) as i64));
        let first_key = key512(&keys.first_key);
        let tail = (len - 64 * (N - 4)) as u64;
        let mut states = [_mm512_setzero_si512(); N];
        for (i, state) in states.iter_mut().enumerate() {
            let offset = if i < 4 { 64 * i } else { opaque(tail + 64 * (i - 4) as u64) as usize };
            *state = aesenc512(load512(ptr.add(offset)), first_key);
        }
        states[0] = aesenc512(states[0], _mm512_xor_si512(key512(&STATE_KEYS[0]), len_key));
        for (i, state) in states.iter_mut().enumerate().skip(1) {
            *state = aesenc512(*state, key512(&STATE_KEYS[i]));
        }
        for state in states.iter_mut() {
            *state = aesenc512(*state, key512(&FINISH_KEY));
        }
        let (xor012, xor345) = (xor3(states[0], states[1], states[2]), xor3(states[3], states[4], states[5]));
        let all = if N == 7 { xor3(xor012, xor345, states[6]) } else { _mm512_xor_si512(xor012, xor345) };
        finish(fold(all))
    }
}

#[target_feature(enable = "avx512f,avx512bw,avx512vl,vaes,aes")]
#[inline]
pub(crate) unsafe fn hash(keys: &Keys, data: &[u8], tweak: u64) -> u64 {
    let len = data.len();
    debug_assert!(len > 256);
    let ptr = data.as_ptr();
    if len <= 384 {
        return unsafe { leaves::<6>(keys, ptr, len, tweak) };
    }
    if len <= 448 {
        return unsafe { leaves::<7>(keys, ptr, len, tweak) };
    }
    unsafe {
        let len_key = _mm512_zextsi128_si512(_mm_cvtsi64_si128(((len as u64).wrapping_mul(LEN_MUL) ^ tweak) as i64));
        let first_key = key512(&keys.first_key);
        let tail = if len <= 512 { len - 256 } else { 256 };
        let (tail0, tail1, tail2, tail3) = (
            opaque(tail as u64) as usize,
            opaque(tail as u64 + 64) as usize,
            opaque(tail as u64 + 128) as usize,
            opaque(tail as u64 + 192) as usize,
        );
        let mut states = [
            aesenc512(load512(ptr), first_key),
            aesenc512(load512(ptr.add(64)), first_key),
            aesenc512(load512(ptr.add(128)), first_key),
            aesenc512(load512(ptr.add(192)), first_key),
            aesenc512(load512(ptr.add(tail0)), first_key),
            aesenc512(load512(ptr.add(tail1)), first_key),
            aesenc512(load512(ptr.add(tail2)), first_key),
            aesenc512(load512(ptr.add(tail3)), first_key),
        ];
        let mut rounds_owed = 2;
        let tail_blocks = if len > 512 { ((len - 513) % 512 + 1).div_ceil(64) } else { 0 };
        if tail_blocks >= 7 {
            let mut next = ptr.add(512);
            let last_group = ptr.add(len - 512);
            loop {
                let group = if next < last_group { next } else { last_group };
                for (i, state) in states.iter_mut().enumerate() {
                    *state = aesenc512(*state, load512(group.add(64 * i)));
                }
                if group == last_group {
                    break;
                }
                next = next.add(512);
            }
            rounds_owed = 3;
        } else if tail_blocks > 0 {
            let last_group = ptr.add(len - 512);
            let mut group = ptr.add(512);
            while group < last_group {
                for (i, state) in states.iter_mut().enumerate() {
                    *state = aesenc512(*state, load512(group.add(64 * i)));
                }
                group = group.add(512);
            }
            let tail = ptr.add(len - 64 * tail_blocks);
            macro_rules! absorb {
                ($($i:literal)*) => {{ $( states[$i] = aesenc512(states[$i], load512(tail.add(64 * $i))); )* }};
            }
            match tail_blocks {
                1 => absorb!(0),
                2 => absorb!(0 1),
                3 => absorb!(0 1 2),
                4 => absorb!(0 1 2 3),
                5 => absorb!(0 1 2 3 4),
                _ => absorb!(0 1 2 3 4 5),
            }
            rounds_owed = 3;
        }
        states[0] = aesenc512(states[0], _mm512_xor_si512(key512(&STATE_KEYS[0]), len_key));
        for (i, state) in states.iter_mut().enumerate().skip(1) {
            *state = aesenc512(*state, key512(&STATE_KEYS[i]));
        }
        for _ in 1..rounds_owed {
            for state in states.iter_mut() {
                *state = aesenc512(*state, key512(&FINISH_KEY));
            }
        }
        let all = xor3(xor3(states[0], states[1], states[2]), xor3(states[3], states[4], states[5]), _mm512_xor_si512(states[6], states[7]));
        finish(fold(all))
    }
}
