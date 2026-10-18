//! Booléens entre objets (union, soustraction, intersection) : moteur Manifold (portage Rust,
//! résultats identiques à la bibliothèque C++ de référence). Le moteur « robuste » prend le relais
//! automatiquement pour les maillages imparfaits (non-manifold, auto-intersections) s'ils sont fermés.

use manifold_rust::manifold::Manifold;
use manifold_rust::progress::ProgressReporter;
use manifold_rust::types::{BooleanEngine, Error, MeshGL64, OpType};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op { Union, Soustraction, Intersection }

/// Soudure exacte (au bit près) des sommets d'une soupe [pos nrm]×3×triangles -> maillage indexé f64.
fn vers_manifold(v: &[f32]) -> MeshGL64 {
    let mut ids: HashMap<[u32; 3], u64> = HashMap::new();
    let mut pos: Vec<f64> = Vec::new();
    let mut tri: Vec<u64> = Vec::with_capacity(v.len() / 6);
    for t in v.chunks_exact(18) {
        let mut idx = [0u64; 3];
        for k in 0..3 {
            let p = [t[k * 6], t[k * 6 + 1], t[k * 6 + 2]];
            let n = ids.len() as u64;
            idx[k] = *ids.entry(p.map(f32::to_bits)).or_insert_with(|| { pos.extend(p.iter().map(|&x| x as f64)); n });
        }
        // triangle dégénéré (deux sommets confondus) : sans surface, il n'apporte rien
        if idx[0] == idx[1] || idx[1] == idx[2] || idx[0] == idx[2] { continue; }
        tri.extend_from_slice(&idx);
    }
    MeshGL64 { num_prop: 3, vert_properties: pos, tri_verts: tri, ..Default::default() }
}

/// Maillage indexé -> soupe [pos nrm]×3×triangles (normale à plat par triangle).
fn depuis_manifold(m: &MeshGL64) -> Vec<f32> {
    let np = m.num_prop as usize;
    let p = |i: u64| { let o = i as usize * np; glam::DVec3::new(m.vert_properties[o], m.vert_properties[o + 1], m.vert_properties[o + 2]) };
    let mut out = Vec::with_capacity(m.tri_verts.len() * 6);
    for t in m.tri_verts.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let n = (b - a).cross(c - a).normalize_or_zero();
        for q in [a, b, c] { out.extend_from_slice(&[q.x as f32, q.y as f32, q.z as f32, n.x as f32, n.y as f32, n.z as f32]); }
    }
    out
}

fn importer(v: &[f32], nom: &str) -> Result<Manifold, String> {
    let mut m = Manifold::from_mesh_gl64_robust(&vers_manifold(v));
    if m.status() != Error::NoError {
        // objet ouvert ou défectueux : réparation automatique (trous rebouchés, faces remises
        // dans le bon sens) puis nouvel essai, au lieu de refuser l'opération
        let r = crate::defects::repair(v);
        if r.len() >= 36 { m = Manifold::from_mesh_gl64_robust(&vers_manifold(&r)); }
    }
    match m.status() {
        Error::NoError => Ok(m),
        Error::NotClosed => Err(format!("{nom} : {}", crate::trc("bool_ouvert"))),
        e => Err(format!("{nom} : {} ({e:?})", crate::trc("bool_invalide"))),
    }
}

/// A op B. Erreur (traduite) si un des deux objets n'est pas fermé ou si le résultat est vide.
pub fn calculer(a: &[f32], b: &[f32], op: Op) -> Result<Vec<f32>, String> {
    calculer_n(&[a, b], op)
}

