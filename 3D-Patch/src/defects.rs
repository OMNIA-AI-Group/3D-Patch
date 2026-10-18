//! Analyse des défauts d'un maillage : trous (arêtes de bord) et erreurs de face
//! (arêtes non-manifold ou faces mal orientées). Fournit aussi un traceur de lignes GL.

use std::collections::VecDeque;
use crate::fasthash::{HashMap, HashSet};
use glow::HasContext;

/// Soudure des sommets : renvoie (positions uniques, indice soudé par coin).
fn weld(verts: &[f32]) -> (Vec<[f32; 3]>, Vec<u32>) {
    let ntri = verts.len() / 18;
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for t in 0..ntri { for k in 0..3 { let o = t * 18 + k * 6; for d in 0..3 { lo[d] = lo[d].min(verts[o + d]); hi[d] = hi[d].max(verts[o + d]); } } }
    let ext = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6);
    let cell = ext * 2e-5;
    let q = |v: f32| (v / cell).round() as i64;
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::default();
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut idx: Vec<u32> = Vec::with_capacity(ntri * 3);
    for t in 0..ntri {
        for k in 0..3 {
            let o = t * 18 + k * 6;
            let key = (q(verts[o]), q(verts[o + 1]), q(verts[o + 2]));
            let id = *map.entry(key).or_insert_with(|| { pos.push([verts[o], verts[o + 1], verts[o + 2]]); (pos.len() - 1) as u32 });
            idx.push(id);
        }
    }
    (pos, idx)
}

