//! Gravure de relief sur un maillage façon BumpMesh : on choisit un motif (heightmap
//! procédural = 100% original, rien de copié), un mapping, une hauteur et une échelle,
//! puis on subdivise le maillage et on déplace les sommets le long de la normale lissée.
//! Déplacement dépendant UNIQUEMENT de la position + normale lissée partagée => pas de fissures.

use crate::fasthash::HashMap;
use glam::Vec3;

#[derive(Clone, Copy, PartialEq)]
pub enum Pattern {
    Knurling, Ribs, Reeds, Ripples, Waves, Bricks, Herringbone, Grid, Dots, Cobblestone,
    Hammered, Stipple, Concrete, Leather, Brushed, Bark, CarbonTwill, DiamondPlate,
    Chainmail, Scales, Pyramids, Hexes,
}
pub const PATTERNS: [Pattern; 22] = [
    Pattern::Knurling, Pattern::Ribs, Pattern::Reeds, Pattern::Ripples, Pattern::Waves, Pattern::Bricks,
    Pattern::Herringbone, Pattern::Grid, Pattern::Dots, Pattern::Cobblestone, Pattern::Hammered, Pattern::Stipple,
    Pattern::Concrete, Pattern::Leather, Pattern::Brushed, Pattern::Bark, Pattern::CarbonTwill, Pattern::DiamondPlate,
    Pattern::Chainmail, Pattern::Scales, Pattern::Pyramids, Pattern::Hexes,
];
impl Pattern {
    pub fn key(self) -> &'static str {
        match self {
            Pattern::Knurling => "knurling", Pattern::Ribs => "ribs", Pattern::Reeds => "reeds",
            Pattern::Ripples => "ripples", Pattern::Waves => "waves", Pattern::Bricks => "bricks",
            Pattern::Herringbone => "herringbone", Pattern::Grid => "grid", Pattern::Dots => "dots",
            Pattern::Cobblestone => "cobblestone", Pattern::Hammered => "hammered", Pattern::Stipple => "stipple",
            Pattern::Concrete => "concrete", Pattern::Leather => "leather", Pattern::Brushed => "brushed",
            Pattern::Bark => "bark", Pattern::CarbonTwill => "carbon", Pattern::DiamondPlate => "diamond",
            Pattern::Chainmail => "chainmail", Pattern::Scales => "scales", Pattern::Pyramids => "pyramids",
            Pattern::Hexes => "hexes",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mapping { PlanarZ, Cylindrical, Triplanar }

// ---------- heightmaps procéduraux : (u,v) tuilés -> hauteur [0,1] ----------

fn frac(x: f32) -> f32 { x - x.floor() }
fn smooth(t: f32) -> f32 { t * t * (3.0 - 2.0 * t) }
fn hash2(x: i32, y: i32) -> f32 {
    let mut h = (x.wrapping_mul(374761393).wrapping_add(y.wrapping_mul(668265263))) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}
fn vnoise(u: f32, v: f32) -> f32 {
    let (x, y) = (u.floor() as i32, v.floor() as i32);
    let (fx, fy) = (smooth(frac(u)), smooth(frac(v)));
    let a = hash2(x, y); let b = hash2(x + 1, y);
    let c = hash2(x, y + 1); let d = hash2(x + 1, y + 1);
    let ab = a + (b - a) * fx; let cd = c + (d - c) * fx;
    ab + (cd - ab) * fy
}
fn fbm(u: f32, v: f32) -> f32 {
    let mut f = 0.0; let mut amp = 0.5; let mut fr = 1.0;
    for _ in 0..4 { f += amp * vnoise(u * fr, v * fr); fr *= 2.0; amp *= 0.5; }
    f
}

const TAU: f32 = std::f32::consts::TAU;

/// Hauteur [0,1] du motif `p` au point tuilé (u,v) (une tuile = 1.0).
pub fn height(p: Pattern, u: f32, v: f32) -> f32 {
    match p {
        Pattern::Knurling => {
            let a = ((u + v) * TAU).sin().abs();
            let b = ((u - v) * TAU).sin().abs();
            (a.min(b)).powf(0.6)
        }
        Pattern::Ribs => 0.5 + 0.5 * (u * TAU).cos(),
        Pattern::Ripples => {
            let w = (v * TAU * 3.0 + 0.6 * (u * TAU).sin()).sin();
            0.5 + 0.5 * w
        }
        Pattern::Waves => 0.5 + 0.5 * (u * TAU).sin() * (v * TAU).cos(),
        Pattern::Bricks => {
            let row = (v * 2.0).floor();
            let uu = u * 2.0 + if (row as i32) % 2 == 0 { 0.0 } else { 0.5 };
            let du = (frac(uu) - 0.5).abs() * 2.0;
            let dv = (frac(v * 2.0) - 0.5).abs() * 2.0;
            // mortier (bords) en creux, brique en relief
            let m = du.max(dv);
            smooth((m - 0.1).clamp(0.0, 0.85) / 0.85)
        }
        Pattern::Grid => {
            let du = (frac(u) - 0.5).abs() * 2.0;
            let dv = (frac(v) - 0.5).abs() * 2.0;
            1.0 - du.min(dv).min(0.3) / 0.3 * 1.0
        }
        Pattern::Dots => {
            let du = frac(u) - 0.5; let dv = frac(v) - 0.5;
            let d = (du * du + dv * dv).sqrt() / 0.5;
            (1.0 - d).clamp(0.0, 1.0).powf(0.7)
        }
        Pattern::Hammered => {
            let n = fbm(u * 3.0, v * 3.0);
            smooth((n * 1.3).clamp(0.0, 1.0))
        }
        Pattern::Stipple => {
            let n = 0.6 * vnoise(u * 12.0, v * 12.0) + 0.4 * vnoise(u * 24.0 + 7.0, v * 24.0 + 3.0);
            n.clamp(0.0, 1.0)
        }
        Pattern::CarbonTwill => {
            // sergé 2x2 : diagonales décalées
            let cu = (u * 4.0).floor() as i32; let cv = (v * 4.0).floor() as i32;
            let diag = ((cu + cv).rem_euclid(4)) < 2;
            let local = if diag { frac(u * 4.0) } else { frac(v * 4.0) };
            0.35 + 0.65 * (local * std::f32::consts::PI).sin()
        }
        Pattern::Pyramids => {
            let du = 1.0 - (frac(u) - 0.5).abs() * 2.0;
            let dv = 1.0 - (frac(v) - 0.5).abs() * 2.0;
            du.min(dv)
        }
        Pattern::Hexes => {
            // trame hexagonale approximée (deux grilles décalées de cellules rondes)
            let f = |ou: f32, ov: f32| {
                let du = frac(u + ou) - 0.5; let dv = frac(v + ov) - 0.5;
                (1.0 - (du * du + dv * dv).sqrt() / 0.5).clamp(0.0, 1.0)
            };
            f(0.0, 0.0).max(f(0.5, 0.5))
        }
        Pattern::Reeds => {
            // demi-cylindres verticaux accolés (cannelures rondes)
            let x = frac(u) * 2.0 - 1.0;
            (1.0 - x * x).max(0.0).sqrt()
        }
        Pattern::Herringbone => {
            // chevrons : blocs diagonaux alternant de sens
            let bx = (u * 2.0).floor() as i32; let by = (v * 2.0).floor() as i32;
            let up = (bx + by).rem_euclid(2) == 0;
            let local = if up { frac(u * 2.0 + v * 2.0) } else { frac(u * 2.0 - v * 2.0) };
            smooth((local - 0.08).clamp(0.0, 0.84) / 0.84)
        }
        Pattern::Cobblestone => {
            // pavés arrondis jointés, taille irrégulière (jitter par cellule)
            let cx = u.floor(); let cy = v.floor();
            let jx = hash2(cx as i32, cy as i32) * 0.3 - 0.15;
            let jy = hash2(cx as i32 + 31, cy as i32 + 7) * 0.3 - 0.15;
            let du = frac(u) - 0.5 - jx; let dv = frac(v) - 0.5 - jy;
            (1.0 - (du * du + dv * dv).sqrt() / 0.52).clamp(0.0, 1.0).powf(0.5)
        }
        Pattern::Concrete => {
            let n = 0.5 * fbm(u * 6.0, v * 6.0) + 0.5 * vnoise(u * 20.0, v * 20.0);
            n.clamp(0.0, 1.0)
        }
        Pattern::Leather => {
            // cuir grainé : cellules bombées + micro-grain
            let f = |ou: f32, ov: f32| {
                let du = frac(u + ou) - 0.5; let dv = frac(v + ov) - 0.5;
                (1.0 - (du * du + dv * dv).sqrt() / 0.5).clamp(0.0, 1.0)
            };
            let cells = f(0.0, 0.0).max(f(0.5, 0.35)).max(f(0.25, 0.7));
            (0.65 * cells + 0.35 * fbm(u * 9.0, v * 9.0)).clamp(0.0, 1.0)
        }
        Pattern::Brushed => {
            // métal brossé : fines stries horizontales
            0.35 + 0.5 * vnoise(u * 90.0, v * 5.0) + 0.15 * (v * TAU * 30.0).sin().abs()
        }
        Pattern::Bark => {
            // écorce : sillons verticaux + bruit ridé
            let ridge = 1.0 - (2.0 * vnoise(u * 7.0, v * 2.0) - 1.0).abs();
            (0.5 * ridge + 0.5 * fbm(u * 5.0, v * 1.5)).clamp(0.0, 1.0)
        }
        Pattern::DiamondPlate => {
            // tôle larmée : losanges en relief sur la diagonale
            let a = ((u + v) * TAU * 2.0).sin();
            let b = ((u - v) * TAU * 2.0).sin();
            let bar = if (u.floor() as i32 + v.floor() as i32) % 2 == 0 { a } else { b };
            (0.25 + 0.75 * bar.abs()).clamp(0.0, 1.0)
        }
        Pattern::Chainmail => {
            // cotte de mailles : anneaux entrelacés (deux grilles d'anneaux)
            let ring = |ou: f32, ov: f32| {
                let du = frac(u + ou) - 0.5; let dv = frac(v + ov) - 0.5;
                let d = (du * du + dv * dv).sqrt();
                (1.0 - (d - 0.33).abs() / 0.17).clamp(0.0, 1.0)
            };
            ring(0.0, 0.0).max(ring(0.5, 0.5))
        }
        Pattern::Scales => {
            // écailles : demi-disques qui se recouvrent, rangées décalées
            let row = (v * 1.0).floor() as i32;
            let off = if row.rem_euclid(2) == 0 { 0.0 } else { 0.5 };
            let du = frac(u + off) - 0.5; let dv = frac(v);
            (1.0 - (du * du + (dv * 0.5) * (dv * 0.5)).sqrt() / 0.5).clamp(0.0, 1.0)
        }
    }
}

// ---------- maillage : soudure, subdivision, déplacement ----------

fn weld(verts: &[f32]) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for c in verts.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
    let ext = (hi - lo).length().max(1e-6);
    let cell = (ext * 2e-5).max(1e-7);
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::default();
    let mut pos: Vec<Vec3> = Vec::new();
    let idx = |p: Vec3, map: &mut HashMap<(i64, i64, i64), u32>, pos: &mut Vec<Vec3>| -> u32 {
        let k = ((p.x / cell).round() as i64, (p.y / cell).round() as i64, (p.z / cell).round() as i64);
        *map.entry(k).or_insert_with(|| { pos.push(p); (pos.len() - 1) as u32 })
    };
    let mut tris = Vec::new();
    for c in verts.chunks_exact(18) {
        let a = idx(Vec3::new(c[0], c[1], c[2]), &mut map, &mut pos);
        let b = idx(Vec3::new(c[6], c[7], c[8]), &mut map, &mut pos);
        let d = idx(Vec3::new(c[12], c[13], c[14]), &mut map, &mut pos);
        if a != b && b != d && a != d { tris.push([a, b, d]); }
    }
    (pos, tris)
}

fn subdivide(pos: &mut Vec<Vec3>, tris: Vec<[u32; 3]>) -> Vec<[u32; 3]> {
    let mut mid: HashMap<(u32, u32), u32> = HashMap::default();
    let get = |a: u32, b: u32, pos: &mut Vec<Vec3>, mid: &mut HashMap<(u32, u32), u32>| -> u32 {
        let k = if a < b { (a, b) } else { (b, a) };
        *mid.entry(k).or_insert_with(|| { let m = (pos[a as usize] + pos[b as usize]) * 0.5; pos.push(m); (pos.len() - 1) as u32 })
    };
    let mut out = Vec::with_capacity(tris.len() * 4);
    for [a, b, c] in tris {
        let ab = get(a, b, pos, &mut mid);
        let bc = get(b, c, pos, &mut mid);
        let ca = get(c, a, pos, &mut mid);
        out.push([a, ab, ca]); out.push([ab, b, bc]); out.push([ca, bc, c]); out.push([ab, bc, ca]);
    }
    out
}

fn smooth_normals(pos: &[Vec3], tris: &[[u32; 3]]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; pos.len()];
    for &[a, b, c] in tris {
        let fn_ = (pos[b as usize] - pos[a as usize]).cross(pos[c as usize] - pos[a as usize]);
        n[a as usize] += fn_; n[b as usize] += fn_; n[c as usize] += fn_;
    }
    for v in &mut n { *v = v.normalize_or_zero(); }
    n
}

