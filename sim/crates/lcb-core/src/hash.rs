//! State hashing for replays, transposition tables and clone verification.
//!
//! The hash is computed over the *canonical JSON encoding* of a value, so it
//! necessarily covers everything that is serialised - RNG state, skill deck,
//! dashboard, every unit's HP/SP/statuses, cooldowns and pending actions.  A
//! state that differs in any field that can affect the future produces a
//! different hash.

use serde::Serialize;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Hash any serialisable value through its canonical JSON encoding.
pub fn state_hash<T: Serialize>(value: &T) -> u64 {
    match serde_json::to_vec(value) {
        Ok(bytes) => fnv1a64(&bytes),
        Err(_) => 0,
    }
}

/// Search / transposition key.
///
/// Identical to the replay hash except that it ignores the things that cannot
/// affect the future: the battle log, the warnings and the per-turn statistics.
/// The RNG continuation is part of the hash (every field of `Rng` is
/// serialised), which is what the plan requires of a transposition key.
pub fn search_key(state: &crate::state::BattleState) -> u64 {
    let mut probe = state.clone();
    probe.log.clear();
    probe.warnings.clear();
    probe.turn_stats = Default::default();
    state_hash(&probe)
}

pub fn hex(hash: u64) -> String {
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        assert_eq!(fnv1a64(b"abc"), fnv1a64(b"abc"));
        assert_ne!(fnv1a64(b"abc"), fnv1a64(b"abd"));
    }

    #[test]
    fn hash_detects_field_changes() {
        #[derive(Serialize)]
        struct S {
            a: i32,
            b: Vec<i32>,
        }
        let x = S { a: 1, b: vec![1, 2, 3] };
        let y = S { a: 1, b: vec![1, 2, 4] };
        assert_ne!(state_hash(&x), state_hash(&y));
    }
}