fn flat_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let (ux, uy, uz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let (vx, vy, vz) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
    let (mut nx, mut ny, mut nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    let l = (nx * nx + ny * ny + nz * nz).sqrt();
    if l > 1e-12 { nx /= l; ny /= l; nz /= l; } else { nz = 1.0; }
    [nx, ny, nz]
}

fn ekey(a: u32, b: u32) -> (u32, u32) { if a < b { (a, b) } else { (b, a) } }

/// Réparation complète (objectif : 0 trou, 0 face en erreur), géométrie d'origine conservée :
///  1. soudure + retrait des faces dégénérées / doublons ;
///  2. arêtes non-manifold (>2 faces) : on n'accepte une face que si ses 3 arêtes ont encore
///     de la place (≤2 faces) — les faces en trop sont retirées, le trou laissé sera rebouché ;
///  3. orientation cohérente par parcours en largeur ; les paires restées incohérentes
///     (zones non orientables type Möbius) perdent une face, puis on recommence ;
///  4. chaque coque est tournée normales vers l'extérieur (volume signé) ;
///  5. bouchage de toutes les boucles de bord (gère les sommets « nœud papillon »).
/// Renvoie un nouveau tableau de sommets entrelacés [pos nrm] * 3 * triangles.
/// Pincements résistants à la réparation : on retire les faces autour des arêtes fautives
/// (anneau qui s'élargit à chaque essai), puis la réparation rebouche le trou proprement.
pub fn retirer_pincements(verts: &[f32]) -> Vec<f32> {
    let mut v = verts.to_vec();
    for anneau in 1..=4 {
        let d = analyze(&v);
        if d.face_errors == 0 && d.holes == 0 { break; }
        let (pos, idx) = weld(&v);
        let tris: Vec<[u32; 3]> = (0..idx.len() / 3).map(|t| [idx[t * 3], idx[t * 3 + 1], idx[t * 3 + 2]]).collect();
        let mut ecount: HashMap<(u32, u32), (u32, i32)> = HashMap::default();
        for t in &tris { for e in 0..3 { let (a, b) = (t[e], t[(e + 1) % 3]); let ent = ecount.entry(ekey(a, b)).or_insert((0, 0)); ent.0 += 1; ent.1 += if a < b { 1 } else { -1 }; } }
        let mut zone: HashSet<u32> = ecount.iter().filter(|(_, &(n, s))| n > 2 || (n == 2 && s != 0)).flat_map(|(&(a, b), _)| [a, b]).collect();
        if zone.is_empty() { return repair(&v); }
        // élargit la zone d'un anneau de sommets voisins par essai
        for _ in 1..anneau {
            let mut plus = zone.clone();
            for t in &tris { if t.iter().any(|x| zone.contains(x)) { plus.extend(t.iter().copied()); } }
            zone = plus;
        }
        let mut out = Vec::with_capacity(v.len());
        for t in &tris {
            if t.iter().any(|x| zone.contains(x)) { continue; }
            let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            let n = flat_normal(a, b, c);
            for q in [a, b, c] { out.extend_from_slice(&[q[0], q[1], q[2], n[0], n[1], n[2]]); }
        }
        v = repair(&out);
    }
    v
}

pub fn repair(verts: &[f32]) -> Vec<f32> {
    let (mut pos, idx) = weld(verts);
    let mut tris: Vec<[u32; 3]> = (0..idx.len() / 3).map(|t| [idx[t * 3], idx[t * 3 + 1], idx[t * 3 + 2]]).collect();
    crate::progress::set(0.1);

    for round in 0..20 {
        crate::progress::set(0.1 + 0.5 * (1.0 - 1.0 / (round + 1) as f32));
        // 1. dégénérées + doublons (même sommets quel que soit l'ordre)
        let mut seen: HashSet<[u32; 3]> = HashSet::default();
        tris.retain(|tri| {
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] { return false; }
            let mut k = *tri; k.sort_unstable(); seen.insert(k)
        });
        // 2. manifold par construction : chaque arête reçoit au plus 2 faces
        let mut ecount: HashMap<(u32, u32), u8> = HashMap::with_capacity_and_hasher(tris.len() * 2, Default::default());
        tris.retain(|tri| {
            let ks = [ekey(tri[0], tri[1]), ekey(tri[1], tri[2]), ekey(tri[2], tri[0])];
            if ks.iter().any(|k| ecount.get(k).copied().unwrap_or(0) >= 2) { return false; }
            for k in ks { *ecount.entry(k).or_insert(0) += 1; }
            true
        });
        // 3. orientation cohérente
        let ntri = tris.len();
        let mut emap: HashMap<(u32, u32), Vec<usize>> = HashMap::with_capacity_and_hasher(ntri * 2, Default::default());
        for (t, tri) in tris.iter().enumerate() { for e in 0..3 { emap.entry(ekey(tri[e], tri[(e + 1) % 3])).or_default().push(t); } }
        let dir_of = |tri: &[u32; 3], k: (u32, u32)| -> i32 {
            for e in 0..3 { if (tri[e], tri[(e + 1) % 3]) == k { return 1; } }
            -1
        };
        let mut comp = vec![usize::MAX; ntri];
        let mut ncomp = 0;
        for seed in 0..ntri {
            if comp[seed] != usize::MAX { continue; }
            comp[seed] = ncomp;
            let mut q = VecDeque::from([seed]);
            while let Some(t) = q.pop_front() {
                let tri = tris[t];
                for e in 0..3 {
                    let k = ekey(tri[e], tri[(e + 1) % 3]);
                    for &nt in &emap[&k] {
                        if nt == t || comp[nt] != usize::MAX { continue; }
                        // même sens sur l'arête commune => incohérent => on retourne le voisin
                        if dir_of(&tris[t], k) == dir_of(&tris[nt], k) { tris[nt].swap(1, 2); }
                        comp[nt] = ncomp;
                        q.push_back(nt);
                    }
                }
            }
            ncomp += 1;
        }
        // paires encore incohérentes (non orientable) : on retire la 2e face et on recommence
        let mut drop: HashSet<usize> = HashSet::default();
        for (k, list) in &emap {
            if list.len() == 2 && dir_of(&tris[list[0]], *k) == dir_of(&tris[list[1]], *k) { drop.insert(list[1]); }
        }
        // 4. chaque coque normales vers l'extérieur
        let mut vol = vec![0.0f64; ncomp];
        for (t, tri) in tris.iter().enumerate() {
            let (a, b, c) = (pos[tri[0] as usize], pos[tri[1] as usize], pos[tri[2] as usize]);
            vol[comp[t]] += a[0] as f64 * (b[1] as f64 * c[2] as f64 - b[2] as f64 * c[1] as f64)
                          - a[1] as f64 * (b[0] as f64 * c[2] as f64 - b[2] as f64 * c[0] as f64)
                          + a[2] as f64 * (b[0] as f64 * c[1] as f64 - b[1] as f64 * c[0] as f64);
        }
        for (t, tri) in tris.iter_mut().enumerate() { if vol[comp[t]] < 0.0 { tri.swap(1, 2); } }
        if drop.is_empty() { break; }
        let mut t = 0;
        tris.retain(|_| { let keep = !drop.contains(&t); t += 1; keep });
    }

    // 5. bouchage : demi-arêtes de bord (multi-sorties par sommet => nœuds papillon gérés)
    let mut und: HashMap<(u32, u32), u8> = HashMap::with_capacity_and_hasher(tris.len() * 2, Default::default());
    for tri in &tris { for e in 0..3 { *und.entry(ekey(tri[e], tri[(e + 1) % 3])).or_insert(0) += 1; } }
    let mut out_edges: HashMap<u32, Vec<u32>> = HashMap::default();
    let mut bedges: Vec<(u32, u32)> = Vec::new();
    for tri in &tris {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            if und[&ekey(a, b)] == 1 { out_edges.entry(a).or_default().push(b); bedges.push((a, b)); }
        }
    }
    // clés de soudure existantes : un centre de trou ne doit pas retomber sur un sommet existant
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in &pos { for d in 0..3 { lo[d] = lo[d].min(p[d]); hi[d] = hi[d].max(p[d]); } }
    let cell = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6) * 2e-5;
    let qk = |p: [f32; 3]| ((p[0] / cell).round() as i64, (p[1] / cell).round() as i64, (p[2] / cell).round() as i64);
    let mut keys: HashSet<(i64, i64, i64)> = pos.iter().map(|&p| qk(p)).collect();
    let mut used: HashSet<(u32, u32)> = HashSet::default();
    crate::progress::set(0.7);
    let nb = bedges.len().max(1);
    for (bi, &(sa, sb)) in bedges.iter().enumerate() {
        if bi % 4096 == 0 { crate::progress::set(0.7 + 0.25 * bi as f32 / nb as f32); }
        if used.contains(&(sa, sb)) { continue; }
        // marche : on suit une sortie libre à chaque sommet jusqu'à revenir au départ
        let mut lp: Vec<u32> = Vec::new();
        let (mut a, mut b) = (sa, sb);
        loop {
            used.insert((a, b));
            lp.push(a);
            if b == sa { break; }
            let nb = out_edges.get(&b).and_then(|v| v.iter().copied().find(|&n| !used.contains(&(b, n))));
            match nb { Some(n) => { a = b; b = n; } None => break }
        }
        // une boucle qui repasse par un même sommet est découpée en boucles simples
        // (sinon le bouchage en éventail créerait des arêtes à 4 faces)
        let mut loops: Vec<Vec<u32>> = Vec::new();
        let mut stack: Vec<u32> = Vec::new();
        let mut at: HashMap<u32, usize> = HashMap::default();
        for &v in &lp {
            if let Some(&i) = at.get(&v) {
                let sub: Vec<u32> = stack.drain(i..).collect();
                for s in &sub { at.remove(s); }
                loops.push(sub);
            }
            at.insert(v, stack.len());
            stack.push(v);
        }
        loops.push(stack);
        for lp in loops { fill_loop(&lp, &mut pos, &mut tris, &mut keys, &qk, &mut und); }
    }

    let mut out = Vec::with_capacity(tris.len() * 18);
    for tri in &tris {
        let p = [pos[tri[0] as usize], pos[tri[1] as usize], pos[tri[2] as usize]];
        let nrm = flat_normal(p[0], p[1], p[2]);
        for k in 0..3 { out.extend_from_slice(&[p[k][0], p[k][1], p[k][2], nrm[0], nrm[1], nrm[2]]); }
    }
    out
}

