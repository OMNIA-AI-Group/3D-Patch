//! TEXTE & RELIEF : polices du système, mise en page (lignes, justification, taille en mm),
//! contours des lettres (en mm, axe Y vers le haut, bloc centré sur l'origine), aperçu 2D
//! (triangles) et volume 3D extrudé.

use ab_glyph::{Font, FontVec, OutlineCurve};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Une police installée : nom affiché (famille + style), fichier, index dans une collection.
#[derive(Clone)]
pub struct Police { pub nom: String, pub chemin: PathBuf, pub index: u32 }

static POLICES: OnceLock<Mutex<Option<Arc<Vec<Police>>>>> = OnceLock::new();

/// Polices du système (recensées une seule fois, en arrière-plan) ; None pendant le recensement.
pub fn polices() -> Option<Arc<Vec<Police>>> {
    let cell = POLICES.get_or_init(|| {
        std::thread::spawn(|| {
            let liste = recenser();
            if let Some(m) = POLICES.get() { *m.lock().unwrap() = Some(Arc::new(liste)); }
        });
        Mutex::new(None)
    });
    cell.lock().unwrap().clone()
}

fn dossiers() -> Vec<PathBuf> {
    let mut d: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        d.push(PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts"));
        if let Ok(l) = std::env::var("LOCALAPPDATA") { d.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts")); }
    } else {
        for p in ["/usr/share/fonts", "/usr/local/share/fonts", "/Library/Fonts", "/System/Library/Fonts"] { d.push(PathBuf::from(p)); }
        if let Ok(h) = std::env::var("HOME") { d.push(PathBuf::from(&h).join(".local/share/fonts")); d.push(PathBuf::from(&h).join(".fonts")); }
    }
    d
}

fn recenser() -> Vec<Police> {
    let mut fichiers: Vec<PathBuf> = Vec::new();
    let mut pile = dossiers();
    while let Some(d) = pile.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { pile.push(p); continue; }
            let ext = p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).unwrap_or_default();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") { fichiers.push(p); }
        }
    }
    let mut vus: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    // polices intégrées à l'appli (celles de l'interface) : toujours disponibles, même sans police système
    for (nom, cle) in [("Ubuntu Light", "Ubuntu-Light"), ("Hack", "Hack")] {
        if integree(cle).is_some() { vus.insert(nom.to_lowercase()); out.push(Police { nom: nom.into(), chemin: PathBuf::from(format!("{INTEGREE}{cle}")), index: 0 }); }
    }
    let nb_integrees = out.len();
    for f in fichiers {
        let Ok(data) = std::fs::read(&f) else { continue };
        let n = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for i in 0..n.min(16) {
            let Ok(face) = ttf_parser::Face::parse(&data, i) else { continue };
            let nom_id = |ids: &[u16]| ids.iter().find_map(|&id| face.names().into_iter().find(|n| n.name_id == id && n.is_unicode()).and_then(|n| n.to_string()));
            let famille = nom_id(&[16, 1]).unwrap_or_else(|| f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            let style = nom_id(&[17, 2]).unwrap_or_default();
            // seulement les polices qui ont les lettres latines de base
            if face.glyph_index('A').is_none() { continue; }
            let nom = if style.is_empty() || style.eq_ignore_ascii_case("regular") { famille } else { format!("{famille} {style}") };
            if vus.insert(nom.to_lowercase()) { out.push(Police { nom, chemin: f.clone(), index: i }); }
        }
    }
    out[nb_integrees..].sort_by(|a, b| a.nom.to_lowercase().cmp(&b.nom.to_lowercase()));
    out
}

const INTEGREE: &str = "integree:";
/// Données d'une police intégrée (polices de l'interface egui).
fn integree(cle: &str) -> Option<Vec<u8>> {
    egui::FontDefinitions::default().font_data.get(cle).map(|f| f.font.to_vec())
}

static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u32), Arc<FontVec>>>> = OnceLock::new();

/// Octets du fichier de police (pour l'afficher aussi dans l'interface).
pub fn octets(p: &Police) -> Option<Vec<u8>> {
    match p.chemin.to_str().and_then(|c| c.strip_prefix(INTEGREE)) { Some(cle) => integree(cle), None => std::fs::read(&p.chemin).ok() }
}

fn charger(p: &Police) -> Option<Arc<FontVec>> {
    let c = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cle = (p.chemin.clone(), p.index);
    if let Some(f) = c.lock().unwrap().get(&cle) { return Some(f.clone()); }
    let data = match p.chemin.to_str().and_then(|c| c.strip_prefix(INTEGREE)) { Some(cle) => integree(cle)?, None => std::fs::read(&p.chemin).ok()? };
    let f = Arc::new(FontVec::try_from_vec_and_index(data, p.index).ok()?);
    let mut m = c.lock().unwrap();
    if m.len() > 40 { m.clear(); } // borne mémoire (aperçus de nombreuses polices)
    m.insert(cle, f.clone());
    Some(f)
}