/// Booléen entre plusieurs objets, comme 3D Builder (ordre de sélection) :
/// fusion / intersection de tous ; soustraction = le dernier retiré de tous les autres.
pub fn calculer_n(objs: &[&[f32]], op: Op) -> Result<Vec<f32>, String> {
    if objs.len() < 2 { return Err(crate::trc("bool_choisir").into()); }
    crate::progress::span(0.0, 0.1);
    let mut ms = Vec::with_capacity(objs.len());
    for (k, v) in objs.iter().enumerate() { ms.push(importer(v, &format!("{} {}", crate::trc("bool_obj"), k + 1))?); }
    // jauge toujours croissante : chaque phase du moteur repart de 0, on les cumule
    let n_ops = (objs.len() - 1) as f32;
    let mut r = ms.remove(0);
    let outil = if op == Op::Soustraction { ms.pop() } else { None };
    let mut etape = 0f32;
    let mut appliquer = |r: Manifold, m: &Manifold, opt: OpType| -> Result<Manifold, String> {
        crate::progress::span(0.1 + 0.85 * etape / n_ops, 0.1 + 0.85 * (etape + 1.0) / n_ops);
        etape += 1.0;
        let etat = std::sync::Mutex::new((u32::MAX, 0u32));
        let rapport = ProgressReporter::new(move |phase, f| {
            let mut e = etat.lock().unwrap();
            if e.0 != phase.id() { e.0 = phase.id(); e.1 += 1; }
            let n = e.1 as f64;
            crate::progress::set(((n - 1.0 + f.unwrap_or(0.5)) / (n + 1.0)) as f32); // None = phase sans total connu
        });
        let r = r.boolean_with_engine_and_progress(m, opt, BooleanEngine::Auto, None, Some(&rapport));
        if r.status() != Error::NoError { return Err(format!("{} ({:?})", crate::trc("bool_echec"), r.status())); }
        Ok(r)
    };
    let opt = match op { Op::Intersection => OpType::Intersect, _ => OpType::Add };
    for m in &ms { r = appliquer(r, m, opt)?; }
    if let Some(o) = outil { r = appliquer(r, &o, OpType::Subtract)?; }
    crate::progress::span(0.95, 1.0);
    let mut out = depuis_manifold(&r.get_mesh_gl64(-1));
    if out.is_empty() { return Err(crate::trc("bool_vide").into()); }
    // sommets presque confondus le long des coupes (gros maillages) : soudés par la réparation,
    // volume inchangé ; l'objet rendu est alors sans défaut pour l'analyse de l'appli
    let d = crate::defects::analyze(&out);
    if d.holes > 0 || d.face_errors > 0 {
        out = crate::defects::repair(&out);
        // pincements restants (deux nappes qui se touchent le long d'une arête) : la réparation seule
        // ne peut rien ; faces retirées autour, puis trou rebouché proprement
        let d = crate::defects::analyze(&out);
        if d.face_errors > 0 || d.holes > 0 { out = crate::defects::retirer_pincements(&out); }
    }
    crate::progress::set(1.0);
    Ok(out)
}

/// Cadre de coupe : on trace une ligne p0 -> p1 (le trait L) dans un plan perpendiculaire à l'axe `e`
/// (0 = X, 1 = Y, 2 = Z), puis le cadre se déplie de `l` en pivotant autour du trait : `incl` = angle
/// depuis le plan de tracé (90° = perpendiculaire, le long de l'axe ; 45° = coupe en biais).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cadre {
    pub p0: glam::Vec3, pub p1: glam::Vec3, pub e: usize,
    /// inclinaison du plan de coupe autour du trait, en degrés
    pub incl: f32,
    /// longueur dépliée l (0 = toute la hauteur de la scène)
    pub l: f32,
}

