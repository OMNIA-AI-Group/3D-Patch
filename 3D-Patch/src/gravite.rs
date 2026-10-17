//! GRAVITÉ : pose l'objet comme il tomberait sur le plateau (plan XY).
//!
//! Bouton GRAVITÉ : `simuler` = vraie chute physique (corps rigide, rebonds, frottement, roulis).
//! Ci-dessous `repos` = ancienne descente quasi statique, gardée en repli :
//!
//! 1. centre de gravité réel (volume du maillage ; repli : centre des surfaces) ;
//! 2. enveloppe convexe (Quickhull) ;
//! 3. bascule « quasi statique » : depuis l'orientation actuelle, la gravité (−Z) désigne la face
//!    de l'enveloppe sous le centre de gravité ; si la verticale du centre de gravité tombe hors de
//!    cette face, l'objet bascule sur la face voisine… jusqu'à une face stable (l'énergie baisse à
//!    chaque bascule, la descente s'arrête toujours) ;
//! 4. rotation qui amène cette face à plat, face vers le bas.

use glam::{DMat3, DQuat, DVec3};
use std::collections::HashMap;

/// Centre de gravité d'un solide (sommets [x y z nx ny nz] × 3 par triangle).
pub fn centre_gravite(verts: &[f32]) -> DVec3 {
    let (mut v6, mut acc, mut aire, mut acc_s) = (0.0f64, DVec3::ZERO, 0.0f64, DVec3::ZERO);
    for t in verts.chunks_exact(18) {
        let a = DVec3::new(t[0] as f64, t[1] as f64, t[2] as f64);
        let b = DVec3::new(t[6] as f64, t[7] as f64, t[8] as f64);
        let c = DVec3::new(t[12] as f64, t[13] as f64, t[14] as f64);
        let v = a.dot(b.cross(c)); // 6 × volume signé du tétraèdre (origine, a, b, c)
        v6 += v; acc += (a + b + c) * v;
        let s = (b - a).cross(c - a).length();
        aire += s; acc_s += (a + b + c) * s;
    }
    // volume trop faible (surface ouverte, plaque) : centre des surfaces
    let ref_v = aire.powf(1.5).max(1e-12);
    if v6.abs() > 1e-6 * ref_v { acc / (4.0 * v6) } else if aire > 0.0 { acc_s / (3.0 * aire) } else { DVec3::ZERO }
}

struct Face { v: [usize; 3], n: DVec3, d: f64, dehors: Vec<usize>, vivant: bool }

