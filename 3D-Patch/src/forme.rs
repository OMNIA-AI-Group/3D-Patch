//! MISE EN FORME : opérations géométriques exactes sur un objet (sommets entrelacés
//! [x y z nx ny nz] × 3 par triangle, couleurs [r g b] par sommet éventuelles).
//! Les normales restent cohérentes (vers l'extérieur) après chaque opération.

use glam::{Mat3, Vec3};

/// Boîte englobante.
pub fn bornes(v: &[f32]) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for c in v.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
    (lo, hi)
}

/// Normales à plat recalculées depuis l'ordre des sommets.
fn normales(v: &mut [f32]) {
    for t in v.chunks_exact_mut(18) {
        let a = Vec3::new(t[0], t[1], t[2]);
        let n = (Vec3::new(t[6], t[7], t[8]) - a).cross(Vec3::new(t[12], t[13], t[14]) - a).normalize_or_zero();
        for k in 0..3 { t[k * 6 + 3] = n.x; t[k * 6 + 4] = n.y; t[k * 6 + 5] = n.z; }
    }
}

/// Rotation `m` autour du point `c` (positions et normales).
pub fn tourner(v: &mut [f32], c: Vec3, m: Mat3) {
    for ch in v.chunks_exact_mut(6) {
        let p = m * (Vec3::new(ch[0], ch[1], ch[2]) - c) + c;
        let n = m * Vec3::new(ch[3], ch[4], ch[5]);
        ch[0] = p.x; ch[1] = p.y; ch[2] = p.z; ch[3] = n.x; ch[4] = n.y; ch[5] = n.z;
    }
}

/// Symétrie par rapport au plan passant par `c`, perpendiculaire à l'axe `axe` (0 X, 1 Y, 2 Z).
/// L'ordre des sommets de chaque triangle est inversé pour que les faces restent tournées vers
/// l'extérieur (sinon l'objet miroir serait « retourné »).
pub fn miroir(v: &mut [f32], cols: &mut [f32], axe: usize, c: Vec3) {
    for ch in v.chunks_exact_mut(6) { ch[axe] = 2.0 * c[axe] - ch[axe]; }
    for t in v.chunks_exact_mut(18) { for k in 0..6 { t.swap(6 + k, 12 + k); } }
    if cols.len() / 3 == v.len() / 6 { for t in cols.chunks_exact_mut(9) { for k in 0..3 { t.swap(3 + k, 6 + k); } } }
    normales(v);
}

/// Mise à l'échelle par axe (facteurs > 0) autour de `c`.
pub fn echelle(v: &mut [f32], c: Vec3, f: Vec3) {
    for ch in v.chunks_exact_mut(6) {
        ch[0] = c.x + (ch[0] - c.x) * f.x; ch[1] = c.y + (ch[1] - c.y) * f.y; ch[2] = c.z + (ch[2] - c.z) * f.z;
    }
    normales(v);
}

/// Translation.
pub fn deplacer(v: &mut [f32], d: Vec3) {
    for ch in v.chunks_exact_mut(6) { ch[0] += d.x; ch[1] += d.y; ch[2] += d.z; }
}

/// Répartition à intervalles égaux le long d'un axe : nouvelles positions (début de boîte) pour
/// des objets donnés par (début, taille), triés par position ; le premier et le dernier ne bougent pas.
pub fn repartir(objets: &[(f32, f32)]) -> Vec<f32> {
    let n = objets.len();
    if n < 3 { return objets.iter().map(|o| o.0).collect(); }
    let mut ordre: Vec<usize> = (0..n).collect();
    ordre.sort_by(|&a, &b| objets[a].0.partial_cmp(&objets[b].0).unwrap_or(std::cmp::Ordering::Equal));
    let debut = objets[ordre[0]].0;
    let fin = objets[ordre[n - 1]].0 + objets[ordre[n - 1]].1;
    let tailles: f32 = objets.iter().map(|o| o.1).sum();
    let ecart = (fin - debut - tailles) / (n - 1) as f32;
    let mut res = vec![0.0; n];
    let mut x = debut;
    for &i in &ordre { res[i] = x; x += objets[i].1 + ecart; }
    res
}
