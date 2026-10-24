//! Réduction du nombre de triangles par quadriques d'erreur (Garland & Heckbert, 1997), par lots :
//! à chaque tour on choisit les fusions d'arêtes les moins coûteuses qui ne se touchent pas, on vérifie
//! qu'elles gardent la surface fermée et sans retournement, puis on les applique toutes d'un coup.

use super::maillage::{cross, dot, norm, sub, V};
use rayon::prelude::*;

type Q = [f64; 10]; // matrice symétrique 4×4 : a², ab, ac, ad, b², bc, bd, c², cd, d²

#[inline]
fn plan_q(n: [f64; 3], d: f64, w: f64) -> Q {
    let (a, b, c) = (n[0], n[1], n[2]);
    [a * a * w, a * b * w, a * c * w, a * d * w, b * b * w, b * c * w, b * d * w, c * c * w, c * d * w, d * d * w]
}
#[inline] fn qadd(x: &mut Q, y: &Q) { for i in 0..10 { x[i] += y[i]; } }
#[inline]
fn qerr(q: &Q, v: [f64; 3]) -> f64 {
    let (x, y, z) = (v[0], v[1], v[2]);
    q[0] * x * x + 2.0 * q[1] * x * y + 2.0 * q[2] * x * z + 2.0 * q[3] * x + q[4] * y * y + 2.0 * q[5] * y * z + 2.0 * q[6] * y + q[7] * z * z + 2.0 * q[8] * z + q[9]
}
/// Position qui minimise l'erreur (système 3×3) ; None si mal conditionné.
fn qmin(q: &Q) -> Option<[f64; 3]> {
    let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
    let b = [-q[3], -q[6], -q[8]];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let tr = m[0][0] + m[1][1] + m[2][2];
    if det.abs() < 1e-9 * tr.powi(3).max(1e-30) { return None; }
    let col = |c: usize| { let mut mm = m; for r in 0..3 { mm[r][c] = b[r]; } mm };
    let d3 = |mm: [[f64; 3]; 3]| mm[0][0] * (mm[1][1] * mm[2][2] - mm[1][2] * mm[2][1]) - mm[0][1] * (mm[1][0] * mm[2][2] - mm[1][2] * mm[2][0]) + mm[0][2] * (mm[1][0] * mm[2][1] - mm[1][1] * mm[2][0]);
    Some([d3(col(0)) / det, d3(col(1)) / det, d3(col(2)) / det])
}