/// Bouche une boucle de bord simple (sommets dans le sens du bord).
fn fill_loop(lp: &[u32], pos: &mut Vec<[f32; 3]>, tris: &mut Vec<[u32; 3]>,
             keys: &mut HashSet<(i64, i64, i64)>, qk: &dyn Fn([f32; 3]) -> (i64, i64, i64),
             und: &mut HashMap<(u32, u32), u8>) {
        if lp.len() < 3 { return; }
        fn add_t(und: &mut HashMap<(u32, u32), u8>, tris: &mut Vec<[u32; 3]>, t: [u32; 3]) {
            for e in 0..3 { *und.entry(ekey(t[e], t[(e + 1) % 3])).or_insert(0) += 1; }
            tris.push(t);
        }
        if lp.len() == 3 { add_t(und, tris, [lp[0], lp[2], lp[1]]); return; }
        // d'abord : découpage en oreilles dans le plan moyen du trou (pas de pli créé)
        if let Some(ts) = ear_clip(lp, pos, und) {
            for t in ts { add_t(und, tris, [t[2], t[1], t[0]]); } // sens inverse du bord
            return;
        }
        let n = lp.len() as f32;
        let mut c = [0.0f32; 3];
        for &v in lp { let p = pos[v as usize]; c[0] += p[0] / n; c[1] += p[1] / n; c[2] += p[2] / n; }
        if keys.insert(qk(c)) {
            let ci = pos.len() as u32;
            pos.push(c);
            for i in 0..lp.len() { add_t(und, tris, [ci, lp[(i + 1) % lp.len()], lp[i]]); } // sens inverse du bord
        } else {
            // micro-trou (centre confondu avec un sommet) : éventail depuis un sommet du bord
            // choisi pour qu'aucune diagonale ne double une arête existante
            let n = lp.len();
            let k = (0..n).find(|&k| (2..n - 1).all(|j| !und.contains_key(&ekey(lp[k], lp[(k + j) % n])))).unwrap_or(0);
            for j in 1..n - 1 { add_t(und, tris, [lp[k], lp[(k + j + 1) % n], lp[(k + j) % n]]); }
        }
}