/// Contours du texte (mm, Y vers le haut), bloc centré sur l'origine : `taille` = hauteur de
/// corps (em) en mm, `just` 0 gauche / 1 centre / 2 droite, alignée sur un cadre de largeur
/// `largeur` (0 = largeur du texte). Renvoie (contours, largeur, hauteur du bloc).
pub fn contours(p: &Police, texte: &str, taille: f32, just: u8, largeur: f32) -> Option<(Vec<Vec<[f64; 2]>>, f32, f32)> {
    let f = charger(p)?;
    let upm = f.units_per_em().unwrap_or(1000.0);
    let k = (taille / upm) as f64;
    let (asc, desc, gap) = (f.ascent_unscaled() as f64, f.descent_unscaled() as f64, f.line_gap_unscaled() as f64);
    let interligne = (asc - desc + gap) * k;
    let lignes: Vec<&str> = texte.split('\n').collect();
    // passe 1 : glyphes et largeur de chaque ligne
    let mut par_ligne: Vec<(Vec<(ab_glyph::GlyphId, f64)>, f64)> = Vec::new();
    for l in &lignes {
        let mut x = 0.0f64;
        let mut gl = Vec::new();
        let mut prec: Option<ab_glyph::GlyphId> = None;
        for ch in l.chars() {
            let id = f.glyph_id(ch);
            if let Some(pr) = prec { x += f.kern_unscaled(pr, id) as f64 * k; }
            gl.push((id, x));
            x += f.h_advance_unscaled(id) as f64 * k;
            prec = Some(id);
        }
        par_ligne.push((gl, x));
    }
    let wmax = par_ligne.iter().map(|l| l.1).fold(0.0, f64::max);
    let w_bloc = if largeur > 0.0 { largeur as f64 } else { wmax };
    let h_bloc = interligne * lignes.len() as f64;
    let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
    // première ligne de base : bloc centré verticalement
    let mut base = h_bloc * 0.5 - asc * k;
    for (gl, w) in &par_ligne {
        let x0 = match just { 0 => -w_bloc * 0.5, 2 => w_bloc * 0.5 - w, _ => -w * 0.5 };
        for &(id, gx) in gl {
            let Some(o) = f.outline(id) else { continue };
            let mut glyphe: Vec<Vec<[f64; 2]>> = Vec::new();
            let mut cur: Vec<[f64; 2]> = Vec::new();
            let pt = |q: ab_glyph::Point| [x0 + gx + q.x as f64 * k, base + q.y as f64 * k];
            let mut dernier: Option<ab_glyph::Point> = None;
            for c in &o.curves {
                let (debut, pts): (ab_glyph::Point, Vec<[f64; 2]>) = match *c {
                    OutlineCurve::Line(a, b) => (a, vec![pt(b)]),
                    OutlineCurve::Quad(a, b, c2) => (a, (1..=8).map(|i| { let t = i as f32 / 8.0; let u = 1.0 - t;
                        pt(ab_glyph::point(u * u * a.x + 2.0 * u * t * b.x + t * t * c2.x, u * u * a.y + 2.0 * u * t * b.y + t * t * c2.y)) }).collect()),
                    OutlineCurve::Cubic(a, b, c2, d) => (a, (1..=12).map(|i| { let t = i as f32 / 12.0; let u = 1.0 - t;
                        pt(ab_glyph::point(u * u * u * a.x + 3.0 * u * u * t * b.x + 3.0 * u * t * t * c2.x + t * t * t * d.x,
                                           u * u * u * a.y + 3.0 * u * u * t * b.y + 3.0 * u * t * t * c2.y + t * t * t * d.y)) }).collect()),
                };
                // nouveau contour quand la courbe ne part pas de la fin de la précédente
                if dernier.map_or(true, |d| (d.x - debut.x).abs() > 1e-3 || (d.y - debut.y).abs() > 1e-3) {
                    if cur.len() >= 3 { glyphe.push(std::mem::take(&mut cur)); } else { cur.clear(); }
                    cur.push(pt(debut));
                }
                cur.extend(pts);
                dernier = Some(match *c { OutlineCurve::Line(_, b) => b, OutlineCurve::Quad(_, _, c2) => c2, OutlineCurve::Cubic(_, _, _, d) => d });
            }
            if cur.len() >= 3 { glyphe.push(cur); }
            // sens des contours : extérieurs dans le sens direct (TrueType les donne à l'envers)
            let aire: f64 = glyphe.iter().map(|c| aire_signee(c)).sum();
            if aire < 0.0 { for c in glyphe.iter_mut() { c.reverse(); } }
            polys.extend(glyphe);
        }
        base -= interligne;
    }
    Some((polys, w_bloc as f32, h_bloc as f32))
}

fn aire_signee(c: &[[f64; 2]]) -> f64 {
    let n = c.len();
    (0..n).map(|i| { let (a, b) = (c[i], c[(i + 1) % n]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>() * 0.5
}

/// Polygones Manifold (union : contours qui se chevauchent fusionnés, trous respectés).
pub fn section(polys: &[Vec<[f64; 2]>]) -> manifold_rust::types::Polygons {
    use manifold_rust::linalg::Vec2;
    let p: manifold_rust::types::Polygons = polys.iter().map(|c| c.iter().map(|q| Vec2::new(q[0], q[1])).collect()).collect();
    manifold_rust::cross_section::CrossSection::new(p).to_polygons()
}

/// Aperçu : triangles 2D remplis du texte (pour l'afficher tel quel dans l'interface).
pub fn triangles(polys: &[Vec<[f64; 2]>]) -> Vec<[[f32; 2]; 3]> {
    let p = section(polys);
    let pts: Vec<[f32; 2]> = p.iter().flatten().map(|v| [v.x as f32, v.y as f32]).collect();
    manifold_rust::polygon::triangulate(&p, -1.0, true).iter()
        .filter_map(|t| Some([*pts.get(t.x as usize)?, *pts.get(t.y as usize)?, *pts.get(t.z as usize)?])).collect()
}