/// Enveloppe convexe (Quickhull 3D). Renvoie (points, faces orientées vers l'extérieur).
pub fn enveloppe(pts_in: &[DVec3]) -> Option<(Vec<DVec3>, Vec<[usize; 3]>)> {
    // dédoublonnage (grille fine) : les maillages répètent chaque sommet 6 fois en moyenne
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in pts_in { lo = lo.min(*p); hi = hi.max(*p); }
    let ext = (hi - lo).max_element();
    if !(ext > 0.0) { return None; }
    let q = ext * 1e-7;
    let mut vus: HashMap<(i64, i64, i64), ()> = HashMap::new();
    let mut p: Vec<DVec3> = Vec::new();
    for x in pts_in {
        let k = ((x.x / q).round() as i64, (x.y / q).round() as i64, (x.z / q).round() as i64);
        if vus.insert(k, ()).is_none() { p.push(*x); }
    }
    let eps = ext * 1e-9 * 100.0;
    if p.len() < 4 { return None; }
    // tétraèdre de départ
    let (mut i0, mut i1) = (0, 0);
    for (i, x) in p.iter().enumerate() { if x.x < p[i0].x { i0 = i; } if x.x > p[i1].x { i1 = i; } }
    if (p[i1] - p[i0]).length() < eps {
        for (i, x) in p.iter().enumerate() { if (*x - p[i0]).length() > (p[i1] - p[i0]).length() { i1 = i; } }
    }
    let ab = (p[i1] - p[i0]).normalize();
    let i2 = (0..p.len()).max_by(|&a, &b| ((p[a] - p[i0]).cross(ab).length()).total_cmp(&(p[b] - p[i0]).cross(ab).length()))?;
    let n0 = (p[i1] - p[i0]).cross(p[i2] - p[i0]);
    if n0.length() < eps * eps { return None; }
    let n0 = n0.normalize();
    let i3 = (0..p.len()).max_by(|&a, &b| ((p[a] - p[i0]).dot(n0).abs()).total_cmp(&(p[b] - p[i0]).dot(n0).abs()))?;
    if (p[i3] - p[i0]).dot(n0).abs() < eps { return None; } // objet plat : pas de volume
    let centre = (p[i0] + p[i1] + p[i2] + p[i3]) / 4.0;

    let mut faces: Vec<Face> = Vec::new();
    let mut aretes: HashMap<(usize, usize), usize> = HashMap::new();
    let mut ajoute = |faces: &mut Vec<Face>, aretes: &mut HashMap<(usize, usize), usize>, a: usize, b: usize, c: usize| -> usize {
        let n = (p[b] - p[a]).cross(p[c] - p[a]).normalize_or_zero();
        let id = faces.len();
        faces.push(Face { v: [a, b, c], n, d: n.dot(p[a]), dehors: Vec::new(), vivant: true });
        aretes.insert((a, b), id); aretes.insert((b, c), id); aretes.insert((c, a), id);
        id
    };
    for (a, b, c) in [(i0, i1, i2), (i0, i2, i3), (i0, i3, i1), (i1, i3, i2)] {
        // orientation extérieure
        let n = (p[b] - p[a]).cross(p[c] - p[a]);
        if n.dot(centre - p[a]) > 0.0 { ajoute(&mut faces, &mut aretes, a, c, b); } else { ajoute(&mut faces, &mut aretes, a, b, c); }
    }
    for i in 0..p.len() {
        if [i0, i1, i2, i3].contains(&i) { continue; }
        for f in faces.iter_mut() { if f.n.dot(p[i]) - f.d > eps { f.dehors.push(i); break; } }
    }
    let mut pile: Vec<usize> = (0..faces.len()).collect();
    let mut garde = 0usize;
    while let Some(fi) = pile.pop() {
        garde += 1; if garde > 4_000_000 { break; }
        if !faces[fi].vivant || faces[fi].dehors.is_empty() { continue; }
        // point le plus éloigné
        let f = &faces[fi];
        let oeil = *f.dehors.iter().max_by(|&&a, &&b| (f.n.dot(p[a]) - f.d).total_cmp(&(f.n.dot(p[b]) - f.d))).unwrap();
        // faces visibles (parcours des voisines)
        let mut visibles = vec![fi];
        let mut vu: HashMap<usize, bool> = HashMap::new();
        vu.insert(fi, true);
        let mut k = 0;
        while k < visibles.len() {
            let v = faces[visibles[k]].v;
            for e in 0..3 {
                let (a, b) = (v[e], v[(e + 1) % 3]);
                if let Some(&g) = aretes.get(&(b, a)) {
                    if vu.contains_key(&g) { continue; }
                    let vis = faces[g].n.dot(p[oeil]) - faces[g].d > eps;
                    vu.insert(g, vis);
                    if vis { visibles.push(g); }
                }
            }
            k += 1;
        }
        // horizon : arêtes des faces visibles dont la voisine n'est pas visible
        let mut horizon = Vec::new();
        for &g in &visibles {
            let v = faces[g].v;
            for e in 0..3 {
                let (a, b) = (v[e], v[(e + 1) % 3]);
                let voisine = aretes.get(&(b, a)).copied();
                if voisine.map_or(true, |h| !vu.get(&h).copied().unwrap_or(false)) { horizon.push((a, b)); }
            }
        }
        let mut orphelins = Vec::new();
        for &g in &visibles {
            faces[g].vivant = false;
            let v = faces[g].v;
            for e in 0..3 { let key = (v[e], v[(e + 1) % 3]); if aretes.get(&key) == Some(&g) { aretes.remove(&key); } }
            orphelins.append(&mut faces[g].dehors);
        }
        let mut nouvelles = Vec::new();
        for (a, b) in horizon { nouvelles.push(ajoute(&mut faces, &mut aretes, a, b, oeil)); }
        for i in orphelins {
            if i == oeil { continue; }
            for &g in &nouvelles { if faces[g].n.dot(p[i]) - faces[g].d > eps { faces[g].dehors.push(i); break; } }
        }
        pile.extend(nouvelles);
    }
    let tris: Vec<[usize; 3]> = faces.iter().filter(|f| f.vivant && f.n != DVec3::ZERO).map(|f| f.v).collect();
    if tris.len() < 4 { return None; }
    Some((p, tris))
}