/// Triangulation « en oreilles » d'une boucle de bord projetée sur son plan moyen (normale de
/// Newell). Refuse toute diagonale qui doublerait une arête existante. None si la projection
/// n'est pas un polygone simple exploitable (on retombe alors sur l'éventail).
fn ear_clip(lp: &[u32], pos: &[[f32; 3]], und: &HashMap<(u32, u32), u8>) -> Option<Vec<[u32; 3]>> {
    let n = lp.len();
    if n < 4 || n > 1500 { return None; }
    let p3: Vec<[f64; 3]> = lp.iter().map(|&v| { let p = pos[v as usize]; [p[0] as f64, p[1] as f64, p[2] as f64] }).collect();
    let mut nn = [0.0f64; 3];
    for i in 0..n {
        let (a, b) = (p3[i], p3[(i + 1) % n]);
        nn[0] += (a[1] - b[1]) * (a[2] + b[2]);
        nn[1] += (a[2] - b[2]) * (a[0] + b[0]);
        nn[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
    if l < 1e-18 { return None; }
    let nz = [nn[0] / l, nn[1] / l, nn[2] / l];
    let t = if nz[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let cr = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let u = { let c = cr(t, nz); let m = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt(); [c[0] / m, c[1] / m, c[2] / m] };
    let v = cr(nz, u);
    let p2: Vec<[f64; 2]> = p3.iter().map(|p| [p[0] * u[0] + p[1] * u[1] + p[2] * u[2], p[0] * v[0] + p[1] * v[1] + p[2] * v[2]]).collect();
    let area2 = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 {
        guard += 1;
        if guard > n * n { return None; }
        let m = idx.len();
        let mut best: Option<(usize, f64)> = None;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (p2[ia], p2[ib], p2[ic]);
            let ar = area2(a, b, c);
            if ar <= 1e-14 { continue; } // sommet rentrant ou plat
            if und.contains_key(&ekey(lp[ia], lp[ic])) { continue; } // diagonale déjà présente
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic { return false; }
                let q = p2[j];
                area2(a, b, q) >= 0.0 && area2(b, c, q) >= 0.0 && area2(c, a, q) >= 0.0
            });
            if inside { continue; }
            // meilleure oreille = la plus « ronde » (évite les triangles aiguilles)
            let la = ((b[0]-a[0]).powi(2) + (b[1]-a[1]).powi(2)) + ((c[0]-b[0]).powi(2) + (c[1]-b[1]).powi(2)) + ((a[0]-c[0]).powi(2) + (a[1]-c[1]).powi(2));
            let q = ar / la.max(1e-30);
            if best.map_or(true, |(_, bq)| q > bq) { best = Some((k, q)); }
        }
        let (k, _) = best?;
        let m = idx.len();
        out.push([lp[idx[(k + m - 1) % m]], lp[idx[k]], lp[idx[(k + 1) % m]]]);
        idx.remove(k);
    }
    if area2(p2[idx[0]], p2[idx[1]], p2[idx[2]]) <= 0.0 { return None; }
    out.push([lp[idx[0]], lp[idx[1]], lp[idx[2]]]);
    Some(out)
}