impl Cadre {
    /// Cadre perpendiculaire au plan de tracé, sur toute la hauteur de la scène.
    pub fn new(p0: glam::Vec3, p1: glam::Vec3, e: usize) -> Cadre { Cadre { p0, p1, e, incl: 90.0, l: 0.0 } }
    pub fn axe(&self) -> glam::Vec3 { let mut v = glam::Vec3::ZERO; v[self.e] = 1.0; v }
    /// Début du trait ramené dans le plan de tracé.
    pub fn origine(&self) -> glam::Vec3 { let mut o = self.p0; o[self.e] = 0.0; o }
    /// Direction du trait (dans le plan de tracé) et sa longueur L.
    fn trait_l(&self) -> Option<(glam::Vec3, f32)> {
        let mut d = self.p1 - self.p0; d[self.e] = 0.0;
        let l = d.length();
        (l >= 1e-3).then(|| (d / l, l))
    }
    /// Perpendiculaire au trait dans le plan de tracé (0° de la roue d'inclinaison).
    pub fn perp_plan(&self) -> Option<glam::Vec3> { self.trait_l().map(|(u, _)| u.cross(self.axe()).normalize()) }
    /// Direction de dépli de l : perpendiculaire au trait, tournée de `incl` depuis le plan de tracé vers l'axe.
    pub fn dir_l(&self) -> Option<glam::Vec3> {
        let np = self.perp_plan()?;
        // angles droits exacts : un cadre à 90° reste rigoureusement le long de l'axe
        let a = self.incl.rem_euclid(360.0);
        let (s, c) = if a == 0.0 { (0.0, 1.0) } else if a == 90.0 { (1.0, 0.0) } else if a == 180.0 { (0.0, -1.0) } else if a == 270.0 { (-1.0, 0.0) }
                     else { a.to_radians().sin_cos() };
        Some(np * c + self.axe() * s)
    }
    /// Direction du trait, normale du cadre (côté A) et longueur L.
    pub fn reperes(&self) -> Option<(glam::Vec3, glam::Vec3, f32)> {
        let (u, l) = self.trait_l()?;
        Some((u, u.cross(self.dir_l()?).normalize(), l))
    }
    /// Les 4 coins du cadre, déplié de `bas` à `haut` le long de dir_l (depuis le trait).
    pub fn coins(&self, bas: f32, haut: f32) -> [glam::Vec3; 4] {
        let (Some((u, l)), Some(w)) = (self.trait_l(), self.dir_l()) else { return [self.p0, self.p1, self.p1, self.p0] };
        let (a, b) = (self.origine(), self.origine() + u * l);
        [a + w * bas, b + w * bas, b + w * haut, a + w * haut]
    }
    /// Étendue (min, max) de la boîte `lo`..`hi` le long de dir_l, depuis le trait.
    pub fn etendue_l(&self, lo: glam::Vec3, hi: glam::Vec3) -> (f32, f32) {
        let Some(w) = self.dir_l() else { return (0.0, 0.0) };
        let o = self.origine();
        (0..8).map(|i| glam::Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }))
            .fold((f32::MAX, f32::MIN), |(a, b), q| { let s = (q - o).dot(w); (a.min(s), b.max(s)) })
    }
}
fn boite_coupe(c: &Cadre, bas: f32, haut: f32, prof: f32) -> Option<Vec<f32>> {
    let (u, n, l) = c.reperes()?;
    let w = c.dir_l()?;
    let o = c.origine();
    let p = |i: u8, j: u8, k: u8| o + u * (l * i as f32) + n * (prof * j as f32) + w * if k == 0 { bas } else { haut };
    let faces = [[(0,0,0),(1,0,0),(1,1,0),(0,1,0)], [(0,0,1),(0,1,1),(1,1,1),(1,0,1)], [(0,0,0),(0,0,1),(1,0,1),(1,0,0)],
                 [(0,1,0),(1,1,0),(1,1,1),(0,1,1)], [(0,0,0),(0,1,0),(0,1,1),(0,0,1)], [(1,0,0),(1,0,1),(1,1,1),(1,1,0)]];
    let mut tris: Vec<[glam::Vec3; 3]> = Vec::new();
    for f in faces {
        let q: Vec<glam::Vec3> = f.iter().map(|&(i, j, k)| p(i, j, k)).collect();
        tris.push([q[0], q[1], q[2]]); tris.push([q[0], q[2], q[3]]);
    }
    // orientation : volume signé positif = normales vers l'extérieur
    let vol: f32 = tris.iter().map(|t| t[0].dot(t[1].cross(t[2]))).sum();
    let mut out = Vec::with_capacity(12 * 18);
    for t in &tris {
        let t = if vol < 0.0 { [t[0], t[2], t[1]] } else { *t };
        for q in t { out.extend_from_slice(&[q.x, q.y, q.z, 0.0, 0.0, 0.0]); }
    }
    Some(out)
}