/// Réduit (p, t) vers `cible` triangles (au mieux). Les sommets marqués `fixe` ne bougent pas.
pub fn reduire(p: &mut Vec<V>, t: &mut Vec<[u32; 3]>, cible: usize, fixe: &[bool]) {
    let nv = p.len();
    let mut pd: Vec<[f64; 3]> = p.iter().map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect();
    // quadriques des sommets : plans des triangles d'origine, pondérés par leur aire
    let qs_t: Vec<(Q, [u32; 3])> = t.par_iter().map(|tr| {
        let (a, b, c) = (pd[tr[0] as usize], pd[tr[1] as usize], pd[tr[2] as usize]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l < 1e-20 { return ([0.0; 10], *tr); }
        let nn = [n[0] / l, n[1] / l, n[2] / l];
        (plan_q(nn, -(nn[0] * a[0] + nn[1] * a[1] + nn[2] * a[2]), l * 0.5), *tr)
    }).collect();
    let mut q = vec![[0f64; 10]; nv];
    for (qq, tr) in &qs_t { for &v in tr { qadd(&mut q[v as usize], qq); } }
    drop(qs_t);
    let mut parent: Vec<u32> = (0..nv as u32).collect();
    let mut tours = 0;
    while t.len() > cible && tours < 60 {
        tours += 1;
        // adjacence sommet → triangles
        let mut deb = vec![0u32; nv + 1];
        for tr in t.iter() { for &v in tr { deb[v as usize + 1] += 1; } }
        for i in 0..nv { deb[i + 1] += deb[i]; }
        let mut adj = vec![0u32; deb[nv] as usize];
        let mut pos = deb.clone();
        for (f, tr) in t.iter().enumerate() { for &v in tr { adj[pos[v as usize] as usize] = f as u32; pos[v as usize] += 1; } }
        let tris_de = |v: u32| &adj[deb[v as usize] as usize..deb[v as usize + 1] as usize];
        // arêtes uniques et leur coût
        let mut cles: Vec<u64> = t.par_iter().flat_map_iter(|tr| (0..3).map(move |k| { let (a, b) = (tr[k], tr[(k + 1) % 3]); ((a.min(b) as u64) << 32) | a.max(b) as u64 })).collect();
        cles.par_sort_unstable();
        cles.dedup();
        let mut cout: Vec<(f64, u32, [f64; 3])> = cles.par_iter().enumerate().filter_map(|(i, &k)| {
            let (a, b) = ((k >> 32) as usize, (k & 0xffff_ffff) as usize);
            if fixe.get(a).copied().unwrap_or(false) && fixe.get(b).copied().unwrap_or(false) { return None; }
            let mut qq = q[a]; qadd(&mut qq, &q[b]);
            let cand: [f64; 3] = if fixe.get(a).copied().unwrap_or(false) { pd[a] } else if fixe.get(b).copied().unwrap_or(false) { pd[b] } else {
                let mil = [(pd[a][0] + pd[b][0]) * 0.5, (pd[a][1] + pd[b][1]) * 0.5, (pd[a][2] + pd[b][2]) * 0.5];
                match qmin(&qq) {
                    // position optimale seulement si elle reste près de l'arête (sinon pointes)
                    Some(o) if ((o[0] - mil[0]).powi(2) + (o[1] - mil[1]).powi(2) + (o[2] - mil[2]).powi(2)).sqrt()
                        <= ((pd[a][0] - pd[b][0]).powi(2) + (pd[a][1] - pd[b][1]).powi(2) + (pd[a][2] - pd[b][2]).powi(2)).sqrt() => o,
                    _ => [pd[a], pd[b], mil].into_iter().min_by(|x, y| qerr(&qq, *x).total_cmp(&qerr(&qq, *y))).unwrap(),
                }
            };
            Some((qerr(&qq, cand).max(0.0), i as u32, cand))
        }).collect();
        cout.par_sort_unstable_by(|x, y| x.0.total_cmp(&y.0));
        // choix des fusions indépendantes (aucun triangle touché par deux fusions)
        let besoin = (t.len() - cible + 1) / 2;
        let mut verrou = vec![false; nv];
        let mut choix: Vec<(u32, u32, [f64; 3])> = Vec::new();
        for &(_, i, cand) in &cout {
            if choix.len() >= besoin { break; }
            let k = cles[i as usize];
            let (a, b) = ((k >> 32) as u32, (k & 0xffff_ffff) as u32);
            if verrou[a as usize] || verrou[b as usize] { continue; }
            if !fusion_valide(a, b, cand, t, &pd, &adj, &deb) { continue; }
            // verrouille l'anneau des deux sommets
            for &f in tris_de(a).iter().chain(tris_de(b)) { for &v in &t[f as usize] { verrou[v as usize] = true; } }
            // b est absorbé par a (a reste, sauf si b est fixe : alors b reste)
            let (garde, part) = if fixe.get(b as usize).copied().unwrap_or(false) { (b, a) } else { (a, b) };
            choix.push((garde, part, cand));
        }
        if choix.is_empty() { break; }
        for &(g, s, cand) in &choix {
            parent[s as usize] = g;
            pd[g as usize] = cand;
            let qs = q[s as usize]; qadd(&mut q[g as usize], &qs);
        }
        let par = &parent;
        let racine = |mut v: u32| { while par[v as usize] != v { v = par[v as usize]; } v };
        *t = t.par_iter().filter_map(|tr| {
            let r = [racine(tr[0]), racine(tr[1]), racine(tr[2])];
            (r[0] != r[1] && r[1] != r[2] && r[0] != r[2]).then_some(r)
        }).collect();
    }
    // sommets utilisés, renumérotés
    let mut nouveau = vec![u32::MAX; nv];
    let mut pp = Vec::new();
    for tr in t.iter_mut() { for v in tr.iter_mut() {
        let i = *v as usize;
        if nouveau[i] == u32::MAX { nouveau[i] = pp.len() as u32; pp.push([pd[i][0] as f32, pd[i][1] as f32, pd[i][2] as f32]); }
        *v = nouveau[i];
    } }
    *p = pp;
}

/// La fusion de l'arête (a, b) au point `cand` garde-t-elle une surface propre ?
/// – condition du lien : a et b n'ont que deux voisins communs (pas de pincement) ;
/// – aucun triangle voisin ne se retourne ni ne s'aplatit.
fn fusion_valide(a: u32, b: u32, cand: [f64; 3], t: &[[u32; 3]], pd: &[[f64; 3]], adj: &[u32], deb: &[u32]) -> bool {
    let tris_de = |v: u32| &adj[deb[v as usize] as usize..deb[v as usize + 1] as usize];
    let voisins = |v: u32| { let mut s: Vec<u32> = tris_de(v).iter().flat_map(|&f| t[f as usize]).filter(|&x| x != v).collect(); s.sort_unstable(); s.dedup(); s };
    let (va, vb) = (voisins(a), voisins(b));
    let communs = va.iter().filter(|x| vb.binary_search(x).is_ok()).count();
    if communs != 2 { return false; }
    let c: V = [cand[0] as f32, cand[1] as f32, cand[2] as f32];
    let pf = |i: u32| { let q = pd[i as usize]; [q[0] as f32, q[1] as f32, q[2] as f32] };
    for &f in tris_de(a).iter().chain(tris_de(b)) {
        let tr = t[f as usize];
        let (ha, hb) = (tr.contains(&a), tr.contains(&b));
        if ha && hb { continue; } // disparaît
        let avant = [pf(tr[0]), pf(tr[1]), pf(tr[2])];
        let mut apres = avant;
        for k in 0..3 { if tr[k] == a || tr[k] == b { apres[k] = c; } }
        let n0 = cross(sub(avant[1], avant[0]), sub(avant[2], avant[0]));
        let n1 = cross(sub(apres[1], apres[0]), sub(apres[2], apres[0]));
        let (l0, l1) = (dot(n0, n0).sqrt(), dot(n1, n1).sqrt());
        if l1 < 1e-12 || l1 < 1e-4 * l0 { return false; }
        if dot(norm(n0), norm(n1)) < 0.2 { return false; }
    }
    true
}

/// Nettoyage avant texture : fusionne les arêtes plus courtes que `lmin` (micro-triangles des fichiers
/// très fins), au milieu de l'arête, par lots indépendants et avec les mêmes garde-fous que la réduction.
/// `src` (face d'origine de chaque triangle) suit les triangles conservés. Renvoie le nombre de fusions.
pub fn nettoyer(p: &mut Vec<V>, t: &mut Vec<[u32; 3]>, src: &mut Vec<u32>, lmin: f32) -> usize {
    let nv = p.len();
    let mut pd: Vec<[f64; 3]> = p.iter().map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect();
    let mut parent: Vec<u32> = (0..nv as u32).collect();
    let mut total = 0;
    for _ in 0..30 {
        let l2 = |a: u32, b: u32| { let (x, y) = (pd[a as usize], pd[b as usize]); (x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2) };
        let seuil = (lmin as f64).powi(2);
        let mut courtes: Vec<(f64, u32, u32)> = t.par_iter().flat_map_iter(|tr| (0..3).filter_map(move |k| {
            let (a, b) = (tr[k], tr[(k + 1) % 3]);
            (a < b).then_some((a, b))
        })).map(|(a, b)| (l2(a, b), a, b)).filter(|x| x.0 < seuil).collect();
        if courtes.is_empty() { break; }
        courtes.par_sort_unstable_by(|x, y| x.0.total_cmp(&y.0));
        courtes.dedup_by_key(|x| (x.1, x.2));
        let mut deb = vec![0u32; nv + 1];
        for tr in t.iter() { for &v in tr { deb[v as usize + 1] += 1; } }
        for i in 0..nv { deb[i + 1] += deb[i]; }
        let mut adj = vec![0u32; deb[nv] as usize];
        let mut pos = deb.clone();
        for (f, tr) in t.iter().enumerate() { for &v in tr { adj[pos[v as usize] as usize] = f as u32; pos[v as usize] += 1; } }
        let tris_de = |v: u32| &adj[deb[v as usize] as usize..deb[v as usize + 1] as usize];
        let mut verrou = vec![false; nv];
        let mut n = 0;
        for &(_, a, b) in &courtes {
            if verrou[a as usize] || verrou[b as usize] { continue; }
            let (x, y) = (pd[a as usize], pd[b as usize]);
            let mil = [(x[0] + y[0]) * 0.5, (x[1] + y[1]) * 0.5, (x[2] + y[2]) * 0.5];
            if !fusion_valide(a, b, mil, t, &pd, &adj, &deb) { continue; }
            for &f in tris_de(a).iter().chain(tris_de(b)) { for &v in &t[f as usize] { verrou[v as usize] = true; } }
            parent[b as usize] = a;
            pd[a as usize] = mil;
            n += 1;
        }
        if n == 0 { break; }
        total += n;
        let par = &parent;
        let racine = |mut v: u32| { while par[v as usize] != v { v = par[v as usize]; } v };
        let gardes: Vec<([u32; 3], u32)> = t.par_iter().zip(src.par_iter()).filter_map(|(tr, &s)| {
            let r = [racine(tr[0]), racine(tr[1]), racine(tr[2])];
            (r[0] != r[1] && r[1] != r[2] && r[0] != r[2]).then_some((r, s))
        }).collect();
        *t = gardes.iter().map(|x| x.0).collect();
        *src = gardes.iter().map(|x| x.1).collect();
    }
    if total == 0 { return 0; }
    // sommets utilisés, renumérotés
    let mut nouveau = vec![u32::MAX; nv];
    let mut pp = Vec::new();
    for tr in t.iter_mut() { for v in tr.iter_mut() {
        let i = *v as usize;
        if nouveau[i] == u32::MAX { nouveau[i] = pp.len() as u32; pp.push([pd[i][0] as f32, pd[i][1] as f32, pd[i][2] as f32]); }
        *v = nouveau[i];
    } }
    *p = pp;
    total
}
