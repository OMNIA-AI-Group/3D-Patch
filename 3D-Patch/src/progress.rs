//! Avancement global du calcul en cours (0..1), lu par la jauge du panneau d'outils.
//! Les algorithmes appellent `set(f)` avec leur avancement local ; `span(a, b)` permet à
//! l'appelant de ranger cet avancement dans une tranche [a, b] de la jauge globale.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

static VAL: AtomicU32 = AtomicU32::new(0);
static LO: AtomicU32 = AtomicU32::new(0);
static HI: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32

fn ld(a: &AtomicU32) -> f32 { f32::from_bits(a.load(Relaxed)) }

/// Avancement local (0..1) de l'étape courante.
pub fn set(f: f32) {
    let (lo, hi) = (ld(&LO), ld(&HI));
    let g = lo + (hi - lo) * f.clamp(0.0, 1.0);
    // la jauge ne recule jamais pendant un même calcul
    if g > ld(&VAL) { VAL.store(g.to_bits(), Relaxed); }
}
/// Les prochains `set` seront rangés dans [lo, hi] de la jauge globale.
pub fn span(lo: f32, hi: f32) { LO.store(lo.to_bits(), Relaxed); HI.store(hi.to_bits(), Relaxed); set(0.0); }
pub fn reset() { VAL.store(0, Relaxed); span(0.0, 1.0); }
pub fn get() -> f32 { ld(&VAL) }
