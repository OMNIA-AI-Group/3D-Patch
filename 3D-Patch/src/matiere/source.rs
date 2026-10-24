//! Source du relief : motif calculé ou image de l'utilisateur (carte de hauteur), avec inversion,
//! contraste et lissage. Échantillonnage en carreaux (u, v), répété sans fin.

use super::motifs;
use std::sync::Arc;

/// Image en niveaux de gris (0 = creux, 1 = sommet), répétée en carreaux.
pub struct Carte { pub nom: String, pub w: usize, pub h: usize, pub g: Vec<f32> }

impl Carte {
    /// Charge une image (PNG, JPEG, BMP) ; la transparence compte comme du noir (creux).
    pub fn charger(p: &std::path::Path) -> Result<Carte, String> {
        let img = image::open(p).map_err(|e| e.to_string())?.to_rgba8();
        let (w, h) = img.dimensions();
        let g = img.pixels().map(|q| {
            let l = (q[0] as f32 * 0.299 + q[1] as f32 * 0.587 + q[2] as f32 * 0.114) / 255.0;
            l * q[3] as f32 / 255.0
        }).collect();
        Ok(Carte { nom: p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), w: w as usize, h: h as usize, g })
    }

    /// Valeur bilinéaire au point (u, v) en carreaux (v vers le haut de l'image).
    pub fn lire(&self, u: f32, v: f32) -> f32 {
        if self.w == 0 || self.h == 0 { return 0.0; }
        let x = (u - u.floor()) * self.w as f32 - 0.5;
        let y = (1.0 - (v - v.floor())) * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let px = |i: i64, j: i64| self.g[(j.rem_euclid(self.h as i64) as usize) * self.w + i.rem_euclid(self.w as i64) as usize];
        let (i, j) = (x0 as i64, y0 as i64);
        let a = px(i, j) + (px(i + 1, j) - px(i, j)) * fx;
        let b = px(i, j + 1) + (px(i + 1, j + 1) - px(i, j + 1)) * fx;
        a + (b - a) * fy
    }

    /// Copie adoucie (flou en boîte de rayon `r` px, deux passes ≈ gaussien), raccords répétés.
    pub fn adoucie(&self, r: usize) -> Carte {
        if r == 0 { return Carte { nom: self.nom.clone(), w: self.w, h: self.h, g: self.g.clone() }; }
        let (w, h) = (self.w, self.h);
        let mut a = self.g.clone();
        let mut b = vec![0f32; w * h];
        for _ in 0..2 {
            for y in 0..h { for x in 0..w {
                let mut s = 0.0;
                for k in -(r as i64)..=r as i64 { s += a[y * w + (x as i64 + k).rem_euclid(w as i64) as usize]; }
                b[y * w + x] = s / (2 * r + 1) as f32;
            } }
            for y in 0..h { for x in 0..w {
                let mut s = 0.0;
                for k in -(r as i64)..=r as i64 { s += b[(y as i64 + k).rem_euclid(h as i64) as usize * w + x]; }
                a[y * w + x] = s / (2 * r + 1) as f32;
            } }
        }
        Carte { nom: self.nom.clone(), w, h, g: a }
    }
}

/// D'où vient le relief.
#[derive(Clone)]
pub enum Source {
    Motif(motifs::Params),
    Image(Arc<Carte>),
}

impl Source {
    pub fn brute(&self, u: f32, v: f32) -> f32 {
        match self { Source::Motif(p) => motifs::hauteur(p, u, v), Source::Image(c) => c.lire(u, v) }
    }
}

/// Source + retouches : inversion, contraste (courbe autour du milieu), niveau de base.
#[derive(Clone)]
pub struct Relief {
    pub source: Source,
    pub inverser: bool,
    pub contraste: f32, // 1 = inchangé ; > 1 accentue, < 1 adoucit
}

impl Relief {
    #[inline]
    pub fn lire(&self, u: f32, v: f32) -> f32 {
        let mut h = self.source.brute(u, v);
        if self.inverser { h = 1.0 - h; }
        if (self.contraste - 1.0).abs() > 1e-3 {
            // courbe en S (ou en S inversé) qui garde 0, 0,5 et 1
            let c = self.contraste.max(0.05);
            h = if h < 0.5 { 0.5 * (2.0 * h).powf(c) } else { 1.0 - 0.5 * (2.0 * (1.0 - h)).powf(c) };
        }
        h
    }
}
