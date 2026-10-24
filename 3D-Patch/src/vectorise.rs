//! INSÈRE IMAGE : image réduite en noir & blanc (seuil, Otsu, adaptatif ou N couleurs), contours
//! vectorisés (bords des pixels, simplifiés à la tolérance voulue), mis à l'échelle en mm comme
//! les contours du texte (Y vers le haut, bloc centré sur l'origine, extérieurs dans le sens direct).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Numéro de chaque vectorisation, unique pour toutes les images : une image rechargée ne reprend
/// jamais le numéro de la précédente (sinon son aperçu et son tracé ne seraient pas rafraîchis).
static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Réduction des couleurs vers le noir & blanc.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode { Seuil, Auto, Adaptatif, Couleurs }

#[derive(Clone, PartialEq, Debug)]
pub struct Params {
    pub mode: Mode,
    pub seuil: u8,          // Seuil : luminance (0..255) sous laquelle le pixel est plein
    pub ecart: u8,          // Adaptatif : écart à la moyenne locale (plus grand = moins de plein)
    pub n_coul: u8,         // Couleurs : nombre de couleurs de la palette
    pub pleines: Vec<bool>, // Couleurs : couleurs de la palette mises en plein
    pub inverser: bool,     // négatif
    pub sans_cadre: bool,   // zones pleines touchant le bord de l'image retirées (bordure, fond)
    pub flou: u8,           // rayon du lissage avant réduction (px)
    pub res: u32,           // plus grand côté de l'image réduite (px)
    pub nettoyage: f32,     // surface minimale d'une tache ou d'un trou (px²)
    pub tolerance: f32,     // écart maximal toléré entre le tracé et les bords des pixels (px)
}

impl Default for Params {
    fn default() -> Self {
        Params { mode: Mode::Auto, seuil: 128, ecart: 10, n_coul: 4, pleines: Vec::new(), inverser: false, sans_cadre: false, flou: 0, res: 400, nettoyage: 4.0, tolerance: 1.0 }
    }
}

/// Image chargée et sa vectorisation courante.
pub struct Image {
    pub nom: String,
    orig: image::RgbaImage,
    res: u32,
    pub w: usize,
    pub h: usize,
    rgb: Vec<[u8; 3]>,
    lum: Vec<u8>,
    palette: (u8, u32, Vec<[u8; 3]>), // (n, résolution, couleurs) : recalculée seulement si elles changent
    pub masque: Vec<bool>,
    pub polys: Vec<Vec<[f64; 2]>>, // en pixels, Y vers le haut
    calcule: Option<Params>,
    pub generation: u64,
}

impl Image {
    pub fn charger(p: &Path) -> Result<Image, String> {
        let img = image::open(p).map_err(|e| e.to_string())?.to_rgba8();
        let nom = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut im = Image { nom, orig: img, res: 0, w: 0, h: 0, rgb: Vec::new(), lum: Vec::new(), palette: (0, 0, Vec::new()),
                             masque: Vec::new(), polys: Vec::new(), calcule: None, generation: 0 };
        im.reduire(Params::default().res);
        Ok(im)
    }

