//! Motifs de relief calculés : chaque motif est une hauteur h(u, v) ∈ [0, 1] définie sur un carreau
//! unité qui se répète sans couture (u, v en carreaux : 1,0 = un motif entier). Aucun fichier image :
//! tout est généré, réglable (joint, arrondi, densité, variante) et identique d'une machine à l'autre.

use std::f32::consts::{PI, TAU};

/// Liste des motifs, dans l'ordre de la galerie.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Motif {
    Briques, Dalles, Hexagones, Losanges, Stries, Vagues, Ondulations, Pois, Ecailles, Chevrons,
    Tissage, Carbone, Pierres, Craquelures, Grain, Bois, Cuir, Martele, Pyramides, Grillage,
    Damier, Triangles, Terrasses, Tressage,
    BatonsRompus, TolerLarmee, Golf, Plots, Croix, Etoiles, Capitonne, Grecque, Zigzag, Cercles, Spirale, Truchet, Labyrinthe, CourbesNiveau, Ecorce, Sable, Crepi, Beton, Eponge, PeauOrange, Alveoles, Oeufs, Gaufre, Pneu, Crampons, Picots, RainuresObliques, Toile, Jean, Tricot, Corde, Mailles, Bambou, Paves, Tuiles, ArtDeco, Argyle, OndesCroisees, Bulles, Hachures,
    Quadrillage, Pointillage, Vulcanise, MicroPointes, PointesDiamant, Rape, Ventouses, GripTape, AnneauxManche,
    MoletagePointu, PyramidesTri, Epines,
}

pub const TOUS: [Motif; 76] = [
    Motif::Briques, Motif::Dalles, Motif::Hexagones, Motif::Losanges, Motif::Stries, Motif::Vagues,
    Motif::Ondulations, Motif::Pois, Motif::Ecailles, Motif::Chevrons, Motif::Tissage, Motif::Carbone,
    Motif::Pierres, Motif::Craquelures, Motif::Grain, Motif::Bois, Motif::Cuir, Motif::Martele,
    Motif::Pyramides, Motif::Grillage, Motif::Damier, Motif::Triangles, Motif::Terrasses, Motif::Tressage,
    Motif::BatonsRompus, Motif::TolerLarmee, Motif::Golf, Motif::Plots, Motif::Croix, Motif::Etoiles, Motif::Capitonne, Motif::Grecque, Motif::Zigzag, Motif::Cercles, Motif::Spirale, Motif::Truchet, Motif::Labyrinthe, Motif::CourbesNiveau, Motif::Ecorce, Motif::Sable, Motif::Crepi, Motif::Beton, Motif::Eponge, Motif::PeauOrange, Motif::Alveoles, Motif::Oeufs, Motif::Gaufre, Motif::Pneu, Motif::Crampons, Motif::Picots, Motif::RainuresObliques, Motif::Toile, Motif::Jean, Motif::Tricot, Motif::Corde, Motif::Mailles, Motif::Bambou, Motif::Paves, Motif::Tuiles, Motif::ArtDeco, Motif::Argyle, Motif::OndesCroisees, Motif::Bulles, Motif::Hachures,
    Motif::Quadrillage, Motif::Pointillage, Motif::Vulcanise, Motif::MicroPointes, Motif::PointesDiamant, Motif::Rape, Motif::Ventouses, Motif::GripTape, Motif::AnneauxManche,
    Motif::MoletagePointu, Motif::PyramidesTri, Motif::Epines,
];

/// Textures GRIP (micro-pointes qui accrochent la peau ou les gants), présentées à part, en tête de galerie.
pub const GRIP: [Motif; 10] = [
    Motif::Quadrillage, Motif::MicroPointes, Motif::PointesDiamant, Motif::MoletagePointu, Motif::PyramidesTri,
    Motif::Pointillage, Motif::GripTape, Motif::Rape, Motif::Epines, Motif::Picots,
];

/// Famille du motif (rangement de la galerie).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Famille { Grip, Geometrique, Prise, Nature, Textile, Pavage }

pub const FAMILLES: [Famille; 6] = [Famille::Grip, Famille::Geometrique, Famille::Prise, Famille::Nature, Famille::Textile, Famille::Pavage];

impl Famille {
    pub fn cle(self) -> &'static str {
        match self { Famille::Grip => "f_grip", Famille::Geometrique => "f_geo", Famille::Prise => "f_prise", Famille::Nature => "f_nature", Famille::Textile => "f_textile", Famille::Pavage => "f_pavage" }
    }
}

