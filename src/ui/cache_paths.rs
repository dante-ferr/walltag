//! XDG cache location and a process-stable hash for cache file names.
//!
//! `std::collections::hash_map::DefaultHasher` is not stable across runs, so cache
//! filenames use this FNV-1a 128-bit mix instead.

use std::path::PathBuf;

use directories::ProjectDirs;

/// Walltag's XDG cache directory (`~/.cache/walltag` on Linux). `None` when the
/// home directory cannot be resolved.
pub fn cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("com", "walltag", "walltag").map(|dirs| dirs.cache_dir().to_path_buf())
}

/// Mixes `parts` into a 128-bit value that does not change between process runs.
pub fn stable_hash(parts: &[&[u8]]) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;
    let mut hash = OFFSET;
    for part in parts {
        for byte in *part {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        // Separate parts so ("ab", "c") and ("a", "bc") do not collide.
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_separates_parts() {
        let first = stable_hash(&[b"wall", b"tag"]);
        let second = stable_hash(&[b"wall", b"tag"]);
        assert_eq!(first, second);
        assert_ne!(first, stable_hash(&[b"walltag"]));
        assert_ne!(first, stable_hash(&[b"wall", b"tags"]));
    }
}
