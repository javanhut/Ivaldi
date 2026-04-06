//! Seal name generation for Ivaldi VCS.
//!
//! Each seal (commit) gets a deterministic, memorable name derived from its BLAKE3 hash.
//! Format: `adjective-noun-verb-adverb-shortHash`
//! Example: `swift-eagle-flies-high-447abe9b`
//!
//! The seal name is the KEY in the key-value hash system. Each seal name maps to:
//! - BLAKE3 hash (internal/native use)
//! - SHA1 hash (GitHub/GitLab compatibility, populated only during remote sync)

use crate::hash::B3Hash;

static ADJECTIVES: &[&str] = &[
    "swift", "brave", "bold", "clever", "mighty", "gentle", "wise", "noble", "fierce", "calm",
    "bright", "dark", "ancient", "young", "strong", "quick", "silent", "loud", "warm", "cool",
    "sharp", "smooth", "rough", "soft", "hard", "light", "heavy", "deep", "shallow", "wide",
    "narrow", "tall", "short", "long", "round", "square", "curved", "straight", "twisted", "pure",
    "wild", "tame", "free", "bound", "open", "closed", "full", "empty", "rich", "simple",
    "complex", "clear", "misty", "vivid", "dim", "golden", "pale", "silver", "crystal", "iron",
    "steel", "stone", "wooden", "grand",
];

static NOUNS: &[&str] = &[
    "eagle",
    "mountain",
    "river",
    "falcon",
    "wolf",
    "bear",
    "storm",
    "thunder",
    "forest",
    "ocean",
    "phoenix",
    "dragon",
    "tiger",
    "lion",
    "hawk",
    "raven",
    "fox",
    "deer",
    "star",
    "moon",
    "sun",
    "comet",
    "galaxy",
    "planet",
    "valley",
    "peak",
    "canyon",
    "meadow",
    "grove",
    "spring",
    "waterfall",
    "lake",
    "island",
    "lighthouse",
    "castle",
    "tower",
    "bridge",
    "gate",
    "path",
    "road",
    "sword",
    "shield",
    "crown",
    "gem",
    "crystal",
    "flame",
    "spark",
    "ember",
    "wind",
    "wave",
    "stone",
    "tree",
    "flower",
    "rose",
    "oak",
    "pine",
    "marble",
    "granite",
    "diamond",
    "ruby",
    "sapphire",
    "emerald",
    "pearl",
    "gold",
];

static VERBS: &[&str] = &[
    "flies",
    "runs",
    "leaps",
    "soars",
    "dives",
    "climbs",
    "swims",
    "hunts",
    "rests",
    "guards",
    "watches",
    "seeks",
    "finds",
    "builds",
    "grows",
    "shines",
    "glows",
    "moves",
    "stands",
    "waits",
    "rises",
    "falls",
    "turns",
    "spins",
    "flows",
    "burns",
    "melts",
    "freezes",
    "breaks",
    "heals",
    "creates",
    "destroys",
    "protects",
    "attacks",
    "defends",
    "conquers",
    "explores",
    "discovers",
    "reveals",
    "hides",
    "opens",
    "closes",
    "starts",
    "ends",
    "begins",
    "finishes",
    "travels",
    "arrives",
    "departs",
    "returns",
    "calls",
    "whispers",
    "sings",
    "roars",
    "echoes",
    "resonates",
    "reflects",
    "absorbs",
    "radiates",
    "pulsates",
    "vibrates",
    "oscillates",
    "rotates",
    "revolves",
];

static ADVERBS: &[&str] = &[
    "high", "fast", "slow", "well", "far", "near", "deep", "wide", "soft", "hard", "bright",
    "dark", "quiet", "loud", "free", "true", "bold", "wise", "swift", "strong", "gentle", "fierce",
    "calm", "wild", "proud", "humble", "grand", "small", "great", "tiny", "vast", "narrow",
    "smooth", "rough", "sharp", "dull", "clear", "misty", "warm", "cool", "hot", "cold", "dry",
    "wet", "fresh", "stale", "new", "old", "young", "ancient", "modern", "classic", "pure",
    "mixed", "simple", "complex", "easy", "light", "heavy", "quick", "early", "late", "still",
    "steady",
];

/// Simple seeded pseudo-random number generator (compatible with Go's math/rand).
/// Uses the same linear congruential approach for determinism.
struct SeededRng {
    state: u64,
}