#[derive(Default)]
pub struct Defects {
    pub holes: usize,        // nombre de trous (boucles de bord)
    pub face_errors: usize,  // nombre de faces en erreur (orientation / non-manifold)
    pub boundary: Vec<f32>,  // arêtes de bord (trous) : [x y z] * 2 * arêtes  -> rouge
    pub errors: Vec<f32>,    // arêtes en erreur : idem                         -> jaune
}

/// `verts` = sommets entrelacés [pos(3) nrm(3)] * 3 * triangles (format stl::Mesh).
pub fn analyze(verts: &[f32]) -> Defects { analyze_suivi(verts, false) }
/// Idem ; `suivi` = l'analyse fait avancer la jauge (chargement d'un fichier). Pas pendant un calcul qui
/// l'appelle en interne (réparation…) : il garde la main sur sa propre jauge.
pub fn analyze_suivi(verts: &[f32], suivi: bool) -> Defects {
    let ntri = verts.len() / 18;
    // Tolérance de soudure relative à la taille de l'objet (robuste quelle que soit l'échelle).
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for t in 0..ntri {
        for k in 0..3 { let o = t * 18 + k * 6; for d in 0..3 { lo[d] = lo[d].min(verts[o + d]); hi[d] = hi[d].max(verts[o + d]); } }
    }
    let ext = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6);
    let cell = ext * 2e-5;
    let q = |v: f32| (v / cell).round() as i64;
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::default();
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut idx: Vec<u32> = Vec::with_capacity(ntri * 3);
    for t in 0..ntri {
        if suivi && t % 65536 == 0 { crate::progress::set(0.5 * t as f32 / ntri.max(1) as f32); }
        for k in 0..3 {
            let o = t * 18 + k * 6;
            let key = (q(verts[o]), q(verts[o + 1]), q(verts[o + 2]));
            let id = *map.entry(key).or_insert_with(|| { pos.push([verts[o], verts[o + 1], verts[o + 2]]); (pos.len() - 1) as u32 });
            idx.push(id);
        }
    }
    // (count, somme des sens, triangles) par arête non orientée
    let mut edges: HashMap<(u32, u32), (u32, i32, Vec<u32>)> = HashMap::default();
    for t in 0..ntri {
        if suivi && t % 65536 == 0 { crate::progress::set(0.5 + 0.45 * t as f32 / ntri.max(1) as f32); }
        let tri = [idx[t * 3], idx[t * 3 + 1], idx[t * 3 + 2]];
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            let (lo, hi, dir) = if a < b { (a, b, 1) } else { (b, a, -1) };
            let ent = edges.entry((lo, hi)).or_insert((0, 0, Vec::new()));
            ent.0 += 1; ent.1 += dir; ent.2.push(t as u32);
        }
    }
    let mut boundary = Vec::new();
    let mut errors = Vec::new();
    let mut err_tris: HashSet<u32> = HashSet::default();
    let mut bnd_adj: HashMap<u32, Vec<u32>> = HashMap::default();
    for (&(lo, hi), (count, dirsum, tris)) in &edges {
        let (pa, pb) = (pos[lo as usize], pos[hi as usize]);
        if *count == 1 {
            boundary.extend_from_slice(&[pa[0], pa[1], pa[2], pb[0], pb[1], pb[2]]);
            bnd_adj.entry(lo).or_default().push(hi);
            bnd_adj.entry(hi).or_default().push(lo);
        } else if *count > 2 || (*count == 2 && *dirsum != 0) {
            errors.extend_from_slice(&[pa[0], pa[1], pa[2], pb[0], pb[1], pb[2]]);
            for &tt in tris { err_tris.insert(tt); }
        }
    }
    // trous = composantes connexes du graphe des arêtes de bord
    // Un vrai trou = boucle de bord d'au moins 3 arêtes (on ignore les micro-fissures isolées).
    let mut holes = 0usize;
    let mut seen: HashSet<u32> = HashSet::default();
    let keys: Vec<u32> = bnd_adj.keys().copied().collect();
    for start in keys {
        if seen.contains(&start) { continue; }
        let mut deg = 0usize;
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            if !seen.insert(v) { continue; }
            if let Some(ns) = bnd_adj.get(&v) { deg += ns.len(); for &n in ns { if !seen.contains(&n) { stack.push(n); } } }
        }
        if deg / 2 >= 3 { holes += 1; }
    }
    Defects { holes, face_errors: err_tris.len(), boundary, errors }
}

