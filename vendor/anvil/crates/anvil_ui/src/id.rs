//! Element ids: what keys a widget's state (animation, focus, scroll, text editing) across frames.
//!
//! An [`ElementId`] is a 64-bit hash of a name and any keys added to it, so ids can come from
//! data at runtime without leaking memory, and two ids with the same parts are always equal.
//! Every widget takes `impl Into<ElementId>`, so a string literal still works as an id.
//!
//! ```
//! use anvil_ui::ElementId;
//! let row = ElementId::new("note").key("groceries.md");
//! assert_eq!(row, ElementId::new("note").key("groceries.md"));
//! assert_ne!(row, ElementId::new("note").key("todo.md"));
//! assert_eq!(ElementId::from("save"), ElementId::new("save"));
//! ```

use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(pub u64);

/// FNV-1a: small, fast, and the same on every platform and build, unlike the std hasher's
/// per-process keys. Ids never cross a trust boundary, so a non-cryptographic hash is fine.
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

impl ElementId {
    /// An id from a name.
    pub fn new(name: &str) -> Self {
        let mut h = Fnv(OFFSET);
        h.write(name.as_bytes());
        Self(h.finish())
    }

    /// This id combined with a key, such as a list item's own id or index. `id.key(a).key(b)`
    /// is a different id from `id.key(b).key(a)`.
    pub fn key(self, key: impl Hash) -> Self {
        let mut h = Fnv(self.0 ^ 0x9e37_79b9_7f4a_7c15);
        key.hash(&mut h);
        Self(h.finish())
    }

    /// The same id on another of its sub-parts, such as a widget's close button.
    pub fn part(self, part: &str) -> Self {
        let mut h = Fnv(self.0.rotate_left(17) ^ OFFSET);
        h.write(part.as_bytes());
        Self(h.finish())
    }
}

impl std::fmt::Debug for ElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ElementId({:016x})", self.0)
    }
}

impl From<&str> for ElementId {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<&String> for ElementId {
    fn from(name: &String) -> Self {
        Self::new(name)
    }
}

impl From<String> for ElementId {
    fn from(name: String) -> Self {
        Self::new(&name)
    }
}

impl From<u64> for ElementId {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_distinct_and_keyed() {
        assert_eq!(ElementId::new("a"), ElementId::from("a"));
        assert_ne!(ElementId::new("a"), ElementId::new("b"));
        let base = ElementId::new("row");
        assert_ne!(base.key(1u32), base.key(2u32));
        assert_eq!(base.key("x"), base.key(String::from("x")));
        assert_ne!(base.key(1u32).key(2u32), base.key(2u32).key(1u32));
        assert_ne!(base.part("close"), base.part("action"));
        // Same on every run: FNV-1a of "a".
        assert_eq!(ElementId::new("a").0, 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn many_keys_do_not_collide() {
        let base = ElementId::new("note");
        let mut seen = std::collections::HashSet::new();
        for i in 0..100_000u32 {
            assert!(seen.insert(base.key(i)), "collision at {i}");
        }
    }
}