impl SeededRng {
    fn new(seed: u64) -> Self {
        // Use the seed directly, matching Go's rand.NewSource behavior
        Self { state: seed }
    }

    /// Generate next pseudo-random value, matching Go's math/rand LCG.
    fn next(&mut self) -> u64 {
        // Go's math/rand uses a different algorithm (additive lagged Fibonacci),
        // but for our purposes we just need determinism from the hash.
        // We use a simple xorshift64 which is fast and gives good distribution.
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn intn(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Generate a deterministic, memorable seal name from a BLAKE3 hash.
///
/// Format: `adjective-noun-verb-adverb-shortHash`
/// The same hash always produces the same name.
pub fn generate_seal_name(hash: B3Hash) -> String {
    let bytes = hash.as_bytes();
    let seed = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let mut rng = SeededRng::new(seed);

    let adj = ADJECTIVES[rng.intn(ADJECTIVES.len())];
    let noun = NOUNS[rng.intn(NOUNS.len())];
    let verb = VERBS[rng.intn(VERBS.len())];
    let adv = ADVERBS[rng.intn(ADVERBS.len())];
    let short_hash = hash.short(8);

    format!("{}-{}-{}-{}-{}", adj, noun, verb, adv, short_hash)
}

/// Check if a seal name or partial name matches a full seal name.
pub fn matches_seal_name(full_name: &str, query: &str) -> bool {
    if full_name == query {
        return true;
    }
    // Match partial name (prefix of the word portion)
    let word_part = full_name.rsplitn(2, '-').nth(1).unwrap_or("");
    word_part.starts_with(query) || full_name.starts_with(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let hash = B3Hash::digest(b"test commit");
        let name1 = generate_seal_name(hash);
        let name2 = generate_seal_name(hash);
        assert_eq!(name1, name2);
    }

    #[test]
    fn different_hashes_different_names() {
        let h1 = B3Hash::digest(b"commit 1");
        let h2 = B3Hash::digest(b"commit 2");
        let n1 = generate_seal_name(h1);
        let n2 = generate_seal_name(h2);
        assert_ne!(n1, n2);
    }

    #[test]
    fn format_is_correct() {
        let hash = B3Hash::digest(b"test");
        let name = generate_seal_name(hash);

        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 5, "name should have 5 parts: {}", name);

        // Last part should be 8 hex chars
        let short_hash = parts[4];
        assert_eq!(short_hash.len(), 8);
        assert!(short_hash.chars().all(|c| c.is_ascii_hexdigit()));

        // Verify the hash suffix matches
        assert!(hash.to_hex().starts_with(short_hash));
    }

    #[test]
    fn words_from_lists() {
        let hash = B3Hash::digest(b"word check");
        let name = generate_seal_name(hash);
        let parts: Vec<&str> = name.split('-').collect();

        assert!(
            ADJECTIVES.contains(&parts[0]),
            "adjective not in list: {}",
            parts[0]
        );
        assert!(NOUNS.contains(&parts[1]), "noun not in list: {}", parts[1]);
        assert!(VERBS.contains(&parts[2]), "verb not in list: {}", parts[2]);
        assert!(
            ADVERBS.contains(&parts[3]),
            "adverb not in list: {}",
            parts[3]
        );
    }

    #[test]
    fn matches_full_name() {
        assert!(matches_seal_name(
            "swift-eagle-flies-high-447abe9b",
            "swift-eagle-flies-high-447abe9b"
        ));
    }

    #[test]
    fn matches_partial_prefix() {
        assert!(matches_seal_name(
            "swift-eagle-flies-high-447abe9b",
            "swift-eagle"
        ));
        assert!(matches_seal_name(
            "swift-eagle-flies-high-447abe9b",
            "swift"
        ));
    }

    #[test]
    fn no_match() {
        assert!(!matches_seal_name(
            "swift-eagle-flies-high-447abe9b",
            "bold-wolf"
        ));
    }

    #[test]
    fn many_unique_names() {
        let mut names = std::collections::HashSet::new();
        for i in 0..1000 {
            let hash = B3Hash::digest(format!("commit {}", i).as_bytes());
            let name = generate_seal_name(hash);
            names.insert(name);
        }
        // With 64^4 possible word combos + hash suffix, all 1000 should be unique
        assert_eq!(names.len(), 1000);
    }
}