    /// Image ramenée à `res` px sur son plus grand côté (jamais agrandie) ; transparent = blanc.
    fn reduire(&mut self, res: u32) {
        let (w0, h0) = self.orig.dimensions();
        let k = (res as f32 / w0.max(h0) as f32).min(1.0);
        let (w, h) = (((w0 as f32 * k).round() as u32).max(1), ((h0 as f32 * k).round() as u32).max(1));
        let r = if (w, h) == (w0, h0) { self.orig.clone() } else { image::imageops::resize(&self.orig, w, h, image::imageops::FilterType::Triangle) };
        self.rgb = r.pixels().map(|p| {
            let a = p[3] as u32;
            let m = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
            [m(p[0]), m(p[1]), m(p[2])]
        }).collect();
        self.lum = self.rgb.iter().map(|c| ((c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000) as u8).collect();
        (self.w, self.h, self.res) = (w as usize, h as usize, res);
    }

    /// Palette réduite (mode Couleurs), de la plus sombre à la plus claire.
    pub fn palette(&self) -> &[[u8; 3]] { &self.palette.2 }

    /// Recalcule masque et contours si les réglages ont changé ; vrai s'il y a eu du nouveau.
    pub fn maj(&mut self, prm: &mut Params) -> bool {
        if self.res != prm.res { self.reduire(prm.res); self.calcule = None; }
        if prm.mode == Mode::Couleurs && (self.palette.0 != prm.n_coul || self.palette.1 != self.res) {
            self.palette = (prm.n_coul, self.res, kmoyennes(&self.rgb, prm.n_coul as usize));
            // par défaut : les couleurs sombres en plein
            prm.pleines = self.palette.2.iter().map(|c| lum(*c) < 128).collect();
        }
        if prm.pleines.len() != self.palette.2.len() { prm.pleines.resize(self.palette.2.len(), false); }
        if self.calcule.as_ref() == Some(prm) { return false; }
        self.masque = self.masque(prm);
        self.polys = contours(&self.masque, self.w, self.h, prm.nettoyage as f64, prm.tolerance as f64);
        self.calcule = Some(prm.clone());
        self.generation = GENERATION.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn masque(&self, prm: &Params) -> Vec<bool> {
        let (w, h) = (self.w, self.h);
        let mut m: Vec<bool> = match prm.mode {
            Mode::Couleurs => {
                let pal = &self.palette.2;
                let rgb = if prm.flou > 0 {
                    let canal = |k: usize| flou(&self.rgb.iter().map(|c| c[k]).collect::<Vec<_>>(), w, h, prm.flou as usize);
                    let (r, g, b) = (canal(0), canal(1), canal(2));
                    (0..w * h).map(|i| [r[i], g[i], b[i]]).collect()
                } else { self.rgb.clone() };
                rgb.iter().map(|c| prm.pleines.get(plus_proche(pal, *c)).copied().unwrap_or(false)).collect()
            }
            _ => {
                let l = if prm.flou > 0 { flou(&self.lum, w, h, prm.flou as usize) } else { self.lum.clone() };
                match prm.mode {
                    Mode::Seuil => l.iter().map(|&v| v < prm.seuil).collect(),
                    Mode::Auto => { let s = otsu(&l); l.iter().map(|&v| v <= s).collect() }
                    _ => {
                        // moyenne locale sur une fenêtre de 1/12 de l'image (images intégrales)
                        let r = (w.max(h) / 24).max(4);
                        let moy = flou(&l, w, h, r);
                        l.iter().zip(&moy).map(|(&v, &m)| (v as i32) < m as i32 - prm.ecart as i32).collect()
                    }
                }
            }
        };
        if prm.inverser { for v in m.iter_mut() { *v = !*v; } }
        if prm.sans_cadre { retirer_bord(&mut m, w, h); }
        m
    }

    /// Contours en mm : `largeur` = largeur du dessin, bloc centré ; (contours, largeur, hauteur).
    pub fn contours_mm(&self, largeur: f32) -> (Vec<Vec<[f64; 2]>>, f32, f32) {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for c in &self.polys { for q in c { for d in 0..2 { lo[d] = lo[d].min(q[d]); hi[d] = hi[d].max(q[d]); } } }
        if self.polys.is_empty() || hi[0] <= lo[0] { return (Vec::new(), 0.0, 0.0); }
        let k = largeur as f64 / (hi[0] - lo[0]);
        let (cx, cy) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
        let p = self.polys.iter().map(|c| c.iter().map(|q| [(q[0] - cx) * k, (q[1] - cy) * k]).collect()).collect();
        (p, largeur, ((hi[1] - lo[1]) * k) as f32)
    }
}

/// Vide toutes les zones pleines reliées au bord de l'image (remplissage depuis le pourtour, 4-voisins).
fn retirer_bord(m: &mut [bool], w: usize, h: usize) {
    let mut pile: Vec<usize> = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1])).collect();
    while let Some(i) = pile.pop() {
        if !m[i] { continue; }
        m[i] = false;
        let (x, y) = (i % w, i / w);
        if x > 0 { pile.push(i - 1); }
        if x + 1 < w { pile.push(i + 1); }
        if y > 0 { pile.push(i - w); }
        if y + 1 < h { pile.push(i + w); }
    }
}