pub struct Params {
    pub pattern: Pattern,
    pub mapping: Mapping,
    pub height_mm: f32,
    pub scale_mm: f32,
    pub invert: bool,
    /// Si présent, le relief n'est gravé que sur la région dont la normale ≈ ce vecteur
    /// (la face cliquée) ; sinon sur tout l'objet.
    pub mask_normal: Option<Vec3>,
}

fn sample(pr: &Params, p: Vec3, n: Vec3, center: Vec3) -> f32 {
    let s = pr.scale_mm.max(0.1);
    let h = match pr.mapping {
        Mapping::PlanarZ => height(pr.pattern, p.x / s, p.y / s),
        Mapping::Cylindrical => {
            let dx = p.x - center.x; let dy = p.y - center.y;
            let ang = dy.atan2(dx);
            let r = (dx * dx + dy * dy).sqrt();
            height(pr.pattern, ang * r / s, p.z / s)
        }
        Mapping::Triplanar => {
            let w = (n * n).max(Vec3::splat(1e-4));
            let w = w / (w.x + w.y + w.z);
            w.z * height(pr.pattern, p.x / s, p.y / s)
                + w.y * height(pr.pattern, p.x / s, p.z / s)
                + w.x * height(pr.pattern, p.y / s, p.z / s)
        }
    };
    if pr.invert { 1.0 - h } else { h }
}