/// Coupe un objet par le cadre : (morceau côté A, morceau côté B). La coupe se limite à la longueur
/// de la ligne tracée (comme un emporte-pièce) ; une ligne qui dépasse de l'objet = coupe franche.
pub fn couper(v: &[f32], c: &Cadre) -> Result<(Vec<f32>, Vec<f32>), String> {
    crate::progress::span(0.0, 0.1);
    let m = importer(v, &crate::trc("bool_obj"))?;
    let (mut lo, mut hi) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
    for p in v.chunks_exact(6) { let q = glam::Vec3::new(p[0], p[1], p[2]); lo = lo.min(q); hi = hi.max(q); }
    let marge = (hi - lo).length() + 1.0;
    let (bas, haut) = c.etendue_l(lo, hi);
    let Some(b) = boite_coupe(c, bas - 1.0, haut + 1.0, marge + (c.p0 - (lo + hi) * 0.5).length()) else { return Err(crate::trc("coupe_rate").into()) };
    let boite = Manifold::from_mesh_gl64(&vers_manifold(&b));
    crate::progress::span(0.1, 0.5);
    let plus = m.boolean_with_engine_and_progress(&boite, OpType::Intersect, BooleanEngine::Auto, None, None);
    crate::progress::span(0.5, 0.9);
    let moins = m.boolean_with_engine_and_progress(&boite, OpType::Subtract, BooleanEngine::Auto, None, None);
    crate::progress::span(0.9, 1.0);
    let sortie = |r: &Manifold| -> Result<Vec<f32>, String> {
        if r.status() != Error::NoError { return Err(format!("{} ({:?})", crate::trc("bool_echec"), r.status())); }
        let mut out = depuis_manifold(&r.get_mesh_gl64(-1));
        if out.is_empty() { return Ok(out); }
        let d = crate::defects::analyze(&out);
        if d.holes > 0 || d.face_errors > 0 {
            out = crate::defects::repair(&out);
            let d = crate::defects::analyze(&out);
            if d.face_errors > 0 || d.holes > 0 { out = crate::defects::retirer_pincements(&out); }
        }
        Ok(out)
    };
    let (p, q) = (sortie(&plus)?, sortie(&moins)?);
    crate::progress::set(1.0);
    if p.is_empty() || q.is_empty() { return Err(crate::trc("coupe_rate").into()); }
    Ok((p, q))
}

/// Lignes d'intersection entre objets qui se chevauchent (aperçu des booléens) : segments où les
/// surfaces se croisent, pour chaque paire d'objets. Grille de recherche sur la zone commune des
/// boîtes englobantes : rapide même sur des maillages de plusieurs centaines de milliers de triangles.
pub fn intersections(objs: &[Vec<[glam::Vec3; 3]>]) -> Vec<[glam::Vec3; 2]> {
    use glam::Vec3;
    let boite = |ts: &[[Vec3; 3]]| ts.iter().flatten().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    let mut out = Vec::new();
    for i in 0..objs.len() {
        for j in i + 1..objs.len() {
            let (bi, bj) = (boite(&objs[i]), boite(&objs[j]));
            let (lo, hi) = (bi.0.max(bj.0), bi.1.min(bj.1));
            if lo.cmpgt(hi).any() { continue; }
            let marge = (hi - lo).max_element() * 1e-4 + 1e-6;
            let (lo, hi) = (lo - Vec3::splat(marge), hi + Vec3::splat(marge));
            let dans = |t: &[Vec3; 3]| { let (a, b) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2])); !(a.cmpgt(hi).any() || b.cmplt(lo).any()) };
            let ta: Vec<&[Vec3; 3]> = objs[i].iter().filter(|t| dans(t)).collect();
            let tb: Vec<&[Vec3; 3]> = objs[j].iter().filter(|t| dans(t)).collect();
            if ta.is_empty() || tb.is_empty() { continue; }
            // grille : ~ (nb triangles)^(1/3) cases par côté, bornée
            let n = ((ta.len() + tb.len()) as f32).cbrt().clamp(1.0, 96.0) as i32;
            let taille = (hi - lo).max(Vec3::splat(1e-6)) / n as f32;
            let case = |p: Vec3| { let c = ((p - lo) / taille).floor(); (c.x.clamp(0.0, (n - 1) as f32) as i32, c.y.clamp(0.0, (n - 1) as f32) as i32, c.z.clamp(0.0, (n - 1) as f32) as i32) };
            let mut grille: std::collections::HashMap<(i32, i32, i32), Vec<u32>> = std::collections::HashMap::new();
            for (k, t) in tb.iter().enumerate() {
                let (a, b) = (case(t[0].min(t[1]).min(t[2])), case(t[0].max(t[1]).max(t[2])));
                for x in a.0..=b.0 { for y in a.1..=b.1 { for z in a.2..=b.2 { grille.entry((x, y, z)).or_default().push(k as u32); } } }
            }
            let mut vus: std::collections::HashSet<u32> = std::collections::HashSet::new();
            for t in &ta {
                let (a, b) = (case(t[0].min(t[1]).min(t[2])), case(t[0].max(t[1]).max(t[2])));
                vus.clear();
                for x in a.0..=b.0 { for y in a.1..=b.1 { for z in a.2..=b.2 {
                    if let Some(l) = grille.get(&(x, y, z)) { for &k in l { if vus.insert(k) { if let Some(sg) = tri_tri(t, tb[k as usize]) { out.push(sg); } } } }
                }}}
            }
        }
    }
    out
}

