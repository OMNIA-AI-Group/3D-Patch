//! MATIÈRE — moteur de textures en relief de 3D-Patch (écrit pour 3D-Patch, aucun code repris).
//!
//! Étapes : soudure du maillage → subdivision ADAPTATIVE (une arête n'est coupée que si le relief
//! varie le long de l'arête plus que la tolérance, ou si elle dépasse 4 × la résolution) → déplacement
//! des sommets le long de la normale lissée → réduction par quadriques seulement si le plafond de
//! triangles est dépassé. Tout est parallèle (rayon) et le résultat reste étanche.

pub mod maillage;
pub mod motifs;
pub mod projection;
pub mod reduction;
pub mod source;

use maillage::{add, long, mul, sub, Maillage, V};
use rayon::prelude::*;

/// Sens du relief par rapport à la surface d'origine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sens { Sortant, Rentrant, Centre }

/// Mélange d'un calque avec ceux du dessous.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Melange { Ajouter, Remplacer }

/// Un calque : un relief, sa projection, sa profondeur.
#[derive(Clone)]
pub struct Calque {
    pub relief: source::Relief,
    pub proj: projection::Params,
    pub profondeur: f32, // mm entre creux et sommet
    pub sens: Sens,
    pub melange: Melange,
    pub opacite: f32, // 0..1
    pub visible: bool,
}

/// Réglages communs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reglages {
    pub resolution: f32,      // plus petite arête produite (mm) : finesse du détail
    pub tolerance: f32,       // écart toléré entre relief voulu et relief obtenu (mm)
    pub max_triangles: usize, // plafond du résultat (réduction au-delà)
    pub adoucir: f32,         // largeur du fondu au bord des zones masquées (mm), 0 = net
    pub plancher: Option<f32>, // Z minimal du résultat (plateau d'impression) : rien ne passe dessous
}

impl Default for Reglages {
    fn default() -> Self { Reglages { resolution: 0.25, tolerance: 0.02, max_triangles: 1_500_000, adoucir: 0.0, plancher: None } }
}