/// Rotation (à appliquer autour du centre de gravité) qui pose l'objet sur sa face stable,
/// en partant de l'orientation actuelle (gravité = −Z). None si impossible (objet plat, vide…).
pub fn repos(verts: &[f32]) -> Option<(DMat3, DVec3)> {
    let cg = centre_gravite(verts);
    let pts: Vec<DVec3> = verts.chunks_exact(6).map(|c| DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64)).collect();
    let (p, tris) = enveloppe(&pts)?;
    // plans des faces (normale extérieure, décalage)
    let plans: Vec<(DVec3, f64)> = tris.iter().map(|t| {
        let n = (p[t[1]] - p[t[0]]).cross(p[t[2]] - p[t[0]]).normalize();
        (n, n.dot(p[t[0]]))
    }).collect();
    // le centre de gravité doit être dans l'enveloppe (sinon on prend le centre de l'enveloppe)
    let dedans = |c: DVec3| plans.iter().all(|(n, d)| n.dot(c) <= d + 1e-9);
    let cg = if dedans(cg) { cg } else { p.iter().fold(DVec3::ZERO, |a, b| a + *b) / p.len() as f64 };
    // face touchée par la demi-droite cg + t·dir (sortie du polyèdre convexe = t minimal)
    let sortie = |dir: DVec3| -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (i, (n, d)) in plans.iter().enumerate() {
            let nd = n.dot(dir);
            if nd <= 1e-12 { continue; }
            let t = (d - n.dot(cg)) / nd;
            if best.map_or(true, |b| t < b.1) { best = Some((i, t)); }
        }
        best.map(|b| b.0)
    };
    let mut f = sortie(DVec3::NEG_Z)?;
    let mut h = plans[f].1 - plans[f].0.dot(cg); // hauteur du centre de gravité au-dessus de la face
    for _ in 0..10_000 {
        let n = plans[f].0;
        let g = sortie(n)?;
        // stable : la verticale du centre de gravité retombe dans la face (ou une face coplanaire)
        if g == f || plans[g].0.dot(n) > 1.0 - 1e-9 { break; }
        let hg = plans[g].1 - plans[g].0.dot(cg);
        if hg >= h - 1e-12 { break; } // plus de descente possible
        f = g; h = hg;
    }
    let n = plans[f].0;
    let q = DQuat::from_rotation_arc(n, DVec3::NEG_Z);
    Some((DMat3::from_quat(q), cg))
}

/// Chute simulée (corps rigide) : trajectoire image par image (60 i/s) = (rotation depuis
/// l'orientation de départ, position du centre de gravité), dernière image exactement posée à plat.
pub struct Chute { pub images: Vec<(glam::Quat, glam::Vec3)>, pub cg0: glam::Vec3 }

/// Inertie (autour du centre de gravité, masse volumique 1) et masse d'un solide maillé ;
/// repli boîte englobante si le maillage n'enferme pas de volume.
fn inertie(verts: &[f32], cg: DVec3) -> (f64, DMat3) {
    let (mut m, mut c) = (0.0f64, [[0.0f64; 3]; 3]); // masse, covariance Σ ∫ x xᵀ
    for t in verts.chunks_exact(18) {
        let a = DVec3::new(t[0] as f64, t[1] as f64, t[2] as f64) - cg;
        let b = DVec3::new(t[6] as f64, t[7] as f64, t[8] as f64) - cg;
        let d = DVec3::new(t[12] as f64, t[13] as f64, t[14] as f64) - cg;
        let v = a.dot(b.cross(d)) / 6.0; // tétraèdre (cg, a, b, d)
        m += v;
        let s = a + b + d;
        for i in 0..3 { for j in 0..3 {
            // ∫ x_i x_j sur le tétraèdre (sommet à l'origine) = v/20 · (Σ a_i a_j + s_i s_j)
            let aa = a[i] * a[j] + b[i] * b[j] + d[i] * d[j];
            c[i][j] += v / 20.0 * (aa + s[i] * s[j]);
        }}
    }
    if m > 1e-9 {
        let tr = c[0][0] + c[1][1] + c[2][2];
        let i = DMat3::from_cols_array(&[tr - c[0][0], -c[1][0], -c[2][0], -c[0][1], tr - c[1][1], -c[2][1], -c[0][2], -c[1][2], tr - c[2][2]]);
        if i.determinant() > 0.0 { return (m, i); }
    }
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in verts.chunks_exact(6) { let q = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64); lo = lo.min(q); hi = hi.max(q); }
    let e = (hi - lo).max(DVec3::splat(1e-3));
    let m = e.x * e.y * e.z;
    (m, DMat3::from_diagonal(DVec3::new(e.y * e.y + e.z * e.z, e.x * e.x + e.z * e.z, e.x * e.x + e.y * e.y) * (m / 12.0)))
}