/// Applique le relief et renvoie une nouvelle soupe de triangles (18 f32/tri, normales à plat).
/// Renvoie aussi (triangles_avant, triangles_après) pour l'info UI.
pub fn apply(verts: &[f32], pr: &Params) -> (Vec<f32>, usize, usize) {
    const CAP: usize = 1_600_000; // plafond de triangles (sécurité, façon BumpMesh)
    let (mut pos, mut tris) = weld(verts);
    let tris_before = tris.len();
    if pos.is_empty() { return (verts.to_vec(), 0, 0); }

    // bornes + centre + arête moyenne
    let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for p in &pos { lo = lo.min(*p); hi = hi.max(*p); }
    let center = (lo + hi) * 0.5;
    let ext = (hi - lo).length().max(1e-3);
    let mut edge = 0.0f32; let mut ne = 0usize;
    for &[a, b, c] in &tris {
        edge += (pos[a as usize] - pos[b as usize]).length();
        edge += (pos[b as usize] - pos[c as usize]).length();
        edge += (pos[c as usize] - pos[a as usize]).length();
        ne += 3;
    }
    let avg_edge = if ne > 0 { edge / ne as f32 } else { ext };

    // niveaux de subdivision : assez fins pour le motif, sous le plafond de triangles
    let target = (pr.scale_mm / 5.0).max(ext * 8e-4);
    let mut e = avg_edge; let mut nt = tris.len();
    let mut levels = 0;
    while e > target && levels < 6 && nt.saturating_mul(4) < CAP { e *= 0.5; nt *= 4; levels += 1; }
    for l in 0..levels { tris = subdivide(&mut pos, tris); crate::progress::set(0.6 * (l + 1) as f32 / levels as f32); }

    // déplacement le long de la normale lissée (partagée => pas de fissure).
    // Masque de face : poids 0→1 selon l'alignement de la normale du sommet avec la face cliquée.
    let normals = smooth_normals(&pos, &tris);
    let base: Vec<Vec3> = pos.clone();
    let mask = pr.mask_normal.map(|n| n.normalize_or_zero());
    let npos = pos.len().max(1);
    for i in 0..pos.len() {
        if i % 65536 == 0 { crate::progress::set(0.6 + 0.4 * i as f32 / npos as f32); }
        let w = match mask {
            Some(mn) => {
                let d = normals[i].dot(mn);
                ((d - 0.55) / 0.35).clamp(0.0, 1.0) // coplanaire = 1, bords adoucis, reste = 0
            }
            None => 1.0,
        };
        if w <= 0.0 { continue; }
        let h = sample(pr, base[i], normals[i], center);
        pos[i] = base[i] + normals[i] * (pr.height_mm * h * w);
    }

    // sortie en soupe à plat
    let mut out = Vec::with_capacity(tris.len() * 18);
    for &[a, b, c] in &tris {
        let (pa, pb, pc) = (pos[a as usize], pos[b as usize], pos[c as usize]);
        let nf = (pb - pa).cross(pc - pa).normalize_or_zero();
        for p in [pa, pb, pc] { out.extend_from_slice(&[p.x, p.y, p.z, nf.x, nf.y, nf.z]); }
    }
    let tris_after = tris.len();
    (out, tris_before, tris_after)
}