impl Motif {
    /// Clé de traduction du nom.
    pub fn cle(self) -> &'static str {
        match self {
            Motif::Briques => "m_briques", Motif::Dalles => "m_dalles", Motif::Hexagones => "m_hexagones",
            Motif::Losanges => "m_losanges", Motif::Stries => "m_stries", Motif::Vagues => "m_vagues",
            Motif::Ondulations => "m_ondulations", Motif::Pois => "m_pois", Motif::Ecailles => "m_ecailles",
            Motif::Chevrons => "m_chevrons", Motif::Tissage => "m_tissage", Motif::Carbone => "m_carbone",
            Motif::Pierres => "m_pierres", Motif::Craquelures => "m_craquelures", Motif::Grain => "m_grain",
            Motif::Bois => "m_bois", Motif::Cuir => "m_cuir", Motif::Martele => "m_martele",
            Motif::Pyramides => "m_pyramides", Motif::Grillage => "m_grillage", Motif::Damier => "m_damier",
            Motif::Triangles => "m_triangles", Motif::Terrasses => "m_terrasses", Motif::Tressage => "m_tressage",
            Motif::BatonsRompus => "m_batons", Motif::TolerLarmee => "m_toler", Motif::Golf => "m_golf", Motif::Plots => "m_plots", Motif::Croix => "m_croix", Motif::Etoiles => "m_etoiles", Motif::Capitonne => "m_capitonne", Motif::Grecque => "m_grecque", Motif::Zigzag => "m_zigzag", Motif::Cercles => "m_cercles", Motif::Spirale => "m_spirale", Motif::Truchet => "m_truchet", Motif::Labyrinthe => "m_labyrinthe", Motif::CourbesNiveau => "m_niveaux", Motif::Ecorce => "m_ecorce", Motif::Sable => "m_sable", Motif::Crepi => "m_crepi", Motif::Beton => "m_beton", Motif::Eponge => "m_eponge", Motif::PeauOrange => "m_peau_orange", Motif::Alveoles => "m_alveoles", Motif::Oeufs => "m_oeufs", Motif::Gaufre => "m_gaufre", Motif::Pneu => "m_pneu", Motif::Crampons => "m_crampons", Motif::Picots => "m_picots", Motif::RainuresObliques => "m_obliques", Motif::Toile => "m_toile", Motif::Jean => "m_jean", Motif::Tricot => "m_tricot", Motif::Corde => "m_corde", Motif::Mailles => "m_mailles", Motif::Bambou => "m_bambou", Motif::Paves => "m_paves", Motif::Tuiles => "m_tuiles", Motif::ArtDeco => "m_artdeco", Motif::Argyle => "m_argyle", Motif::OndesCroisees => "m_ondes", Motif::Bulles => "m_bulles", Motif::Hachures => "m_hachures",
            Motif::Quadrillage => "m_quadrillage", Motif::Pointillage => "m_pointillage", Motif::Vulcanise => "m_vulcanise", Motif::MicroPointes => "m_micropointes", Motif::PointesDiamant => "m_pointes_diamant", Motif::Rape => "m_rape", Motif::Ventouses => "m_ventouses", Motif::GripTape => "m_griptape", Motif::AnneauxManche => "m_anneaux",
            Motif::MoletagePointu => "m_moletage_pointu", Motif::PyramidesTri => "m_pyramides_tri", Motif::Epines => "m_epines",
        }
    }
    pub fn famille(self) -> Famille {
        if GRIP.contains(&self) { return Famille::Grip; }
        match self {
            Motif::Losanges | Motif::Stries | Motif::Pyramides | Motif::Grain | Motif::TolerLarmee | Motif::Golf | Motif::Plots
            | Motif::Pneu | Motif::Crampons | Motif::Picots | Motif::RainuresObliques | Motif::Gaufre
            | Motif::Quadrillage | Motif::Pointillage | Motif::Vulcanise | Motif::MicroPointes | Motif::PointesDiamant
            | Motif::Rape | Motif::Ventouses | Motif::GripTape | Motif::AnneauxManche => Famille::Prise,
            Motif::Pierres | Motif::Craquelures | Motif::Bois | Motif::Cuir | Motif::Martele | Motif::Terrasses | Motif::Ecailles
            | Motif::CourbesNiveau | Motif::Ecorce | Motif::Sable | Motif::Crepi | Motif::Beton | Motif::Eponge | Motif::PeauOrange
            | Motif::Bambou | Motif::Bulles => Famille::Nature,
            Motif::Tissage | Motif::Carbone | Motif::Tressage | Motif::Grillage | Motif::Toile | Motif::Jean | Motif::Tricot
            | Motif::Corde | Motif::Mailles | Motif::Capitonne => Famille::Textile,
            Motif::Briques | Motif::Dalles | Motif::Hexagones | Motif::BatonsRompus | Motif::Paves | Motif::Tuiles | Motif::Alveoles
            | Motif::Damier | Motif::Argyle => Famille::Pavage,
            _ => Famille::Geometrique,
        }
    }
    /// Le réglage « densité » a-t-il un effet (motifs à cellules ou à bruit) ?
    pub fn a_densite(self) -> bool {
        matches!(self, Motif::Pierres | Motif::Craquelures | Motif::Grain | Motif::Bois | Motif::Cuir | Motif::Martele | Motif::Terrasses | Motif::Pois | Motif::Stries | Motif::Vagues | Motif::Ondulations
            | Motif::Plots | Motif::Croix | Motif::Etoiles | Motif::Zigzag | Motif::Cercles | Motif::Spirale | Motif::Truchet | Motif::Labyrinthe
            | Motif::CourbesNiveau | Motif::Ecorce | Motif::Sable | Motif::Crepi | Motif::Beton | Motif::Eponge | Motif::PeauOrange | Motif::Oeufs
            | Motif::Picots | Motif::RainuresObliques | Motif::Jean | Motif::Tricot | Motif::Corde | Motif::Bambou | Motif::OndesCroisees
            | Motif::Bulles | Motif::Hachures | Motif::Golf
            | Motif::Quadrillage | Motif::Pointillage | Motif::Vulcanise | Motif::MicroPointes | Motif::PointesDiamant
            | Motif::Rape | Motif::Ventouses | Motif::GripTape | Motif::AnneauxManche | Motif::MoletagePointu | Motif::PyramidesTri | Motif::Epines)
    }
    /// Le réglage « joint » (largeur des creux / traits) a-t-il un effet ?
    pub fn a_joint(self) -> bool {
        !matches!(self, Motif::Vagues | Motif::Ondulations | Motif::Grain | Motif::Bois | Motif::Martele | Motif::Terrasses | Motif::Pyramides
            | Motif::Spirale | Motif::Ecorce | Motif::Sable | Motif::Crepi | Motif::Beton | Motif::PeauOrange | Motif::Oeufs | Motif::OndesCroisees | Motif::Picots
            | Motif::Quadrillage | Motif::Pointillage | Motif::Vulcanise | Motif::MicroPointes | Motif::PointesDiamant | Motif::Rape
            | Motif::Ventouses | Motif::GripTape | Motif::MoletagePointu | Motif::PyramidesTri | Motif::Epines)
    }
}

/// Réglages d'un motif.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    pub motif: Motif,
    pub joint: f32,   // largeur relative des joints / traits (0,02 … 0,45)
    pub arrondi: f32, // 0 = arêtes vives, 1 = bosses très arrondies
    pub densite: u32, // cellules par carreau (motifs à cellules / à bruit / répétitions)
    pub graine: u32,  // variante (motifs aléatoires)
}

impl Default for Params {
    fn default() -> Self { Params { motif: Motif::Briques, joint: 0.12, arrondi: 0.4, densite: 4, graine: 1 } }
}