/// Le point `c` est-il à l'intérieur de l'enveloppe convexe (2D) des points `pts` (avec une petite marge) ?
fn dans_polygone(c: glam::DVec2, pts: &[glam::DVec2]) -> bool {
    // enveloppe 2D (Andrew), puis côté de chaque bord
    let mut p: Vec<glam::DVec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap().then(a.y.partial_cmp(&b.y).unwrap()));
    p.dedup_by(|a, b| (*a - *b).length() < 1e-9);
    if p.len() < 3 { return false; }
    let croix = |o: glam::DVec2, a: glam::DVec2, b: glam::DVec2| (a - o).perp_dot(b - o);
    let mut h: Vec<glam::DVec2> = Vec::new();
    for pass in 0..2 {
        let debut = h.len();
        let it: Box<dyn Iterator<Item = &glam::DVec2>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= debut + 2 && croix(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 { h.pop(); }
            h.push(q);
        }
        h.pop();
    }
    if h.len() < 3 { return false; }
    (0..h.len()).all(|i| croix(h[i], h[(i + 1) % h.len()], c) >= 0.0)
}

/// GRAVITÉ : vraie chute physique de l'objet (enveloppe convexe contre le plateau Z=0) depuis son
/// orientation actuelle, lâché de `hauteur` (point le plus bas) : gravité, rebonds amortis,
/// frottement, roulis et bascules jusqu'à l'arrêt. L'objet finit là où la physique le mène,
/// pas sur un équilibre précaire. None si l'objet n'a pas de volume.
pub fn simuler(verts: &[f32], hauteur: f32) -> Option<Chute> {
    let cg0 = centre_gravite(verts);
    let pts: Vec<DVec3> = verts.chunks_exact(6).map(|c| DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64)).collect();
    let (hp, htris) = enveloppe(&pts)?;
    // sommets réellement sur l'enveloppe (repère du corps) : seuls eux peuvent toucher le plateau
    let mut sur: Vec<bool> = vec![false; hp.len()];
    for t in &htris { for &i in t { sur[i] = true; } }
    let r0: Vec<DVec3> = hp.iter().zip(&sur).filter(|(_, &b)| b).map(|(p, _)| *p - cg0).collect();
    let (masse, ib) = inertie(verts, cg0);
    let ib_inv = ib.inverse();
    let taille = { let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)); for p in &hp { lo = lo.min(*p); hi = hi.max(*p); } (hi - lo).length() };
    let g = DVec3::new(0.0, 0.0, -9810.0); // mm/s²
    let (e_rebond, mu) = (0.3f64, 0.55f64);
    let dt = 1.0 / 2000.0;
    let zmin0 = r0.iter().map(|r| r.z).fold(f64::INFINITY, f64::min);
    let mut x = DVec3::new(cg0.x, cg0.y, hauteur as f64 - zmin0);
    let mut q = DQuat::IDENTITY;
    let (mut v, mut w) = (DVec3::ZERO, DVec3::ZERO);
    let mut images = vec![(glam::Quat::IDENTITY, x.as_vec3())];
    let (mut calme, mut t) = (0.0f64, 0.0f64);
    let pas_image = (2000 / 60) as usize;
    let mut k = 0usize;
    while t < 12.0 {
        v += g * dt;
        let rm = DMat3::from_quat(q);
        let iw_inv = rm * ib_inv * rm.transpose();
        // contacts : sommets de l'enveloppe sous le plateau
        let contacts: Vec<DVec3> = r0.iter().map(|r| rm * *r).filter(|r| x.z + r.z < 0.0).collect();
        if !contacts.is_empty() {
            let mut jn_acc = vec![0.0f64; contacts.len()];
            for _ in 0..10 {
                for (ci, r) in contacts.iter().enumerate() {
                    let vp = v + w.cross(*r);
                    // rebond seulement pour un choc franc, sinon contact inélastique (l'objet se pose)
                    let e = if vp.z < -300.0 { e_rebond } else { 0.0 };
                    let n = DVec3::Z;
                    let kn = 1.0 / masse + n.dot((iw_inv * r.cross(n)).cross(*r));
                    let jn = -(1.0 + e) * vp.z / kn;
                    let ancien = jn_acc[ci];
                    jn_acc[ci] = (ancien + jn).max(0.0);
                    let dj = jn_acc[ci] - ancien;
                    v += n * (dj / masse); w += iw_inv * r.cross(n * dj);
                    // frottement (cône de Coulomb)
                    let vp = v + w.cross(*r);
                    let vt = DVec3::new(vp.x, vp.y, 0.0);
                    let l = vt.length();
                    if l > 1e-9 {
                        let tt = vt / l;
                        let kt = 1.0 / masse + tt.dot((iw_inv * r.cross(tt)).cross(*r));
                        let jt = (-l / kt).max(-mu * jn_acc[ci]);
                        v += tt * (jt / masse); w += iw_inv * r.cross(tt * jt);
                    }
                }
            }
            // correction de pénétration
            let pen = contacts.iter().map(|r| -(x.z + r.z)).fold(0.0, f64::max);
            x.z += pen * 0.8;
            // résistance au roulement : l'objet finit par s'immobiliser
            w *= 1.0 - 3.0 * dt;
        }
        x += v * dt;
        let dq = DQuat::from_xyzw(w.x, w.y, w.z, 0.0) * q;
        q = DQuat::from_xyzw(q.x + 0.5 * dt * dq.x, q.y + 0.5 * dt * dq.y, q.z + 0.5 * dt * dq.z, q.w + 0.5 * dt * dq.w).normalize();
        t += dt; k += 1;
        if k % pas_image == 0 { images.push((q.as_quat(), x.as_vec3())); }
        // arrêt : seulement en position VRAIMENT stable (au moins 3 appuis, centre de gravité au-dessus
        // de leur polygone) et presque immobile ; jamais sur un équilibre précaire
        let lent = v.length() < 0.08 * taille && w.length() * taille * 0.5 < 0.08 * taille;
        let stable = lent && {
            let rm = DMat3::from_quat(q);
            let appuis: Vec<glam::DVec2> = r0.iter().map(|r| rm * *r).filter(|r| x.z + r.z < 2e-4 * taille + 0.02).map(|r| glam::DVec2::new(x.x + r.x, x.y + r.y)).collect();
            appuis.len() >= 3 && dans_polygone(glam::DVec2::new(x.x, x.y), &appuis)
        };
        if stable { calme += dt; if calme > 0.15 { break; } } else { calme = 0.0; }
    }
    // pose finale exacte : la face de l'enveloppe la plus tournée vers le bas est mise à plat
    let rm = DMat3::from_quat(q);
    let bas = htris.iter().map(|t| { let n = (hp[t[1]] - hp[t[0]]).cross(hp[t[2]] - hp[t[0]]).normalize_or_zero(); rm * n })
        .max_by(|a, b| (-a.z).partial_cmp(&-b.z).unwrap_or(std::cmp::Ordering::Equal))?;
    let qf = DQuat::from_rotation_arc(bas, DVec3::NEG_Z) * q;
    let rmf = DMat3::from_quat(qf);
    let zf = r0.iter().map(|r| (rmf * *r).z).fold(f64::INFINITY, f64::min);
    let xf = DVec3::new(x.x, x.y, -zf);
    // quelques images pour glisser en douceur vers la pose exacte
    let (q1, x1) = (q.as_quat(), x.as_vec3());
    for i in 1..=6 { let s = i as f32 / 6.0; images.push((q1.slerp(qf.as_quat(), s), x1.lerp(xf.as_vec3(), s))); }
    Some(Chute { images, cg0: cg0.as_vec3() })
}