/// Jeu de segments GL colorés (uniforme), pour surligner bords/erreurs.
pub struct LineSet { program: glow::NativeProgram, vao: glow::NativeVertexArray, vbo: glow::NativeBuffer,
                     u_mvp: Option<glow::NativeUniformLocation>, u_color: Option<glow::NativeUniformLocation>, count: i32 }

const VS: &str = "#version 330 core\nlayout(location=0) in vec3 p;\nuniform mat4 uMVP;\nvoid main(){ gl_Position=uMVP*vec4(p,1.0); }";
const FS: &str = "#version 330 core\nuniform vec4 uColor;\nout vec4 f;\nvoid main(){ f=uColor; }";

impl LineSet {
    pub unsafe fn new(gl: &glow::Context) -> LineSet {
        let program = gl.create_program().unwrap();
        for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let s = gl.create_shader(ty).unwrap();
            gl.shader_source(s, src); gl.compile_shader(s);
            gl.attach_shader(program, s);
        }
        gl.link_program(program);
        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(vao));
        let vbo = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 3 * 4, 0);
        LineSet { program, vao, vbo, u_mvp: gl.get_uniform_location(program, "uMVP"), u_color: gl.get_uniform_location(program, "uColor"), count: 0 }
    }
    pub unsafe fn update(&mut self, gl: &glow::Context, verts: &[f32]) {
        gl.bind_vertex_array(Some(self.vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
        let bytes: &[u8] = core::slice::from_raw_parts(verts.as_ptr() as *const u8, verts.len() * 4);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        crate::cache3d::touche();
        self.count = (verts.len() / 3) as i32;
    }
    pub unsafe fn draw(&self, gl: &glow::Context, mvp: &[f32; 16], color: [f32; 4], width: f32) {
        if self.count == 0 { return; }
        gl.line_width(width);
        gl.use_program(Some(self.program));
        gl.uniform_matrix_4_f32_slice(self.u_mvp.as_ref(), false, mvp);
        gl.uniform_4_f32(self.u_color.as_ref(), color[0], color[1], color[2], color[3]);
        gl.bind_vertex_array(Some(self.vao));
        gl.draw_arrays(glow::LINES, 0, self.count);
        gl.line_width(1.0);
    }
}

/// Lissage interne : supprime toute la géométrie qui n'est pas visible de l'extérieur
/// (excroissances, parois et coques internes). Voxelisation de la surface (fermée d'1 voxel
/// pour boucher les petites ouvertures), remplissage de l'extérieur depuis les bords de la
/// grille, puis on ne garde que les triangles qui touchent l'extérieur.
/// Renvoie (sommets conservés, nombre de triangles supprimés).
pub fn remove_internal(verts: &[f32], res: usize) -> (Vec<f32>, usize) {
    let ntri = verts.len() / 18;
    if ntri == 0 { return (verts.to_vec(), 0); }
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for c in verts.chunks_exact(6) { for d in 0..3 { lo[d] = lo[d].min(c[d]); hi[d] = hi[d].max(c[d]); } }
    let ext = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6);
    let cell = ext / res as f32;
    const PAD: i64 = 3;
    let dims: [i64; 3] = [0, 1, 2].map(|d| ((hi[d] - lo[d]) / cell).ceil() as i64 + 1 + 2 * PAD);
    let (nx, ny, nz) = (dims[0], dims[1], dims[2]);
    let id = |x: i64, y: i64, z: i64| ((z * ny + y) * nx + x) as usize;
    let cell_of = |p: [f32; 3]| -> [i64; 3] { [0, 1, 2].map(|d| ((p[d] - lo[d]) / cell).floor() as i64 + PAD) };
    // cellules touchées par chaque triangle (échantillonnage à ≤ 1/2 voxel)
    let tri_cells = |t: usize, f: &mut dyn FnMut([i64; 3])| {
        let p = |k: usize| { let o = t * 18 + k * 6; [verts[o], verts[o + 1], verts[o + 2]] };
        let (a, b, c) = (p(0), p(1), p(2));
        let len = |u: [f32; 3], v: [f32; 3]| ((u[0]-v[0]).powi(2) + (u[1]-v[1]).powi(2) + (u[2]-v[2]).powi(2)).sqrt();
        let n = ((len(a, b).max(len(b, c)).max(len(c, a)) / (cell * 0.5)).ceil() as usize).clamp(1, 4096);
        for i in 0..=n { for j in 0..=(n - i) {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            let w = 1.0 - u - v;
            f(cell_of([0, 1, 2].map(|d| a[d] * w + b[d] * u + c[d] * v)));
        }}
    };
    // 0 = libre, 1 = surface (dilatée), 2 = extérieur
    let mut g = vec![0u8; (nx * ny * nz) as usize];
    // GPU (option « Activer GPU ») : marquage dilaté + remplissage de l'extérieur sur la carte
    let gpu_ext = crate::gpu::exterior(crate::gpu::Req {
        tris: (0..ntri).flat_map(|t| (0..3).flat_map(move |k| { let o = t * 18 + k * 6; [verts[o], verts[o + 1], verts[o + 2]] })).collect(),
        org: lo, h: cell, off: PAD as i32, dims: [nx as i32, ny as i32, nz as i32], cube: true, grow6: 0 });
    if let Some(bits) = &gpu_ext {
        for (i, v) in g.iter_mut().enumerate() { if crate::gpu::bit(bits, i) { *v = 2; } }
        crate::progress::set(0.55);
    }
    if gpu_ext.is_none() {
    for t in 0..ntri {
        if t % 16384 == 0 { crate::progress::set(0.4 * t as f32 / ntri as f32); }
        tri_cells(t, &mut |c| {
            for dz in -1..=1 { for dy in -1..=1 { for dx in -1..=1 {
                g[id(c[0] + dx, c[1] + dy, c[2] + dz)] = 1;
            }}}
        });
    }
    crate::progress::set(0.4);
    let mut stack: Vec<(i64, i64, i64)> = vec![(0, 0, 0)];
    g[0] = 2;
    while let Some((x, y, z)) = stack.pop() {
        for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (a, b, c) = (x + dx, y + dy, z + dz);
            if a < 0 || b < 0 || c < 0 || a >= nx || b >= ny || c >= nz { continue; }
            let k = id(a, b, c);
            if g[k] == 0 { g[k] = 2; stack.push((a, b, c)); }
        }
    }
    crate::progress::set(0.55);
    } // fin du chemin processeur
    // triangle conservé s'il est à ≤ 2 voxels de l'extérieur (compense la dilatation)
    let mut out = Vec::with_capacity(verts.len());
    let mut removed = 0;
    for t in 0..ntri {
        if t % 16384 == 0 { crate::progress::set(0.55 + 0.45 * t as f32 / ntri as f32); }
        let mut keep = false;
        tri_cells(t, &mut |c| {
            if keep { return; }
            'n: for dz in -2..=2 { for dy in -2..=2 { for dx in -2..=2 {
                let (a, b, cc) = (c[0] + dx, c[1] + dy, c[2] + dz);
                if a >= 0 && b >= 0 && cc >= 0 && a < nx && b < ny && cc < nz && g[id(a, b, cc)] == 2 { keep = true; break 'n; }
            }}}
        });
        if keep { out.extend_from_slice(&verts[t * 18..t * 18 + 18]); } else { removed += 1; }
    }
    (out, removed)
}