fn lum(c: [u8; 3]) -> u32 { (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000 }

fn plus_proche(pal: &[[u8; 3]], c: [u8; 3]) -> usize {
    let d = |p: &[u8; 3]| (0..3).map(|k| { let e = p[k] as i32 - c[k] as i32; e * e }).sum::<i32>();
    (0..pal.len()).min_by_key(|&i| d(&pal[i])).unwrap_or(0)
}

/// Palette de `n` couleurs (k-moyennes à départ déterministe : résultat stable d'un calcul à l'autre).
fn kmoyennes(px: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    let n = n.clamp(2, 16);
    let pas = (px.len() / 60_000).max(1); // échantillon
    let ech: Vec<[u8; 3]> = px.iter().step_by(pas).copied().collect();
    if ech.is_empty() { return vec![[0; 3], [255; 3]]; }
    // départ : la couleur la plus sombre, puis chaque fois celle la plus éloignée des centres déjà pris
    // (les petites zones de couleur franche ne sont pas noyées dans le fond)
    let f = |c: [u8; 3]| [c[0] as f32, c[1] as f32, c[2] as f32];
    let d2 = |a: &[f32; 3], c: &[u8; 3]| (0..3).map(|k| (a[k] - c[k] as f32).powi(2)).sum::<f32>();
    let mut centres: Vec<[f32; 3]> = vec![f(*ech.iter().min_by_key(|c| lum(**c)).unwrap())];
    let mut dmin: Vec<f32> = ech.iter().map(|c| d2(&centres[0], c)).collect();
    while centres.len() < n {
        let (i, &d) = dmin.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        if d < 1.0 { break; } // moins de couleurs distinctes que demandé
        centres.push(f(ech[i]));
        let c = *centres.last().unwrap();
        for (k, q) in ech.iter().enumerate() { dmin[k] = dmin[k].min(d2(&c, q)); }
    }
    let n = centres.len();
    for _ in 0..12 {
        let mut somme = vec![[0f32; 4]; n];
        for c in &ech {
            let d = |p: &[f32; 3]| (0..3).map(|k| (p[k] - c[k] as f32).powi(2)).sum::<f32>();
            let i = (0..n).min_by(|&a, &b| d(&centres[a]).total_cmp(&d(&centres[b]))).unwrap();
            for k in 0..3 { somme[i][k] += c[k] as f32; }
            somme[i][3] += 1.0;
        }
        for i in 0..n { if somme[i][3] > 0.0 { for k in 0..3 { centres[i][k] = somme[i][k] / somme[i][3]; } } }
    }
    let mut pal: Vec<[u8; 3]> = centres.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]).collect();
    pal.sort_by_key(|c| lum(*c));
    pal.dedup();
    pal
}

/// Seuil d'Otsu (sépare au mieux les clairs des sombres).
fn otsu(l: &[u8]) -> u8 {
    let mut hist = [0f64; 256];
    for &v in l { hist[v as usize] += 1.0; }
    let total = l.len() as f64;
    let somme: f64 = (0..256).map(|i| i as f64 * hist[i]).sum();
    let (mut sb, mut wb, mut best, mut seuil) = (0.0, 0.0, -1.0, 128u8);
    for t in 0..256 {
        wb += hist[t];
        if wb == 0.0 { continue; }
        let wf = total - wb;
        if wf == 0.0 { break; }
        sb += t as f64 * hist[t];
        let (mb, mf) = (sb / wb, (somme - sb) / wf);
        let v = wb * wf * (mb - mf) * (mb - mf);
        if v > best { best = v; seuil = t as u8; }
    }
    seuil
}