/// Face plane sur laquelle l'objet peut être posé (bouton SURFACE) : normale extérieure `n`,
/// plan n·p = d, triangles de l'objet qui la composent (pour l'afficher en rouge), aire.
pub struct FacePose { pub n: glam::Vec3, pub d: f32, pub tol: f32, pub tris: Vec<[glam::Vec3; 3]>, pub aire: f32 }

/// Faces planes de l'objet qui touchent son enveloppe convexe (l'objet peut reposer dessus),
/// d'aire suffisante (les facettes des surfaces courbes sont écartées), de la plus grande à la
/// plus petite.
pub fn faces_de_pose(verts: &[f32]) -> Vec<FacePose> {
    use glam::Vec3;
    let pts: Vec<DVec3> = verts.chunks_exact(6).map(|c| DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64)).collect();
    let Some((p, tris)) = enveloppe(&pts) else { return Vec::new() };
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for q in &p { lo = lo.min(*q); hi = hi.max(*q); }
    let tol = ((hi - lo).max_element() * 2e-4).max(1e-4);
    // plans de l'enveloppe, regroupés (faces coplanaires)
    let mut plans: HashMap<(i64, i64, i64, i64), DVec3> = HashMap::new();
    for t in &tris {
        let n = (p[t[1]] - p[t[0]]).cross(p[t[2]] - p[t[0]]);
        if n.length() < 1e-18 { continue; }
        let n = n.normalize();
        let d = n.dot(p[t[0]]);
        let k = ((n.x * 500.0).round() as i64, (n.y * 500.0).round() as i64, (n.z * 500.0).round() as i64, (d / tol).round() as i64);
        plans.entry(k).or_insert(n);
    }
    // triangles de l'objet rangés par normale approchée (recherche rapide)
    let tv: Vec<[Vec3; 3]> = verts.chunks_exact(18).map(|t| [Vec3::new(t[0], t[1], t[2]), Vec3::new(t[6], t[7], t[8]), Vec3::new(t[12], t[13], t[14])]).collect();
    let mut aire_tot = 0.0f32;
    let mut seau: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    let cle = |n: Vec3| ((n.x * 40.0).round() as i32, (n.y * 40.0).round() as i32, (n.z * 40.0).round() as i32);
    let mut nt: Vec<Vec3> = Vec::with_capacity(tv.len());
    for (i, t) in tv.iter().enumerate() {
        let c = (t[1] - t[0]).cross(t[2] - t[0]);
        aire_tot += c.length() * 0.5;
        let n = c.normalize_or_zero();
        nt.push(n);
        if n != Vec3::ZERO { seau.entry(cle(n)).or_default().push(i); }
    }
    let mut out: Vec<FacePose> = Vec::new();
    for n in plans.values() {
        let n32 = n.as_vec3();
        // plan d'appui : le plus loin de l'objet dans la direction n
        let appui = p.iter().map(|q| n.dot(*q)).fold(f64::MIN, f64::max) as f32;
        let tolf = tol as f32;
        let c0 = cle(n32);
        let mut sel: Vec<[Vec3; 3]> = Vec::new();
        let mut aire = 0.0f32;
        for dx in -1..=1 { for dy in -1..=1 { for dz in -1..=1 {
            if let Some(l) = seau.get(&(c0.0 + dx, c0.1 + dy, c0.2 + dz)) {
                for &i in l {
                    if nt[i].dot(n32) < 0.9995 { continue; }
                    let t = &tv[i];
                    if t.iter().all(|v| (n32.dot(*v) - appui).abs() < tolf) {
                        aire += (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5;
                        sel.push(*t);
                    }
                }
            }
        }}}
        if aire < aire_tot * 0.004 || sel.is_empty() { continue; }
        // plans voisins (même face vue par plusieurs groupes) : on garde un seul exemplaire
        if out.iter().any(|f| f.n.dot(n32) > 0.9995 && (f.d - appui).abs() < tolf) { continue; }
        out.push(FacePose { n: n32, d: appui, tol: tolf, tris: sel, aire });
    }
    out.sort_by(|a, b| b.aire.partial_cmp(&a.aire).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(40);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn boite(l: f32, w: f32, h: f32) -> Vec<f32> {
        let c = |x: f32, y: f32, z: f32| [x * l, y * w, z * h];
        let q = [
            [c(0.,0.,0.), c(0.,1.,0.), c(1.,1.,0.), c(1.,0.,0.)], [c(0.,0.,1.), c(1.,0.,1.), c(1.,1.,1.), c(0.,1.,1.)],
            [c(0.,0.,0.), c(1.,0.,0.), c(1.,0.,1.), c(0.,0.,1.)], [c(0.,1.,0.), c(0.,1.,1.), c(1.,1.,1.), c(1.,1.,0.)],
            [c(0.,0.,0.), c(0.,0.,1.), c(0.,1.,1.), c(0.,1.,0.)], [c(1.,0.,0.), c(1.,1.,0.), c(1.,1.,1.), c(1.,0.,1.)],
        ];
        let mut v = Vec::new();
        for f in q { for t in [[0, 1, 2], [0, 2, 3]] { for i in t { v.extend_from_slice(&f[i]); v.extend_from_slice(&[0.0, 0.0, 0.0]); } } }
        v
    }
    fn tourne(v: &mut [f32], m: glam::Mat3) { for c in v.chunks_exact_mut(6) { let p = m * glam::Vec3::new(c[0], c[1], c[2]); c[0] = p.x; c[1] = p.y; c[2] = p.z; } }
    #[test]
    fn faces_de_pose_boite() {
        // boîte : ses 6 faces sont des faces de pose, chacune de l'aire attendue
        let f = faces_de_pose(&boite(10.0, 20.0, 30.0));
        assert_eq!(f.len(), 6);
        let mut aires: Vec<f32> = f.iter().map(|x| x.aire).collect();
        aires.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for (a, b) in aires.iter().zip([200.0, 200.0, 300.0, 300.0, 600.0, 600.0]) { assert!((a - b).abs() < 1e-3, "{aires:?}"); }
        // la plus grande face est en premier, normale unitaire extérieure
        assert!((f[0].aire - 600.0).abs() < 1e-3 && f[0].n.x.abs() > 0.999);
    }
    #[test]
    fn cg_boite() {
        let g = centre_gravite(&boite(10.0, 20.0, 30.0));
        assert!((g - DVec3::new(5.0, 10.0, 15.0)).length() < 1e-6);
    }
    #[test]
    fn enveloppe_boite_et_sphere() {
        let v = boite(10.0, 20.0, 30.0);
        let pts: Vec<DVec3> = v.chunks_exact(6).map(|c| DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64)).collect();
        let (p, t) = enveloppe(&pts).unwrap();
        assert_eq!(p.len(), 8); assert_eq!(t.len(), 12);
        // nuage sphérique : tous les points sur l'enveloppe, aucun dehors
        let mut s = Vec::new();
        for i in 0..2000 { let a = i as f64 * 2.399963; let z = 1.0 - 2.0 * (i as f64 + 0.5) / 2000.0; let r = (1.0 - z * z).sqrt(); s.push(DVec3::new(r * a.cos(), r * a.sin(), z)); }
        s.push(DVec3::ZERO);
        let (p, t) = enveloppe(&s).unwrap();
        for x in &s { for f in &t { let n = (p[f[1]] - p[f[0]]).cross(p[f[2]] - p[f[0]]).normalize(); assert!(n.dot(*x - p[f[0]]) < 1e-6); } }
        assert_eq!(t.len(), 2 * 2000 - 4);
    }
    #[test]
    fn boite_penchee_retombe_a_plat() {
        // boîte 10×20×30 penchée de 20° : elle retombe sur la face où elle reposait (30 de haut)
        let mut v = boite(10.0, 20.0, 30.0);
        tourne(&mut v, glam::Mat3::from_rotation_x(20f32.to_radians()));
        let (m, _) = repos(&v).unwrap();
        let z = m * DMat3::from_rotation_x(20f64.to_radians()) * DVec3::Z;
        assert!((z.z.abs() - 1.0).abs() < 1e-6, "hauteur toujours verticale : {z:?}");
        // penchée de 70° : elle bascule sur le flanc (axe Y de la boîte vertical)
        let mut v = boite(10.0, 20.0, 30.0);
        tourne(&mut v, glam::Mat3::from_rotation_x(70f32.to_radians()));
        let (m, _) = repos(&v).unwrap();
        let y = m * DMat3::from_rotation_x(70f64.to_radians()) * DVec3::Y;
        assert!((y.z.abs() - 1.0).abs() < 1e-6, "bascule sur le flanc : {y:?}");
    }
}