/// Segment d'intersection de deux triangles (None s'ils ne se croisent pas ou sont coplanaires) :
/// la traversée de A par le plan de B, rognée à l'intérieur de B.
fn tri_tri(a: &[glam::Vec3; 3], b: &[glam::Vec3; 3]) -> Option<[glam::Vec3; 2]> {
    let nb = (b[1] - b[0]).cross(b[2] - b[0]);
    if nb.length_squared() < 1e-24 { return None; }
    let d = [(a[0] - b[0]).dot(nb), (a[1] - b[0]).dot(nb), (a[2] - b[0]).dot(nb)];
    if (d[0] > 0.0 && d[1] > 0.0 && d[2] > 0.0) || (d[0] < 0.0 && d[1] < 0.0 && d[2] < 0.0) || (d[0] == 0.0 && d[1] == 0.0 && d[2] == 0.0) { return None; }
    let mut pts = Vec::with_capacity(2);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        if (d[i] < 0.0) != (d[j] < 0.0) { pts.push(a[i] + (a[j] - a[i]) * (d[i] / (d[i] - d[j]))); }
    }
    if pts.len() != 2 { return None; }
    let (p, q) = (pts[0], pts[1]);
    // rognage par les 3 demi-plans intérieurs de B (dans son plan)
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for e in 0..3 {
        let (u, v) = (b[e], b[(e + 1) % 3]);
        let interieur = nb.cross(v - u); // pointe vers l'intérieur de B
        let (fp, fq) = ((p - u).dot(interieur), (q - u).dot(interieur));
        if fp < 0.0 && fq < 0.0 { return None; }
        if fp < 0.0 { t0 = t0.max(fp / (fp - fq)); } else if fq < 0.0 { t1 = t1.min(fp / (fp - fq)); }
    }
    if t1 - t0 <= 1e-6 { return None; }
    Some([p + (q - p) * t0, p + (q - p) * t1])
}

/// Dernier trou d'évacuation percé par `creuser` : [x, y, z bas, z haut, rayon].
static TROU: std::sync::Mutex<Option<[f32; 5]>> = std::sync::Mutex::new(None);
pub fn trou_perce() -> Option<[f32; 5]> { TROU.lock().unwrap().take() }