/// Flou en boîte de rayon `r` (image intégrale, bords repliés).
fn flou(l: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut ii = vec![0u64; (w + 1) * (h + 1)];
    for y in 0..h { let mut s = 0u64; for x in 0..w { s += l[y * w + x] as u64; ii[(y + 1) * (w + 1) + x + 1] = ii[y * (w + 1) + x + 1] + s; } }
    let mut out = vec![0u8; w * h];
    for y in 0..h { for x in 0..w {
        let (x0, x1, y0, y1) = (x.saturating_sub(r), (x + r + 1).min(w), y.saturating_sub(r), (y + r + 1).min(h));
        let s = ii[y1 * (w + 1) + x1] + ii[y0 * (w + 1) + x0] - ii[y0 * (w + 1) + x1] - ii[y1 * (w + 1) + x0];
        out[y * w + x] = (s / ((x1 - x0) * (y1 - y0)) as u64) as u8;
    } }
    out
}

/// Bords des zones pleines du masque (`w`×`h`, ligne 0 en haut) en contours fermés, Y vers le haut :
/// extérieurs dans le sens direct, trous dans le sens indirect. Les taches et trous plus petits que
/// `aire_min` px² sont retirés ; les tracés sont simplifiés à `tol` px.
pub fn contours(m: &[bool], w: usize, h: usize, aire_min: f64, tol: f64) -> Vec<Vec<[f64; 2]>> {
    let plein = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && m[y as usize * w + x as usize];
    let w1 = w + 1;
    let id = |x: usize, y: usize| (y * w1 + x) as u32; // sommet (x, Y) de la grille, Y = h − ligne
    // arêtes orientées, le plein à gauche ; au plus 2 arêtes partent d'un même sommet (pixels en diagonale)
    let mut arr: Vec<(u32, u32)> = Vec::new();
    let mut part = vec![[u32::MAX; 2]; w1 * (h + 1)];
    let mut ajoute = |a: u32, b: u32, arr: &mut Vec<(u32, u32)>| {
        let s = &mut part[a as usize];
        if s[0] == u32::MAX { s[0] = arr.len() as u32 } else { s[1] = arr.len() as u32 }
        arr.push((a, b));
    };
    for y in 0..h { for x in 0..w {
        if !m[y * w + x] { continue; }
        let (xi, yi) = (x as isize, y as isize);
        let (bas, haut) = (h - y - 1, h - y);
        if !plein(xi, yi + 1) { ajoute(id(x, bas), id(x + 1, bas), &mut arr); }
        if !plein(xi + 1, yi) { ajoute(id(x + 1, bas), id(x + 1, haut), &mut arr); }
        if !plein(xi, yi - 1) { ajoute(id(x + 1, haut), id(x, haut), &mut arr); }
        if !plein(xi - 1, yi) { ajoute(id(x, haut), id(x, bas), &mut arr); }
    } }
    let xy = |v: u32| [(v as usize % w1) as f64, (v as usize / w1) as f64];
    let mut pris = vec![false; arr.len()];
    let mut out = Vec::new();
    for e0 in 0..arr.len() {
        if pris[e0] { continue; }
        // chaînage : point = milieu de chaque arête (les marches d'escalier deviennent des pentes)
        let mut boucle: Vec<[f64; 2]> = Vec::new();
        let mut e = e0;
        loop {
            pris[e] = true;
            let (a, b) = arr[e];
            let (pa, pb) = (xy(a), xy(b));
            boucle.push([(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5]);
            let s = part[b as usize];
            let suiv = if s[1] == u32::MAX || pris[s[1] as usize] { s[0] } else if pris[s[0] as usize] { s[1] } else {
                // pixels en diagonale : on tourne à gauche (les deux pixels restent séparés)
                let d = [pb[0] - pa[0], pb[1] - pa[1]];
                let dir = |k: u32| { let q = xy(arr[k as usize].1); [q[0] - pb[0], q[1] - pb[1]] };
                if d[0] * dir(s[0])[1] - d[1] * dir(s[0])[0] > 0.0 { s[0] } else { s[1] }
            };
            if suiv == u32::MAX || pris[suiv as usize] { break; }
            e = suiv as usize;
        }
        if boucle.len() < 3 || aire(&boucle).abs() < aire_min.max(0.25) { continue; }
        let s = simplifier(&boucle, tol);
        if s.len() >= 3 { out.push(s); }
    }
    out
}

pub fn aire(c: &[[f64; 2]]) -> f64 {
    let n = c.len();
    (0..n).map(|i| { let (a, b) = (c[i], c[(i + 1) % n]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>() * 0.5
}

/// Douglas-Peucker sur une boucle fermée (coupée en deux au point le plus éloigné du premier).
fn simplifier(c: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    if tol <= 0.0 || c.len() < 4 { return c.to_vec(); }
    let d2 = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    let loin = (1..c.len()).max_by(|&i, &j| d2(c[0], c[i]).total_cmp(&d2(c[0], c[j]))).unwrap();
    let mut garde = vec![false; c.len()];
    garde[0] = true; garde[loin] = true;
    let ferme: Vec<[f64; 2]> = c.iter().copied().chain(std::iter::once(c[0])).collect();
    dp(&ferme, 0, loin, tol, &mut garde);
    let mut g2 = vec![false; ferme.len()];
    dp(&ferme, loin, c.len(), tol, &mut g2);
    for i in 0..c.len() { garde[i] |= g2[i]; }
    c.iter().zip(&garde).filter(|(_, &g)| g).map(|(p, _)| *p).collect()
}

fn dp(c: &[[f64; 2]], i: usize, j: usize, tol: f64, garde: &mut [bool]) {
    if j <= i + 1 { return; }
    let (a, b) = (c[i], c[j]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = (dx * dx + dy * dy).sqrt();
    let (mut kmax, mut dmax) = (0, 0.0);
    for k in i + 1..j {
        let p = c[k];
        let d = if l < 1e-12 { ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt() } else { ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / l };
        if d > dmax { dmax = d; kmax = k; }
    }
    if dmax > tol {
        garde[kmax] = true;
        dp(c, i, kmax, tol, garde);
        dp(c, kmax, j, tol, garde);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carre_troue() {
        // carré plein 8×8 avec un trou 2×2 au milieu : 1 extérieur direct + 1 trou indirect
        let (w, h) = (10, 10);
        let mut m = vec![false; w * h];
        for y in 1..9 { for x in 1..9 { m[y * w + x] = !(4..6).contains(&x) || !(4..6).contains(&y); } }
        let c = contours(&m, w, h, 0.0, 0.0);
        assert_eq!(c.len(), 2);
        let a: Vec<f64> = c.iter().map(|b| aire(b)).collect();
        assert!(a.iter().any(|&x| x > 50.0) && a.iter().any(|&x| x < 0.0));
    }

    #[test]
    fn cadre_retire() {
        // bordure pleine sur le pourtour + disque plein au centre : seul le disque reste
        let (w, h) = (20, 20);
        let mut m: Vec<bool> = (0..w * h).map(|i| { let (x, y) = (i % w, i / w); x < 2 || y < 2 || x >= w - 2 || y >= h - 2 }).collect();
        for y in 7..13 { for x in 7..13 { m[y * w + x] = true; } }
        retirer_bord(&mut m, w, h);
        assert_eq!(m.iter().filter(|&&b| b).count(), 36);
        assert_eq!(contours(&m, w, h, 0.0, 0.0).len(), 1);
    }

    #[test]
    fn diagonale_separee() {
        let (w, h) = (4, 4);
        let mut m = vec![false; w * h];
        m[1 * w + 1] = true; m[2 * w + 2] = true;
        assert_eq!(contours(&m, w, h, 0.0, 0.0).len(), 2);
    }
}
