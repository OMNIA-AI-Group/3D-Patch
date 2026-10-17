//! Hachage rapide (type FxHash) pour les tables internes de géométrie : le hachage standard
//! (SipHash, résistant aux attaques) est inutilement lent pour des indices de sommets.

use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub struct FxHasher { h: u64 }
const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
impl FxHasher {
    #[inline] fn add(&mut self, w: u64) { self.h = (self.h.rotate_left(5) ^ w).wrapping_mul(K); }
}
impl Hasher for FxHasher {
    #[inline] fn write(&mut self, bytes: &[u8]) {
        let mut c = bytes.chunks_exact(8);
        for b in &mut c { self.add(u64::from_le_bytes(b.try_into().unwrap())); }
        for &b in c.remainder() { self.add(b as u64); }
    }
    #[inline] fn write_u8(&mut self, i: u8) { self.add(i as u64); }
    #[inline] fn write_u32(&mut self, i: u32) { self.add(i as u64); }
    #[inline] fn write_u64(&mut self, i: u64) { self.add(i); }
    #[inline] fn write_i64(&mut self, i: i64) { self.add(i as u64); }
    #[inline] fn write_usize(&mut self, i: usize) { self.add(i as u64); }
    #[inline] fn finish(&self) -> u64 { self.h }
}
pub type FxBuild = BuildHasherDefault<FxHasher>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, FxBuild>;
pub type HashSet<K> = std::collections::HashSet<K, FxBuild>;