/// CREUSER : l'objet devient une coque de paroi `epaisseur` (mm). `trou` = Ø (mm) d'un trou
/// d'évacuation percé verticalement sous le point le plus bas de la cavité (résine, poudre…).
/// Renvoie (objet creusé, part de matière économisée 0..1).
pub fn creuser(v: &[f32], epaisseur: f32, trou: Option<f32>, res: usize) -> Result<(Vec<f32>, f64), String> {
    *TROU.lock().unwrap() = None;
    crate::progress::span(0.0, 0.05);
    let m = importer(v, &crate::trc("bool_obj"))?;
    let v_rep = depuis_manifold(&m.get_mesh_gl64(-1)); // objet réparé si besoin : base du calcul
    crate::progress::span(0.05, 0.6);
    // paroi intérieure allégée (booléen bien plus rapide, objet final pas plus lourd que nécessaire)
    let cible = (v.len() / 18).clamp(20_000, 200_000);
    let int = crate::remesh::interieur(&v_rep, epaisseur, res, cible);
    if int.len() < 36 { return Err(crate::trc("creux_plein").into()); }
    let mi = Manifold::from_mesh_gl64_robust(&vers_manifold(&int));
    if mi.status() != Error::NoError || mi.is_empty() { return Err(crate::trc("creux_plein").into()); }
    crate::progress::span(0.6, 0.85);
    let mut r = m.boolean_with_engine_and_progress(&mi, OpType::Subtract, BooleanEngine::Auto, None, None);
    if r.status() != Error::NoError { return Err(format!("{} ({:?})", crate::trc("bool_echec"), r.status())); }
    if let Some(d) = trou.filter(|&d| d > 0.0) {
        // zone la plus basse de la cavité (fond plat : toute une surface) -> le point de cette zone
        // le plus proche de son centre : trou centré sous la cavité
        let zc = int.chunks_exact(6).map(|p| p[2]).fold(f32::INFINITY, f32::min);
        let bas: Vec<(f32, f32, f32)> = int.chunks_exact(6).filter(|p| p[2] < zc + epaisseur * 0.25 + 1e-3).map(|p| (p[0], p[1], p[2])).collect();
        let (mx, my) = bas.iter().fold((0.0f32, 0.0f32), |a, p| (a.0 + p.0, a.1 + p.1));
        let (mx, my) = (mx / bas.len() as f32, my / bas.len() as f32);
        let bas_cav = *bas.iter().min_by(|a, b| ((a.0 - mx).powi(2) + (a.1 - my).powi(2)).partial_cmp(&((b.0 - mx).powi(2) + (b.1 - my).powi(2))).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
        let z0 = v_rep.chunks_exact(6).map(|p| p[2]).fold(f32::INFINITY, f32::min);
        let h = (bas_cav.2 - z0) as f64 + epaisseur as f64 * 1.5 + 2.0;
        let cyl = Manifold::cylinder(h, d as f64 * 0.5, d as f64 * 0.5, 32)
            .translate(manifold_rust::linalg::Vec3::new(bas_cav.0 as f64, bas_cav.1 as f64, z0 as f64 - 1.0));
        r = r.boolean_with_engine_and_progress(&cyl, OpType::Subtract, BooleanEngine::Auto, None, None);
        if r.status() != Error::NoError { return Err(format!("{} ({:?})", crate::trc("bool_echec"), r.status())); }
        // emplacement du trou (pour le marqueur affiché après le calcul) : du dessous de l'objet
        // jusqu'au fond de la cavité
        *TROU.lock().unwrap() = Some([bas_cav.0, bas_cav.1, z0, bas_cav.2 + epaisseur * 0.25, d * 0.5]);
    }
    crate::progress::span(0.85, 1.0);
    let mut out = depuis_manifold(&r.get_mesh_gl64(-1));
    if out.is_empty() { return Err(crate::trc("creux_plein").into()); }
    let d = crate::defects::analyze(&out);
    if d.holes > 0 || d.face_errors > 0 {
        out = crate::defects::repair(&out);
        let d = crate::defects::analyze(&out);
        if d.face_errors > 0 || d.holes > 0 { out = crate::defects::retirer_pincements(&out); }
    }
    let vol = |w: &[f32]| w.chunks_exact(18).map(|t| { let p = |k: usize| glam::DVec3::new(t[k * 6] as f64, t[k * 6 + 1] as f64, t[k * 6 + 2] as f64); p(0).dot(p(1).cross(p(2))) }).sum::<f64>() / 6.0;
    let (v0, v1) = (vol(&v_rep), vol(&out));
    crate::progress::set(1.0);
    Ok((out, if v0 > 0.0 { (1.0 - v1 / v0).clamp(0.0, 1.0) } else { 0.0 }))
}

/// TEXTE & RELIEF : placement du texte. Repère de la vue où le cadre a été tracé : `u` vers la
/// droite, `v` vers le haut, `n` vers l'observateur (axes du monde) ; `centre` = centre du cadre
/// (sa composante selon `n` est ignorée), `angle` en radians dans le plan (u, v).
pub struct ParamsTexte {
    pub polys: Vec<Vec<[f64; 2]>>, pub u: glam::Vec3, pub v: glam::Vec3, pub n: glam::Vec3,
    pub centre: glam::Vec3, pub angle: f32, pub prof: f32, pub relief: bool, pub suivre: bool,
}

/// Hauteurs de la surface de l'objet vue depuis l'observateur (le long de `n`), avec une grille
/// de recherche dans le plan (u, v).
struct Relief { tris: Vec<[[f32; 3]; 3]>, grille: std::collections::HashMap<(i32, i32), Vec<u32>>, pas: f32 }
impl Relief {
    fn new(v: &[f32], u: glam::Vec3, vv: glam::Vec3, n: glam::Vec3) -> Relief {
        let tris: Vec<[[f32; 3]; 3]> = v.chunks_exact(18).map(|t| {
            let q = |k: usize| { let p = glam::Vec3::new(t[k * 6], t[k * 6 + 1], t[k * 6 + 2]); [p.dot(u), p.dot(vv), p.dot(n)] };
            [q(0), q(1), q(2)]
        }).collect();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for t in &tris { for p in t { for d in 0..2 { lo[d] = lo[d].min(p[d]); hi[d] = hi[d].max(p[d]); } } }
        let pas = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / (tris.len() as f32).sqrt().max(1.0) * 2.0).max(1e-3);
        let mut grille: std::collections::HashMap<(i32, i32), Vec<u32>> = std::collections::HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let (a0, a1) = (t[0][0].min(t[1][0]).min(t[2][0]), t[0][0].max(t[1][0]).max(t[2][0]));
            let (b0, b1) = (t[0][1].min(t[1][1]).min(t[2][1]), t[0][1].max(t[1][1]).max(t[2][1]));
            for x in (a0 / pas).floor() as i32..=(a1 / pas).floor() as i32 {
                for y in (b0 / pas).floor() as i32..=(b1 / pas).floor() as i32 { grille.entry((x, y)).or_default().push(i as u32); }
            }
        }
        Relief { tris, grille, pas }
    }
    /// Hauteur (le long de n) de la surface la plus proche de l'observateur au point (a, b).
    fn h(&self, a: f32, b: f32) -> Option<f32> {
        let l = self.grille.get(&((a / self.pas).floor() as i32, (b / self.pas).floor() as i32))?;
        let mut best: Option<f32> = None;
        for &i in l {
            let t = &self.tris[i as usize];
            let (x0, y0, x1, y1, x2, y2) = (t[0][0], t[0][1], t[1][0], t[1][1], t[2][0], t[2][1]);
            let d = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2);
            if d.abs() < 1e-12 { continue; }
            let l0 = ((y1 - y2) * (a - x2) + (x2 - x1) * (b - y2)) / d;
            let l1 = ((y2 - y0) * (a - x2) + (x0 - x2) * (b - y2)) / d;
            let l2 = 1.0 - l0 - l1;
            if l0 < -1e-5 || l1 < -1e-5 || l2 < -1e-5 { continue; }
            let z = l0 * t[0][2] + l1 * t[1][2] + l2 * t[2][2];
            if best.map_or(true, |bz| z > bz) { best = Some(z); }
        }
        best
    }
}