/// Résolution conseillée pour un objet de diagonale `diag` mm et un carreau de `taille` mm.
pub fn resolution_conseillee(diag: f32, taille: f32) -> f32 {
    (taille / 40.0).min(diag / 250.0).clamp(0.05, 5.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode { Apercu, Final }

pub struct Resultat {
    pub verts: Vec<f32>,
    pub avant: usize,
    pub subdivises: usize,
    pub apres: usize,
    pub etapes: Vec<(&'static str, f32)>, // durées (s)
    /// Face d'origine de chaque triangle du résultat (vide si la réduction a eu lieu).
    pub src: Vec<u32>,
}

/// Calques préparés pour la lecture rapide.
struct Prepare<'a> { calques: Vec<(&'a Calque, projection::Projection, f32)> }

impl<'a> Prepare<'a> {
    fn new(calques: &'a [Calque]) -> Self {
        Prepare { calques: calques.iter().map(|c| {
            let base = match c.sens { Sens::Sortant => 0.0, Sens::Rentrant => 1.0, Sens::Centre => 0.5 };
            (c, projection::Projection::new(c.proj), base)
        }).collect() }
    }
    /// Déplacement (mm, le long de la normale) au point p, normale n, avec le poids de chaque calque.
    fn deplacement(&self, p: V, n: V, poids: &dyn Fn(usize) -> f32) -> f32 {
        let mut d = 0.0;
        for (l, (c, pr, base)) in self.calques.iter().enumerate() {
            let w = poids(l) * c.opacite;
            if !c.visible || w <= 0.0 { continue; }
            let h = pr.hauteur(p, n, &|u, v| c.relief.lire(u, v));
            let val = c.profondeur * (h - base);
            match c.melange { Melange::Ajouter => d += val * w, Melange::Remplacer => d += (val - d) * w }
        }
        d
    }
}

/// Applique les calques à l'objet `verts` (soupe : 18 flottants par triangle).
/// `faces[i]` = calques actifs sur le triangle i (bit l = calque l) ; 0 = triangle laissé intact.
pub fn calculer(verts: &[f32], calques: &[Calque], faces: &[u32], r: &Reglages, mode: Mode) -> Result<Resultat, String> {
    if verts.len() < 18 { return Err("objet vide".into()); }
    if calques.iter().all(|c| !c.visible || c.profondeur.abs() < 1e-6) { return Err("aucun calque actif".into()); }
    let t0 = std::time::Instant::now();
    let mut etapes = Vec::new();
    let mut lap = |nom: &'static str| etapes.push((nom, t0.elapsed().as_secs_f32()));
    let mut m = maillage::souder(verts);
    let avant = m.t.len();
    // triangles en aiguille (exports CAO) retournés : le découpage n'y créera pas de points confondus
    let ext0 = (0..3).map(|d| m.p.iter().map(|q| q[d]).fold(f32::MIN, f32::max) - m.p.iter().map(|q| q[d]).fold(f32::MAX, f32::min)).fold(0.0f32, f32::max);
    let flips = maillage::aiguilles(&mut m, ext0 * 2e-5 * 6.0);
    if std::env::var_os("MAT_TRACE").is_some() { eprintln!("  aiguilles retournées : {flips}"); }
    lap("soudure");
    let (res, tol, plafond) = match mode {
        Mode::Apercu => (r.resolution * 2.0, r.tolerance * 2.0, 1_200_000usize),
        Mode::Final => (r.resolution, r.tolerance, (r.max_triangles * 4).max(2_000_000).min(12_000_000)),
    };
    let bits_face = |f: u32| faces.get(f as usize).copied().unwrap_or(0);
    // motif trop fin pour le plafond : plus petit détail relevé d'emblée, pour un résultat uniforme sur toute
    // la pièce (sinon le découpage s'arrêterait au plafond, fini par endroits seulement)
    let aire: f32 = m.t.iter().zip(&m.src).filter(|(_, &f)| bits_face(f) != 0).map(|(t, _)| {
        let (a, b, c) = (m.p[t[0] as usize], m.p[t[1] as usize], m.p[t[2] as usize]);
        long(maillage::cross(sub(b, a), sub(c, a))) * 0.5
    }).sum();
    let res = res.max((aire / (0.433 * plafond as f32 * 0.6)).sqrt());
    // garde-fou contre les détails manqués : aucune arête plus longue que 1/24 du plus petit carreau
    // (ses 3 points de contrôle lisent alors le motif tous les 1/96 de carreau), ni que 4 × la résolution
    // si elle est plus grossière
    let carreau = calques.iter().filter(|c| c.visible).map(|c| c.proj.taille_u.min(c.proj.taille_v)).fold(f32::MAX, f32::min);
    let lmax = (carreau / 24.0).max(res * 4.0);
    let prep = Prepare::new(calques);
    // subdivision adaptative, en deux temps : (1) arêtes plus longues que le garde-fou, partout, jusqu'à un
    // maillage régulier ; (2) seulement ensuite, découpe selon l'écart de relief, dans la limite du plafond.
    // Sans le premier temps, un motif plus fin que le détail permis ferait couper toute la pièce à chaque
    // passe : le plafond serait atteint avant que les grands triangles soient découpés (éventails).
    let geometrie_seule = std::sync::atomic::AtomicBool::new(true);
    let couper = |m: &Maillage, ar: &maillage::Aretes, e: usize| -> bool {
        let (a, b) = (ar.ab[e].0 as usize, ar.ab[e].1 as usize);
        // calques actifs autour de l'arête ; une arête entièrement en zone intacte n'a pas besoin d'être coupée.
        // Au bord d'une zone intacte, elle est coupée comme les autres : le triangle intact est redécoupé
        // (sans bouger, il reste à plat), sinon le triangle texturé voisin s'effilerait à l'infini.
        let mut bits = 0u32;
        for s in &ar.occ[ar.debut[e] as usize..ar.debut[e + 1] as usize] { bits |= bits_face(m.src[*s as usize / 3]); }
        if bits == 0 { return false; }
        let l = long(sub(m.p[b], m.p[a]));
        if l > lmax { return true; }
        if l <= res || geometrie_seule.load(std::sync::atomic::Ordering::Relaxed) { return false; }
        let poids = |k: usize| if bits & (1 << k) != 0 { 1.0 } else { 0.0 };
        let (da, db) = (prep.deplacement(m.p[a], m.n[a], &poids), prep.deplacement(m.p[b], m.n[b], &poids));
        // relief lu en 3 points de l'arête comparé à la droite entre ses extrémités
        [0.25f32, 0.5, 0.75].iter().any(|&s| {
            let p = add(mul(m.p[a], 1.0 - s), mul(m.p[b], s));
            let n = maillage::norm(add(mul(m.n[a], 1.0 - s), mul(m.n[b], s)));
            (prep.deplacement(p, n, &poids) - (da + (db - da) * s)).abs() > tol
        })
    };
    let trace = std::env::var_os("MAT_TRACE").is_some();
    for _ in 0..64 {
        if m.t.len() >= plafond * 3 / 2 { break; }
        let n = maillage::passe(&mut m, &couper);
        if trace { eprintln!("  passe (garde-fou) : {n} arêtes coupées → {} triangles", m.t.len()); }
        if n == 0 { break; }
    }
    geometrie_seule.store(false, std::sync::atomic::Ordering::Relaxed);
    m.actif.iter_mut().for_each(|x| *x = true); // toutes les arêtes sont à juger selon le relief
    for _ in 0..64 { // sécurité : chaque passe divise par 2 les arêtes coupées, 64 passes suffisent toujours
        if m.t.len() >= plafond { break; }
        let n = maillage::passe(&mut m, &couper);
        if trace { eprintln!("  passe : {n} arêtes coupées → {} triangles ({:.2} s)", m.t.len(), t0.elapsed().as_secs_f32()); }
        if n == 0 { break; }
        // queue de découpage : quelques arêtes isolées sur une marche franche du motif ne descendent jamais
        // sous la tolérance ; passé le gros du travail, une passe qui coupe moins d'une arête sur 10 000
        // termine la subdivision (chaque passe coûte un parcours complet du maillage)
        if n * 10_000 < m.t.len() * 3 / 2 { break; }
    }
    let subdivises = m.t.len();
    lap("subdivision");
    // poids de chaque calque par sommet (part des triangles voisins où il est actif) ; 0 au contact d'une zone intacte
    let nl = calques.len().min(32);
    let mut compte = vec![0u16; m.p.len()];
    let mut actif = vec![0u16; m.p.len() * nl.max(1)];
    let mut gele = vec![false; m.p.len()];
    for (f, tr) in m.t.iter().enumerate() {
        let b = bits_face(m.src[f]);
        for &v in tr {
            let v = v as usize;
            if b == 0 { gele[v] = true; }
            compte[v] = compte[v].saturating_add(1);
            for l in 0..nl { if b & (1 << l) != 0 { actif[v * nl + l] = actif[v * nl + l].saturating_add(1); } }
        }
    }
    let fondu = fondu_bords(&m, &gele, bits_face, if mode == Mode::Apercu { r.adoucir } else { r.adoucir });
    let dep: Vec<V> = (0..m.p.len()).into_par_iter().map(|v| {
        if gele[v] || compte[v] == 0 { return m.p[v]; }
        let poids = |l: usize| if l < nl { actif[v * nl + l] as f32 / compte[v] as f32 } else { 0.0 };
        let d = prep.deplacement(m.p[v], m.n[v], &poids) * fondu.as_ref().map_or(1.0, |f| f[v]);
        add(m.p[v], mul(m.n[v], d))
    }).collect();
    lap("déplacement");
    // anti-repli : là où le relief retourne un triangle (creux serrés, parois minces), le déplacement
    // de ses sommets est ramené vers la moyenne de leurs voisins, jusqu'à ce qu'il n'en reste aucun
    let mut dep = dep;
    // plateau : aucun point sous le plancher (bords arrondis du dessous dont le relief pousserait vers le bas)
    if let Some(z0) = r.plancher { dep.par_iter_mut().for_each(|q| if q[2] < z0 { q[2] = z0; }); }
    let replis = deplier(&m, &mut dep, &gele);
    lap("anti-repli");
    if std::env::var_os("MAT_TRACE").is_some() { eprintln!("  replis corrigés : {replis:?}"); }
    let mut p = dep;
    let mut t = std::mem::take(&mut m.t);
    let mut src = std::mem::take(&mut m.src);
    if mode == Mode::Final && t.len() > r.max_triangles {
        reduction::reduire(&mut p, &mut t, r.max_triangles, &[]);
        src = vec![u32::MAX; t.len()];
        lap("réduction");
    }
    // points devenus quasi confondus (triangles en aiguille du fichier d'origine) : fusionnés au seuil de
    // soudure de 3D-Patch, sinon l'analyse des défauts les verrait comme une surface qui se pince
    let ext = (0..3).map(|d| p.iter().map(|q| q[d]).fold(f32::MIN, f32::max) - p.iter().map(|q| q[d]).fold(f32::MAX, f32::min)).fold(0.0f32, f32::max);
    reduction::nettoyer(&mut p, &mut t, &mut src, ext * 2e-5 * 3.0);
    lap("soudure finale");

    let verts = maillage::soupe(&p, &t);
    lap("assemblage");
    let apres = t.len();
    if src.first() == Some(&u32::MAX) { src.clear(); }
    Ok(Resultat { verts, avant, subdivises, apres, etapes, src })
}

/// Corrige les triangles retournés par le déplacement : leurs sommets reçoivent d'abord un déplacement
/// plus doux (moyenne des voisins), puis, s'ils restent retournés, un relief réduit de moitié à chaque
/// passe — au pire la surface d'origine, jamais retournée : la correction aboutit toujours.
/// Renvoie le nombre de triangles retournés à chaque passe.
fn deplier(m: &Maillage, dep: &mut Vec<V>, gele: &[bool]) -> Vec<usize> {
    let retourne = |dep: &[V], tr: &[u32; 3]| -> bool {
        let (a, b, c) = (m.p[tr[0] as usize], m.p[tr[1] as usize], m.p[tr[2] as usize]);
        let (x, y, z) = (dep[tr[0] as usize], dep[tr[1] as usize], dep[tr[2] as usize]);
        let n0 = maillage::cross(sub(b, a), sub(c, a));
        let n1 = maillage::cross(sub(y, x), sub(z, x));
        let (l0, l1) = (maillage::dot(n0, n0).sqrt(), maillage::dot(n1, n1).sqrt());
        // retourné = sa face regarde à l'opposé de la face d'origine (une paroi raide, à ~90°, est normale)
        l1 < 1e-9 * l0.max(1e-12) || maillage::dot(n0, n1) < -0.02 * l0 * l1
    };
    // voisins de chaque sommet (par les triangles)
    let nv = m.p.len();
    let mut deb = vec![0u32; nv + 1];
    for tr in &m.t { for &v in tr { deb[v as usize + 1] += 2; } }
    for i in 0..nv { deb[i + 1] += deb[i]; }
    let mut vois = vec![0u32; deb[nv] as usize];
    let mut pos = deb.clone();
    for tr in &m.t { for k in 0..3 { let v = tr[k] as usize; for j in [1, 2] { vois[pos[v] as usize] = tr[(k + j) % 3]; pos[v] += 1; } } }
    let mut bilan = Vec::new();
    for passe in 0..40 {
        let mauvais: Vec<u32> = m.t.par_iter().filter(|tr| retourne(dep, tr)).flat_map_iter(|tr| tr.iter().copied()).collect();
        bilan.push(mauvais.len() / 3);
        if mauvais.is_empty() { break; }
        let mut cible = vec![false; nv];
        for v in mauvais { if !gele[v as usize] { cible[v as usize] = true; } }
        // déplacement (vecteur) de chaque sommet ciblé ← moyenne de celui de ses voisins
        let nouveaux: Vec<(usize, V)> = (0..nv).into_par_iter().filter(|&v| cible[v]).map(|v| {
            let vs = &vois[deb[v] as usize..deb[v + 1] as usize];
            let mut s = [0f32; 3];
            for &w in vs { s = add(s, sub(dep[w as usize], m.p[w as usize])); }
            let moy = mul(s, 1.0 / vs.len().max(1) as f32);
            let d = sub(dep[v], m.p[v]);
            (v, add(m.p[v], if passe < 4 { mul(add(moy, d), 0.5) } else { mul(d, 0.5) }))
        }).collect();
        for (v, q) in nouveaux { dep[v] = q; }
    }
    bilan
}

/// Fondu près des zones intactes : facteur 0 au bord → 1 à `largeur` mm (courbe douce). None si largeur nulle.
fn fondu_bords(m: &Maillage, gele: &[bool], bits_face: impl Fn(u32) -> u32, largeur: f32) -> Option<Vec<f32>> {
    if largeur <= 0.0 { return None; }
    // points du bord : arêtes entre un triangle intact et un triangle texturé, échantillonnées
    let ar = maillage::aretes(&m.t);
    let pas = (largeur / 4.0).max(1e-3);
    let mut pts: Vec<V> = Vec::new();
    for e in 0..ar.ab.len() {
        let occ = &ar.occ[ar.debut[e] as usize..ar.debut[e + 1] as usize];
        let g: Vec<bool> = occ.iter().map(|s| bits_face(m.src[*s as usize / 3]) == 0).collect();
        if g.iter().any(|&x| x) && g.iter().any(|&x| !x) {
            let (a, b) = (m.p[ar.ab[e].0 as usize], m.p[ar.ab[e].1 as usize]);
            let n = (long(sub(b, a)) / pas).ceil().max(1.0) as usize;
            for k in 0..=n { pts.push(add(a, mul(sub(b, a), k as f32 / n as f32))); }
        }
    }
    if pts.is_empty() { return None; }
    // grille de cases de côté `largeur`
    let cle = |q: V| ((q[0] / largeur).floor() as i32, (q[1] / largeur).floor() as i32, (q[2] / largeur).floor() as i32);
    let mut grille: std::collections::HashMap<(i32, i32, i32), Vec<u32>> = std::collections::HashMap::new();
    for (i, q) in pts.iter().enumerate() { grille.entry(cle(*q)).or_default().push(i as u32); }
    Some((0..m.p.len()).into_par_iter().map(|v| {
        if gele[v] { return 0.0; }
        let q = m.p[v];
        let (cx, cy, cz) = cle(q);
        let mut d = f32::MAX;
        for dx in -1..=1 { for dy in -1..=1 { for dz in -1..=1 {
            if let Some(l) = grille.get(&(cx + dx, cy + dy, cz + dz)) { for &i in l { d = d.min(long(sub(pts[i as usize], q))); } }
        } } }
        let t = (d / largeur).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }).collect())
}

/// Faces dont la normale est à moins de `haut`° de la verticale montante ou `bas`° de la descendante :
/// laissées intactes (dessus et dessous plats). Renvoie le masque à combiner avec les calques.
pub fn masque_angle(verts: &[f32], haut: f32, bas: f32) -> Vec<bool> {
    let (ch, cb) = (haut.to_radians().cos(), bas.to_radians().cos());
    verts.par_chunks_exact(18).map(|t| {
        let a = [t[0], t[1], t[2]]; let b = [t[6], t[7], t[8]]; let c = [t[12], t[13], t[14]];
        let n = maillage::norm(maillage::cross(sub(b, a), sub(c, a)));
        (haut > 0.0 && n[2] >= ch) || (bas > 0.0 && -n[2] >= cb)
    }).collect()
}

#[cfg(test)]
mod tests;