#[inline] fn fract(x: f32) -> f32 { x - x.floor() }
#[inline] fn sat(x: f32) -> f32 { x.clamp(0.0, 1.0) }
#[inline] fn lisse(a: f32, b: f32, x: f32) -> f32 { let t = sat((x - a) / (b - a).max(1e-6)); t * t * (3.0 - 2.0 * t) }

/// Profil d'un plateau bordé de joints : `d` = distance au bord le plus proche (en carreaux de cellule,
/// 0 au joint, 0,5 au centre) → 0 dans le joint, 1 sur le plateau, chanfrein réglé par l'arrondi.
#[inline]
fn plateau(d: f32, p: &Params) -> f32 {
    let j = p.joint * 0.5;
    let pente = 0.02 + p.arrondi * 0.25;
    lisse(j, j + pente, d)
}

/// Hachage entier → [0, 1) (cellule (x, y) d'une grille repliée sur `n` cellules, variante `g`).
#[inline]
fn hache(x: i32, y: i32, n: i32, g: u32) -> f32 {
    let (x, y) = (x.rem_euclid(n.max(1)) as u32, y.rem_euclid(n.max(1)) as u32);
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ g.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13; h = h.wrapping_mul(0x5bd1_e995); h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Bruit de valeur périodique (période = n cellules sur le carreau).
fn bruit(u: f32, v: f32, n: u32, g: u32) -> f32 {
    let n = n.max(1) as i32;
    let (x, y) = (fract(u) * n as f32, fract(v) * n as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hache(ix, iy, n, g) + (hache(ix + 1, iy, n, g) - hache(ix, iy, n, g)) * sx;
    let b = hache(ix, iy + 1, n, g) + (hache(ix + 1, iy + 1, n, g) - hache(ix, iy + 1, n, g)) * sx;
    a + (b - a) * sy
}

/// Bruit fractal périodique (4 octaves), ∈ [0, 1].
fn fbm(u: f32, v: f32, n: u32, g: u32) -> f32 {
    let (mut s, mut amp, mut tot, mut f) = (0.0, 0.5, 0.0, n.max(1));
    for o in 0..4 { s += amp * bruit(u, v, f, g.wrapping_add(o * 7919)); tot += amp; amp *= 0.5; f *= 2; }
    s / tot
}

/// Cellules de Voronoï périodiques : (distance au germe le plus proche, distance au 2e, valeur de la cellule),
/// distances en cellules.
fn voronoi(u: f32, v: f32, n: u32, g: u32) -> (f32, f32, f32) {
    let n = n.max(1) as i32;
    let (x, y) = (fract(u) * n as f32, fract(v) * n as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (mut d1, mut d2, mut val) = (9.0f32, 9.0f32, 0.0);
    for dy in -1..=1 { for dx in -1..=1 {
        let (cx, cy) = (ix + dx, iy + dy);
        let px = cx as f32 + 0.15 + 0.7 * hache(cx, cy, n, g);
        let py = cy as f32 + 0.15 + 0.7 * hache(cx, cy, n, g ^ 0x9e37);
        let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
        if d < d1 { d2 = d1; d1 = d; val = hache(cx, cy, n, g ^ 0x51ed); } else if d < d2 { d2 = d; }
    } }
    (d1, d2, val)
}

/// Hauteur du motif au point (u, v) (en carreaux), ∈ [0, 1].
pub fn hauteur(p: &Params, u: f32, v: f32) -> f32 {
    let (u, v) = (fract(u), fract(v));
    let n = p.densite.max(1);
    let h = match p.motif {
        Motif::Briques => {
            // 4 rangées de 2 briques, une rangée sur deux décalée d'une demi-brique
            let r = v * 4.0;
            let ry = r.floor();
            let x = fract(u * 2.0 + if ry as i32 % 2 == 1 { 0.5 } else { 0.0 });
            // distances au joint en carreaux (brique = 0,5 × 0,25), ramenées à la hauteur d'une rangée
            let (dx, dy) = (x.min(1.0 - x) * 0.5, fract(r).min(1.0 - fract(r)) * 0.25);
            plateau(dx.min(dy) / 0.25, p)
        }
        Motif::Dalles => {
            let (x, y) = (fract(u * 2.0), fract(v * 2.0));
            plateau(x.min(1.0 - x).min(y.min(1.0 - y)), p)
        }
        Motif::Hexagones => {
            // pavage hexagonal périodique : 2 colonnes × 1 rangée par carreau (rapport √3 compensé)
            // 2 colonnes par carreau ; en hauteur, une période √3 par carreau : raccord exact
            plateau(hex_bord(u * 2.0, v * 1.732_050_8).1, p)
        }
        Motif::Losanges => {
            // moletage croisé : deux familles de stries à ±45° → pyramides en losange
            let a = fract(u * 2.0 + v * 2.0); let b = fract(u * 2.0 - v * 2.0);
            let d = a.min(1.0 - a).min(b.min(1.0 - b));
            plateau(d, p)
        }
        Motif::Stries => {
            let x = fract(u * n as f32);
            plateau(x.min(1.0 - x), p)
        }
        Motif::Vagues => 0.5 + 0.5 * (TAU * (v * n as f32 + 0.18 * (TAU * u).sin())).sin(),
        Motif::Ondulations => {
            let (x, y) = (u - 0.5, v - 0.5);
            let r = (x * x + y * y).sqrt();
            0.5 + 0.5 * (TAU * r * n as f32 * 2.0).cos() * (1.0 - lisse(0.42, 0.5, r))
        }
        Motif::Pois => {
            let (x, y) = (fract(u * n as f32) - 0.5, fract(v * n as f32) - 0.5);
            let r = (x * x + y * y).sqrt();
            let rayon = 0.5 - p.joint;
            let dome = sat(1.0 - (r / rayon.max(0.05)).powi(2)).sqrt();
            if p.arrondi > 0.5 { dome } else { lisse(rayon, rayon - 0.03 - p.arrondi * 0.1, r) }
        }
        Motif::Ecailles => {
            // rangées d'arcs décalées : chaque écaille recouvre la précédente
            let rows = 4.0;
            let r = v * rows; let ry = r.floor();
            let x = fract(u * 4.0 + if ry as i32 % 2 == 1 { 0.5 } else { 0.0 }) - 0.5;
            let y = fract(r);
            let d = (x * x + (y - 1.0).powi(2)).sqrt(); // centre de l'arc en haut de la case
            let bombe = sat(1.0 - d / 0.75);
            bombe.powf(0.6 - 0.4 * p.arrondi) * lisse(0.0, p.joint + 0.02, 0.75 - d)
        }
        Motif::Chevrons => {
            let x = fract(u * 2.0);
            let w = fract(v * 4.0 + (x - 0.5).abs() * 2.0);
            plateau(w.min(1.0 - w), p)
        }
        Motif::Tissage => {
            // vannerie 2 × 2 : brins alternés horizontaux / verticaux
            let (cx, cy) = ((u * 4.0).floor() as i32, (v * 4.0).floor() as i32);
            let (x, y) = (fract(u * 4.0), fract(v * 4.0));
            let horiz = (cx + cy).rem_euclid(2) == 0;
            let (a, b) = if horiz { (y, x) } else { (x, y) };
            let brin = plateau(a.min(1.0 - a), p);
            brin * (0.75 + 0.25 * (PI * b).sin())
        }
        Motif::Carbone => {
            // sergé 2/2 : diagonales de brins tressés
            let (cx, cy) = ((u * 8.0).floor() as i32, (v * 8.0).floor() as i32);
            let (x, y) = (fract(u * 8.0), fract(v * 8.0));
            let horiz = ((cx + cy).rem_euclid(4)) < 2;
            let (a, b) = if horiz { (y, x) } else { (x, y) };
            plateau(a.min(1.0 - a), p) * (0.7 + 0.3 * (PI * b).sin())
        }
        Motif::Pierres => {
            let (d1, d2, val) = voronoi(u, v, n, p.graine);
            let bord = (d2 - d1) * 0.5;
            plateau(bord, p) * (0.75 + 0.25 * val) * (0.85 + 0.15 * fbm(u, v, n * 4, p.graine))
        }
        Motif::Craquelures => {
            let (d1, d2, _) = voronoi(u, v, n, p.graine);
            let fente = (d2 - d1) * 0.5;
            lisse(p.joint * 0.25, p.joint * 0.25 + 0.03 + p.arrondi * 0.08, fente)
        }
        Motif::Grain => fbm(u, v, n * 4, p.graine),
        Motif::Bois => {
            let x = u + 0.08 * fbm(u, v, n, p.graine);
            let anneaux = fract(x * n as f32 * 3.0 + 0.6 * fbm(u * 0.5, v, n, p.graine ^ 7));
            0.55 * anneaux.powf(2.0 + 4.0 * (1.0 - p.arrondi)) + 0.45 * fbm(u, v * 0.25, n * 8, p.graine ^ 3)
        }
        Motif::Cuir => {
            let (d1, d2, _) = voronoi(u, v, n * 3, p.graine);
            let pli = lisse(0.0, 0.08 + p.joint * 0.3, d2 - d1);
            pli * (0.7 + 0.3 * fbm(u, v, n * 6, p.graine))
        }
        Motif::Martele => {
            let (d1, _, val) = voronoi(u, v, n * 2, p.graine);
            let creux = sat(d1 / 0.75);
            1.0 - (1.0 - creux * creux) * (0.7 + 0.3 * val)
        }
        Motif::Pyramides => {
            let (x, y) = (fract(u * n as f32), fract(v * n as f32));
            let d = x.min(1.0 - x).min(y.min(1.0 - y)) * 2.0;
            let k = 0.4 + 0.6 * (1.0 - p.arrondi);
            sat(d).powf(k)
        }
        Motif::Grillage => {
            // fils croisés : relief = fils, le reste en creux
            let (x, y) = (fract(u * 4.0), fract(v * 4.0));
            let fil = |a: f32| 1.0 - plateau(a.min(1.0 - a), p);
            fil(x).max(fil(y))
        }
        Motif::Damier => {
            let (cx, cy) = ((u * 4.0).floor() as i32, (v * 4.0).floor() as i32);
            let (x, y) = (fract(u * 4.0), fract(v * 4.0));
            let plein = (cx + cy).rem_euclid(2) == 0;
            if plein { plateau(x.min(1.0 - x).min(y.min(1.0 - y)), p) } else { 0.0 }
        }
        Motif::Triangles => {
            let (x, y) = (fract(u * 4.0), fract(v * 4.0));
            // chaque case coupée en deux par sa diagonale
            let d = x.min(y).min(((x - y).abs()) * std::f32::consts::FRAC_1_SQRT_2).min(1.0 - x.max(y));
            plateau(d, p)
        }
        Motif::Terrasses => {
            let f = fbm(u, v, n, p.graine);
            let niveaux = 6.0;
            let s = f * niveaux;
            let marche = s.floor() + lisse(0.5 - 0.5 * p.arrondi.max(0.05), 0.5 + 0.5 * p.arrondi.max(0.05), fract(s));
            marche / niveaux
        }
        Motif::Tressage => {
            // tresse : deux brins sinusoïdaux qui se croisent
            let x = fract(u * 2.0);
            let y = v * 2.0;
            let a = 0.5 + 0.35 * (TAU * y).sin();
            let b = 0.5 - 0.35 * (TAU * y).sin();
            let l = 0.12 + p.joint * 0.5;
            let da = (x - a).abs(); let db = (x - b).abs();
            let dessus = (TAU * y).cos() > 0.0; // le brin a passe dessus une demi-période sur deux
            let ha = sat(1.0 - da / l).sqrt() * if dessus { 1.0 } else { 0.7 };
            let hb = sat(1.0 - db / l).sqrt() * if dessus { 0.7 } else { 1.0 };
            ha.max(hb)
        }

        // ───── motifs ajoutés ─────
        Motif::BatonsRompus => {
            // lames à 45° alternées d'une colonne à l'autre (parquet en chevrons)
            let x = u * 4.0;
            let c = x.floor() as i32;
            let fx = fract(x);
            let y = v * 4.0 + if c.rem_euclid(2) == 0 { fx } else { 1.0 - fx };
            let e = fract(y * 2.0);
            plateau((fx.min(1.0 - fx)).min(e.min(1.0 - e) * 0.5) * 1.0, p)
        }
        Motif::TolerLarmee => {
            // tôle larmée : larmes allongées, orientées à ±45° en damier
            let (cx, cy) = ((u * 6.0).floor() as i32, (v * 6.0).floor() as i32);
            let (x, y) = (fract(u * 6.0) - 0.5, fract(v * 6.0) - 0.5);
            let s = if (cx + cy).rem_euclid(2) == 0 { 1.0 } else { -1.0 };
            let (a, b) = ((x + s * y) * 0.707, (x - s * y) * 0.707);
            let d = (a / 0.40).powi(2) + (b / (0.08 + p.joint * 0.2)).powi(2);
            sat(1.0 - d).powf(0.5 - 0.3 * p.arrondi.min(1.0) + 0.3)
        }
        Motif::Golf => {
            // alvéoles de balle de golf sur un pavage hexagonal
            let (x, y) = (u * n as f32, v * n as f32 * 1.732_050_8);
            let r = hex_centre(x, y);
            let creux = sat(1.0 - (r / 0.46).powi(2)).sqrt();
            1.0 - creux * (0.7 + 0.3 * p.arrondi)
        }
        Motif::Plots => {
            let (x, y) = (fract(u * n as f32) - 0.5, fract(v * n as f32) - 0.5);
            let r = (x * x + y * y).sqrt();
            let rayon = 0.5 - p.joint - 0.1;
            lisse(rayon + 0.02 + p.arrondi * 0.08, rayon, r)
        }
        Motif::Croix => {
            let (x, y) = ((fract(u * n as f32) - 0.5).abs(), (fract(v * n as f32) - 0.5).abs());
            let bras = 0.06 + p.joint * 0.4;
            let d = (bras - x.min(y)).min(0.38 - x.max(y));
            lisse(0.0, 0.02 + p.arrondi * 0.06, d)
        }
        Motif::Etoiles => {
            let (x, y) = (fract(u * n as f32) - 0.5, fract(v * n as f32) - 0.5);
            let (r, a) = ((x * x + y * y).sqrt(), y.atan2(x) + PI / 2.0);
            let f = 0.38 * (0.55 + 0.45 * (5.0 * a).cos().abs().powf(0.6 + p.joint * 2.0));
            lisse(f + 0.01 + p.arrondi * 0.05, f - 0.01, r)
        }
        Motif::Capitonne => {
            // coussins en losange, boutons aux croisements
            let (a, b) = (fract((u + v) * 3.0), fract((u - v) * 3.0));
            let d = a.min(1.0 - a).min(b.min(1.0 - b)) * 2.0;
            let coussin = sat(d).powf(0.35 + 0.4 * (1.0 - p.arrondi));
            let (bx, by) = (a.min(1.0 - a), b.min(1.0 - b));
            let bouton = sat(1.0 - (bx * bx + by * by).sqrt() / 0.07);
            (coussin - 0.5 * bouton).max(0.0)
        }
        Motif::Grecque => {
            // frise grecque : grille de 8 × 8 cases (1 = relief), adoucie au bord des cases
            const G: [&str; 8] = ["11111111", "10000001", "10111101", "10100101", "10101101", "10100001", "10111111", "10000000"];
            let val = |i: i32, j: i32| -> f32 { if G[j.rem_euclid(8) as usize].as_bytes()[i.rem_euclid(8) as usize] == b'1' { 1.0 } else { 0.0 } };
            let (x, y) = (u * 8.0, (1.0 - v) * 8.0);
            let (i, j) = (x.floor() as i32, y.floor() as i32);
            let (fx, fy) = (fract(x), fract(y));
            let k = 0.05 + p.arrondi * 0.25;
            let sx = if fx < 0.5 { lisse(-k, k, fx) * 0.5 + 0.5 } else { 1.0 - (lisse(-k, k, 1.0 - fx) * 0.5 + 0.5) };
            let sy = if fy < 0.5 { lisse(-k, k, fy) * 0.5 + 0.5 } else { 1.0 - (lisse(-k, k, 1.0 - fy) * 0.5 + 0.5) };
            let (ox, oy) = (if fx < 0.5 { i - 1 } else { i + 1 }, if fy < 0.5 { j - 1 } else { j + 1 });
            let a = val(i, j) * sx + val(ox, j) * (1.0 - sx);
            let b = val(i, oy) * sx + val(ox, oy) * (1.0 - sx);
            a * sy + b * (1.0 - sy)
        }
        Motif::Zigzag => {
            let t = fract(u * 2.0);
            let w = fract(v * n as f32 + (2.0 * t - 1.0).abs() * 0.5);
            plateau(w.min(1.0 - w), p)
        }
        Motif::Cercles => {
            let (x, y) = (fract(u * n as f32) - 0.5, fract(v * n as f32) - 0.5);
            let r = (x * x + y * y).sqrt();
            if r > 0.5 { 0.0 } else { 0.5 + 0.5 * (TAU * r * 4.0).cos() }
        }
        Motif::Spirale => {
            let (x, y) = (u - 0.5, v - 0.5);
            let (r, a) = ((x * x + y * y).sqrt(), y.atan2(x));
            (0.5 + 0.5 * (TAU * (r * n as f32 * 2.0) - a).cos()) * (1.0 - lisse(0.42, 0.5, r))
        }
        Motif::Truchet => {
            let (cx, cy) = ((u * n as f32).floor() as i32, (v * n as f32).floor() as i32);
            let (mut x, y) = (fract(u * n as f32), fract(v * n as f32));
            if hache(cx, cy, n as i32, p.graine) < 0.5 { x = 1.0 - x; }
            let d = ((x * x + y * y).sqrt() - 0.5).abs().min((((1.0 - x).powi(2) + (1.0 - y).powi(2)).sqrt() - 0.5).abs());
            1.0 - lisse(0.06 + p.joint * 0.3, 0.06 + p.joint * 0.3 + 0.03 + p.arrondi * 0.08, d)
        }
        Motif::Labyrinthe => {
            let (cx, cy) = ((u * n as f32).floor() as i32, (v * n as f32).floor() as i32);
            let (x, y) = (fract(u * n as f32), fract(v * n as f32));
            let d = if hache(cx, cy, n as i32, p.graine) < 0.5 { (x - y).abs() } else { (x + y - 1.0).abs() } * std::f32::consts::FRAC_1_SQRT_2;
            1.0 - lisse(0.05 + p.joint * 0.25, 0.05 + p.joint * 0.25 + 0.03 + p.arrondi * 0.06, d)
        }
        Motif::CourbesNiveau => {
            let f = fbm(u, v, n, p.graine) * 10.0;
            let e = fract(f);
            1.0 - lisse(0.04 + p.joint * 0.3, 0.1 + p.joint * 0.3 + p.arrondi * 0.1, e.min(1.0 - e))
        }
        Motif::Ecorce => {
            let (d1, d2, _) = voronoi(u * 1.0, v * 1.0, n.max(2), p.graine);
            let fibres = fbm(u * 1.0, v, n * 6, p.graine ^ 5);
            let fente = lisse(0.0, 0.12, d2 - d1);
            0.25 + 0.45 * fibres + 0.3 * fente * (0.7 + 0.3 * fbm(u, v, n * 2, p.graine ^ 9))
        }
        Motif::Sable => fbm(u, v, n * 16, p.graine),
        Motif::Crepi => {
            let r = 1.0 - (2.0 * bruit(u, v, n * 8, p.graine) - 1.0).abs();
            0.6 * r * r + 0.4 * fbm(u, v, n * 4, p.graine ^ 3)
        }
        Motif::Beton => {
            let (d1, _, val) = voronoi(u, v, n * 6, p.graine);
            let pore = if val < 0.3 { 1.0 - sat(d1 / 0.18) } else { 0.0 };
            (0.75 + 0.25 * fbm(u, v, n * 8, p.graine ^ 1) - 0.7 * pore).max(0.0)
        }
        Motif::Eponge => {
            let (d1, _, val) = voronoi(u, v, n * 3, p.graine);
            lisse(0.12 + 0.2 * val, 0.32 + 0.2 * val, d1)
        }
        Motif::PeauOrange => {
            let (d1, _, _) = voronoi(u, v, n * 5, p.graine);
            0.5 + 0.5 * (1.0 - sat(d1 / 0.6)).powf(0.6)
        }
        Motif::Alveoles => 1.0 - plateau(hex_bord(u * 2.0, v * 1.732_050_8).1, p),
        Motif::Oeufs => 0.5 + 0.25 * ((TAU * u * n as f32).sin() + (TAU * v * n as f32).sin()),
        Motif::Gaufre => {
            let (x, y) = (fract(u * 4.0), fract(v * 4.0));
            let d = x.min(1.0 - x).min(y.min(1.0 - y));
            1.0 - lisse(0.08 + p.joint * 0.4, 0.08 + p.joint * 0.4 + 0.04 + p.arrondi * 0.15, d)
        }
        Motif::Pneu => {
            // bande de roulement : pavés en chevrons, rainure centrale
            let x = fract(u * 2.0);
            let w = fract(v * 4.0 + (x - 0.5).abs() * 2.0);
            plateau(w.min(1.0 - w), p) * lisse(0.03, 0.03 + 0.04 + p.arrondi * 0.06, (x - 0.5).abs())
        }
        Motif::Crampons => {
            let r = v * 4.0;
            let ry = r.floor() as i32;
            let x = fract(u * 2.0 + if ry.rem_euclid(2) == 1 { 0.25 } else { 0.0 });
            let y = fract(r);
            let (dx, dy) = (0.3 - (x - 0.5).abs(), 0.32 - (y - 0.5).abs());
            lisse(0.0, 0.03 + p.arrondi * 0.08, dx.min(dy))
        }
        Motif::Picots => {
            let (x, y) = (fract(u * n as f32) - 0.5, fract(v * n as f32) - 0.5);
            sat(1.0 - (x * x + y * y).sqrt() / 0.45)
        }
        Motif::RainuresObliques => {
            let w = fract((u + v) * n as f32);
            plateau(w.min(1.0 - w), p)
        }
        Motif::Toile => {
            let (cx, cy) = ((u * 8.0).floor() as i32, (v * 8.0).floor() as i32);
            let (x, y) = (fract(u * 8.0), fract(v * 8.0));
            let (a, b) = if (cx + cy).rem_euclid(2) == 0 { (y, x) } else { (x, y) };
            plateau(a.min(1.0 - a), p) * (0.8 + 0.2 * (PI * b).sin())
        }
        Motif::Jean => {
            let w = fract((u * 2.0 + v) * n as f32 * 2.0);
            let cote = 0.5 + 0.5 * (PI * w).sin();
            cote * (0.85 + 0.15 * fbm(u, v, n * 8, p.graine))
        }
        Motif::Tricot => {
            let (x, y) = (fract(u * n as f32), v * n as f32 * 1.5);
            let w = fract(y + (x - 0.5).abs() * 1.2);
            sat((PI * w).sin()) * lisse(0.0, 0.08, (x - 0.5).abs())
        }
        Motif::Corde => {
            let x = fract(u * n as f32);
            let w = fract(x * 3.0 + v * n as f32 * 2.0);
            let brin = (PI * w).sin().max(0.0).powf(0.7);
            brin * (1.0 - (2.0 * x - 1.0).powi(8))
        }
        Motif::Mailles => {
            // anneaux entrelacés : distance à l'anneau le plus proche (centres décalés une rangée sur deux)
            let (x, y) = (u * 4.0, v * 4.0);
            let mut d = f32::MAX;
            for (ox, oy) in [(0.0f32, 0.0f32), (0.5, 0.5)] {
                let (cx, cy) = ((x - ox).round() + ox, (y - oy).round() + oy);
                let r = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                d = d.min((r - 0.42).abs());
            }
            1.0 - lisse(0.05 + p.joint * 0.2, 0.05 + p.joint * 0.2 + 0.03 + p.arrondi * 0.06, d)
        }
        Motif::Bambou => {
            let c = (u * n as f32).floor() as i32;
            let x = fract(u * n as f32);
            let tige = sat(1.0 - (2.0 * x - 1.0).powi(2)).sqrt();
            let y = fract(v * 2.0 + hache(c, 0, n as i32, p.graine));
            let noeud = 1.0 - 0.25 * lisse(0.06, 0.0, y.min(1.0 - y)) + 0.1 * lisse(0.03, 0.0, (y - 0.03).abs());
            tige * noeud.min(1.0)
        }
        Motif::Paves => {
            let r = v * 5.0;
            let ry = r.floor() as i32;
            let xo = u * 3.0 + if ry.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
            let (cx, x, y) = (xo.floor() as i32, fract(xo), fract(r));
            let d = (x.min(1.0 - x) * (5.0 / 3.0)).min(y.min(1.0 - y)) * 0.5;
            plateau(d, p) * (0.8 + 0.2 * hache(cx, ry, 3, p.graine))
        }
        Motif::Tuiles => {
            let r = v * 4.0;
            let ry = r.floor() as i32;
            let x = fract(u * 3.0 + if ry.rem_euclid(2) == 1 { 0.5 } else { 0.0 });
            let y = fract(r);
            (0.25 + 0.75 * y.powf(0.8 - 0.5 * p.arrondi)) * plateau(x.min(1.0 - x) * 0.5, p)
        }
        Motif::ArtDeco => {
            let r = v * 4.0;
            let ry = r.floor() as i32;
            let x = fract(u * 4.0 + if ry.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) - 0.5;
            let y = fract(r);
            let d = (x * x + y * y).sqrt();
            if d < 0.75 { 0.5 + 0.5 * (TAU * d * 4.0).cos() } else { 0.0 }
        }
        Motif::Argyle => {
            let (a, b) = ((u + v) * 2.0, (u - v) * 2.0);
            let (ca, cb) = (a.floor() as i32, b.floor() as i32);
            let (fa, fb) = (fract(a), fract(b));
            let d = fa.min(1.0 - fa).min(fb.min(1.0 - fb));
            plateau(d, p) * if (ca + cb).rem_euclid(2) == 0 { 1.0 } else { 0.55 }
        }
        Motif::OndesCroisees => 0.5 + 0.25 * ((TAU * (u + v) * n as f32).sin() + (TAU * (u - v) * n as f32).sin()),
        Motif::Bulles => {
            let (d1, _, val) = voronoi(u, v, n * 2, p.graine);
            let rb = 0.15 + 0.3 * val;
            sat(1.0 - (d1 / rb).powi(2)).sqrt()
        }
        Motif::Hachures => {
            let (a, b) = (fract((u + v) * n as f32 * 2.0), fract((u - v) * n as f32 * 2.0));
            let d = a.min(1.0 - a).min(b.min(1.0 - b));
            1.0 - lisse(0.04 + p.joint * 0.2, 0.04 + p.joint * 0.2 + 0.03 + p.arrondi * 0.06, d)
        }

        // ───── prise : poignées, crosses, manches (micro-pointes pour la peau ou les gants) ─────
        Motif::Quadrillage => {
            // quadrillage d'arme : pyramides à pointe vive sur une grille en losanges ; l'arrondi émousse les pointes
            let (a, b) = (fract((u + v) * n as f32 * 2.0) - 0.5, fract((u - v) * n as f32 * 2.0) - 0.5);
            let h = sat(1.0 - 2.0 * a.abs().max(b.abs()));
            let plafond = 1.0 - p.arrondi * 0.35;
            h.min(plafond) / plafond
        }
        Motif::Pointillage => {
            // pointillage : cratères serrés et irréguliers, pointes entre eux
            let (d1, d2, val) = voronoi(u, v, n * 6, p.graine);
            let crat = if val < 0.65 { sat(1.0 - d1 / (0.25 + 0.15 * val)).powi(2) } else { 0.0 };
            let arete = 1.0 - lisse(0.0, 0.12 + p.arrondi * 0.2, d2 - d1);
            (0.55 + 0.3 * arete + 0.15 * fbm(u, v, n * 12, p.graine ^ 7) - 0.55 * crat).clamp(0.0, 1.0)
        }
        Motif::Vulcanise => {
            // caoutchouc vulcanisé : galets arrondis serrés de tailles variées
            let (d1, d2, val) = voronoi(u, v, n * 4, p.graine);
            let galet = sat(1.0 - (d1 / (0.45 + 0.15 * val)).powi(2)).sqrt();
            galet * lisse(0.0, 0.04 + p.joint * 0.3, d2 - d1) * (0.8 + 0.2 * val)
        }
        Motif::MicroPointes => {
            // micro-pointes : cônes pointus très denses en quinconce
            let r = hex_centre(u * n as f32 * 2.0, v * n as f32 * 2.0 * 1.732_050_8);
            sat(1.0 - r / 0.5).powf(1.6 - 1.2 * p.arrondi)
        }
        Motif::PointesDiamant => {
            let r = v * n as f32 * 2.0;
            let ry = r.floor() as i32;
            let x = fract(u * n as f32 * 2.0 + if ry.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) - 0.5;
            let y = fract(r) - 0.5;
            let h = sat(1.0 - 2.0 * x.abs().max(y.abs()));
            let plafond = 1.0 - p.arrondi * 0.35;
            h.min(plafond) / plafond
        }
        Motif::Rape => {
            // râpe : dents décalées, montée douce et chute franche (prise dans un sens)
            let r = v * n as f32 * 2.0;
            let ry = r.floor() as i32;
            let x = fract(u * n as f32 * 3.0 + if ry.rem_euclid(2) == 1 { 0.5 } else { 0.0 });
            let y = fract(r);
            y.powf(1.2 + p.arrondi) * sat(1.0 - (x - 0.5).abs() / (0.3 + p.joint * 0.4))
        }
        Motif::Ventouses => {
            // ventouses : petits cratères bordés d'un bourrelet
            let r = hex_centre(u * n as f32, v * n as f32 * 1.732_050_8);
            let creux = sat(1.0 - (r / 0.3).powi(2));
            let bord = (-((r - 0.34) / (0.05 + p.arrondi * 0.04)).powi(2)).exp();
            (0.55 - 0.5 * creux + 0.45 * bord).clamp(0.0, 1.0)
        }
        Motif::GripTape => {
            // ruban antidérapant : grains vifs, denses et aléatoires
            let (d1, _, val) = voronoi(u, v, n * 10, p.graine);
            let grain = sat(1.0 - d1 / (0.35 + 0.25 * val)).powf(2.5 - 1.5 * p.arrondi);
            0.15 + 0.85 * grain * (0.6 + 0.4 * val)
        }
        Motif::AnneauxManche => {
            // anneaux de manche (projection cylindre conseillée) : nervures coiffées d'un micro-quadrillage
            let w = fract(v * n as f32);
            let nervure = plateau(w.min(1.0 - w), p);
            let (a, b) = (fract((u + v) * n as f32 * 8.0) - 0.5, fract((u - v) * n as f32 * 8.0) - 0.5);
            let micro = sat(1.0 - 2.0 * a.abs().max(b.abs()));
            nervure * (0.8 + 0.2 * micro)
        }

        Motif::MoletagePointu => {
            // moletage pointu : pyramides carrées serrées à pointe vive (dans l'axe)
            let (a, b) = (fract(u * n as f32 * 3.0) - 0.5, fract(v * n as f32 * 3.0) - 0.5);
            sat(1.0 - 2.0 * a.abs().max(b.abs())).powf(1.3 - 0.8 * p.arrondi)
        }
        Motif::PyramidesTri => {
            // pyramides à 3 faces sur un réseau triangulaire (raccord exact : √3 par carreau en hauteur)
            let (x, y) = (u * n as f32 * 2.0, v * n as f32 * 1.732_050_8);
            let (s, t) = (x - y / 1.732_050_8, 2.0 * y / 1.732_050_8);
            let (fs, ft) = (fract(s), fract(t));
            let h = if fs + ft < 1.0 { 3.0 * fs.min(ft).min(1.0 - fs - ft) } else { 3.0 * (1.0 - fs).min(1.0 - ft).min(fs + ft - 1.0) };
            sat(h).powf(1.2 - 0.7 * p.arrondi)
        }
        Motif::Epines => {
            // épines : cônes pointus penchés dans un sens (accroche dans la direction de la prise)
            let r = v * n as f32 * 2.0;
            let ry = r.floor() as i32;
            let x = fract(u * n as f32 * 2.0 + if ry.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) - 0.5;
            let y = fract(r) - 0.35;
            let d = (x * x + (y * if y > 0.0 { 2.2 } else { 1.0 }).powi(2)).sqrt();
            sat(1.0 - d / 0.45).powf(1.8 - 1.2 * p.arrondi)
        }
    };
    sat(h)
}

/// Distance au centre de cellule le plus proche du pavage hexagonal de `hex_bord` (même réseau).
fn hex_centre(x: f32, y: f32) -> f32 {
    let s = 1.732_050_8f32;
    let (ax, ay) = (x - x.round(), y - (y / s).round() * s);
    let (bx, by) = (x - (x - 0.5).round() - 0.5, y - ((y - s * 0.5) / s).round() * s - s * 0.5);
    (ax * ax + ay * ay).min(bx * bx + by * by).sqrt()
}

/// Pavage hexagonal (rangées décalées) : (indice du jeu de centres, distance au bord de la cellule 0..0,5).
fn hex_bord(x: f32, y: f32) -> (i32, f32) {
    // centres sur une grille rectangulaire 1 × √3 avec un second jeu décalé de (0,5 ; √3/2)
    let s = 1.732_050_8f32;
    let (ax, ay) = (x - (x).round(), y - (y / s).round() * s);
    let (bx, by) = (x - (x - 0.5).round() - 0.5, y - ((y - s * 0.5) / s).round() * s - s * 0.5);
    let (px, py, id) = if ax * ax + ay * ay < bx * bx + by * by { (ax, ay, 0) } else { (bx, by, 1) };
    // distance au bord de l'hexagone (rayon intérieur 0,5) : max des projections sur les 3 directions
    let (px, py) = (px.abs(), py.abs());
    let d = px.max(px * 0.5 + py * 0.866_025_4);
    (id, (0.5 - d).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tous les motifs : valeurs dans [0, 1], qui varient, et raccord sans couture au bord du carreau.
    #[test]
    fn motifs_periodiques_et_bornes() {
        for m in TOUS {
            let p = Params { motif: m, ..Default::default() };
            let (mut lo, mut hi) = (1.0f32, 0.0f32);
            for i in 0..64 { for j in 0..64 {
                let h = hauteur(&p, i as f32 / 64.0, j as f32 / 64.0);
                assert!((0.0..=1.0).contains(&h), "{m:?} hors bornes");
                lo = lo.min(h); hi = hi.max(h);
            } }
            assert!(hi - lo > 0.3, "{m:?} presque plat ({lo}..{hi})");
            // raccord : le saut à travers le bord du carreau ne dépasse pas les sauts présents à l'intérieur
            // du motif (les écailles, par exemple, ont des marches voulues entre rangées)
            let e = 1e-4;
            let (mut saut, mut interne) = (0.0f32, 0.0f32);
            for k in 0..200 {
                let t = k as f32 / 200.0 + 0.0013;
                saut = saut.max((hauteur(&p, 1.0 - e, t) - hauteur(&p, e, t)).abs());
                saut = saut.max((hauteur(&p, t, 1.0 - e) - hauteur(&p, t, e)).abs());
                for j in 1..16 {
                    let c = j as f32 / 16.0; // lignes intérieures (dont les limites de rangées et de cellules)
                    interne = interne.max((hauteur(&p, c - e, t) - hauteur(&p, c + e, t)).abs());
                    interne = interne.max((hauteur(&p, t, c - e) - hauteur(&p, t, c + e)).abs());
                }
            }
            assert!(saut <= interne.max(0.05) + 0.05, "{m:?} : couture au bord du carreau ({saut} > {interne})");
        }
    }
}