/// Sortie commune des opérations : maillage réparé si besoin (pincements compris).
fn finaliser(r: &Manifold) -> Result<Vec<f32>, String> {
    if r.status() != Error::NoError { return Err(format!("{} ({:?})", crate::trc("bool_echec"), r.status())); }
    let mut out = depuis_manifold(&r.get_mesh_gl64(-1));
    if out.is_empty() { return Err(crate::trc("bool_vide").into()); }
    let d = crate::defects::analyze(&out);
    if d.holes > 0 || d.face_errors > 0 {
        out = crate::defects::repair(&out);
        let d = crate::defects::analyze(&out);
        if d.face_errors > 0 || d.holes > 0 { out = crate::defects::retirer_pincements(&out); }
    }
    Ok(out)
}

/// Incruste le texte dans l'objet : en relief (ajouté, fait partie de l'objet) ou gravé (creusé).
/// `suivre` : le texte épouse la courbure de la surface ; sinon il est posé à plat au plus haut
/// point de la surface sous le texte.
pub fn texte(obj: &[f32], p: &ParamsTexte) -> Result<Vec<f32>, String> {
    use manifold_rust::linalg::{Vec2, Vec3};
    crate::progress::span(0.0, 0.1);
    let sec = crate::texte::section(&p.polys);
    if sec.is_empty() { return Err(crate::trc("texte_vide").into()); }
    crate::progress::span(0.1, 0.3);
    let rel = Relief::new(obj, p.u, p.v, p.n);
    let (ca, cb) = (p.centre.dot(p.u), p.centre.dot(p.v));
    let (sn, cs) = p.angle.sin_cos();
    let plan = |x: f64, y: f64| (ca + cs * x as f32 - sn * y as f32, cb + sn * x as f32 + cs * y as f32);
    // à plat : posé sur le plus haut point de la surface sous le texte ; le pied d'un relief descend jusqu'au
    // plus bas (texte incliné, surface courbe) pour que les lettres restent soudées à l'objet
    let (mut h_max, mut h_min): (Option<f32>, f32) = (None, f32::MAX);
    for c in &p.polys { for q in c {
        let (a, b) = plan(q[0], q[1]);
        if let Some(h) = rel.h(a, b) { h_max = Some(h_max.map_or(h, |x: f32| x.max(h))); h_min = h_min.min(h); }
    } }
    let Some(h_plat) = h_max else { return Err(crate::trc("texte_hors").into()) };
    let rec = (p.prof * 0.2).max(0.2) + if p.relief && !p.suivre { h_plat - h_min } else { 0.0 }; // recouvrement : le texte entre un peu dans l'objet
    let haut = (p.prof + rec) as f64;
    let mut prisme = Manifold::extrude(&sec, haut, 0, 0.0, Vec2::new(1.0, 1.0))
        .translate(Vec3::new(0.0, 0.0, if p.relief { -rec as f64 } else { -p.prof as f64 }));
    if p.suivre {
        // subdivision : assez fine pour épouser la courbure, bornée pour rester rapide
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for c in &p.polys { for q in c { for d in 0..2 { lo[d] = lo[d].min(q[d]); hi[d] = hi[d].max(q[d]); } } }
        let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2)).sqrt();
        prisme = prisme.refine_to_length((diag / 120.0).max(0.25));
    }
    let (u, v, n, suivre) = (p.u, p.v, p.n, p.suivre);
    let pose = prisme.warp(|q: &mut Vec3| {
        let (a, b) = plan(q.x, q.y);
        let h = if suivre { rel.h(a, b).unwrap_or(h_plat) } else { h_plat } + q.z as f32;
        let w = u * a + v * b + n * h;
        *q = Vec3::new(w.x as f64, w.y as f64, w.z as f64);
    });
    crate::progress::span(0.3, 0.9);
    let o = importer(obj, &crate::trc("bool_obj"))?;
    let r = o.boolean_with_engine_and_progress(&pose, if p.relief { OpType::Add } else { OpType::Subtract }, BooleanEngine::Auto, None, None);
    crate::progress::span(0.9, 1.0);
    let out = finaliser(&r)?;
    crate::progress::set(1.0);
    Ok(out)
}

#[cfg(test)]
pub fn test_vers(v: &[f32]) -> MeshGL64 { vers_manifold(v) }
