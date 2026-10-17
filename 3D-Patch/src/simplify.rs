//! Simplification / densification à nombre de triangles choisi.
//!  - Réduction : fusion d'arêtes guidée par l'erreur quadrique (Garland-Heckbert). Chaque
//!    fusion vérifie la condition de lien (pas d'arête non-manifold créée) et refuse les
//!    faces retournées -> un objet sans défaut reste sans défaut.
//!  - Augmentation : subdivision de Loop (lisse la forme), répétée jusqu'à dépasser la
//!    cible, puis réduction quadrique jusqu'au nombre exact demandé.

use glam::DVec3;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use crate::fasthash::{HashMap, HashSet};

/// Plafond de sécurité (mémoire / temps de calcul).
pub const MAX_TRIS: usize = 3_000_000;

struct Mesh { pos: Vec<DVec3>, tris: Vec<[u32; 3]> }

fn ekey(a: u32, b: u32) -> (u32, u32) { if a < b { (a, b) } else { (b, a) } }

fn weld(verts: &[f32]) -> Mesh {
    let ntri = verts.len() / 18;
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for c in verts.chunks_exact(6) { for d in 0..3 { lo[d] = lo[d].min(c[d]); hi[d] = hi[d].max(c[d]); } }
    let cell = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6) * 2e-5;
    let q = |v: f32| (v / cell).round() as i64;
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::default();
    let mut pos = Vec::new();
    let mut tris = Vec::with_capacity(ntri);
    for t in 0..ntri {
        let mut tri = [0u32; 3];
        for k in 0..3 {
            let o = t * 18 + k * 6;
            tri[k] = *map.entry((q(verts[o]), q(verts[o + 1]), q(verts[o + 2]))).or_insert_with(|| {
                pos.push(DVec3::new(verts[o] as f64, verts[o + 1] as f64, verts[o + 2] as f64));
                (pos.len() - 1) as u32
            });
        }
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] { tris.push(tri); }
    }
    Mesh { pos, tris }
}

fn to_verts(m: &Mesh) -> Vec<f32> {
    let mut out = Vec::with_capacity(m.tris.len() * 18);
    for t in &m.tris {
        let (a, b, c) = (m.pos[t[0] as usize], m.pos[t[1] as usize], m.pos[t[2] as usize]);
        let n = (b - a).cross(c - a).normalize_or_zero();
        for p in [a, b, c] { out.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32, n.x as f32, n.y as f32, n.z as f32]); }
    }
    out
}

/// Une passe de subdivision de Loop (×4 triangles).
fn loop_subdivide(m: &Mesh) -> Mesh {
    let nv = m.pos.len();
    // arête -> sommets opposés
    let mut opp: HashMap<(u32, u32), Vec<u32>> = HashMap::with_capacity_and_hasher(m.tris.len() * 2, Default::default());
    for t in &m.tris { for e in 0..3 { opp.entry(ekey(t[e], t[(e + 1) % 3])).or_default().push(t[(e + 2) % 3]); } }
    let mut nbr: Vec<Vec<u32>> = vec![Vec::new(); nv];
    let mut bnd: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (&(a, b), o) in &opp {
        nbr[a as usize].push(b); nbr[b as usize].push(a);
        if o.len() == 1 { bnd[a as usize].push(b); bnd[b as usize].push(a); }
    }
    // nouveaux sommets « pairs » (anciens repositionnés)
    let mut pos: Vec<DVec3> = (0..nv).map(|v| {
        let p = m.pos[v];
        if bnd[v].len() == 2 { return p * 0.75 + (m.pos[bnd[v][0] as usize] + m.pos[bnd[v][1] as usize]) * 0.125; }
        if !bnd[v].is_empty() { return p; }
        let n = nbr[v].len();
        if n < 3 { return p; }
        let beta = if n == 3 { 3.0 / 16.0 } else { 3.0 / (8.0 * n as f64) };
        let s: DVec3 = nbr[v].iter().map(|&u| m.pos[u as usize]).sum();
        p * (1.0 - n as f64 * beta) + s * beta
    }).collect();
    // sommets « impairs » (milieux d'arêtes)
    let mut mid: HashMap<(u32, u32), u32> = HashMap::with_capacity_and_hasher(opp.len(), Default::default());
    for (&(a, b), o) in &opp {
        let (pa, pb) = (m.pos[a as usize], m.pos[b as usize]);
        let p = if o.len() == 2 { (pa + pb) * 0.375 + (m.pos[o[0] as usize] + m.pos[o[1] as usize]) * 0.125 } else { (pa + pb) * 0.5 };
        mid.insert((a, b), pos.len() as u32);
        pos.push(p);
    }
    let mut tris = Vec::with_capacity(m.tris.len() * 4);
    for t in &m.tris {
        let [a, b, c] = *t;
        let (ab, bc, ca) = (mid[&ekey(a, b)], mid[&ekey(b, c)], mid[&ekey(c, a)]);
        tris.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
    }
    Mesh { pos, tris }
}

type Quad = [f64; 10];
fn plane_quad(n: DVec3, d: f64, w: f64) -> Quad {
    let (a, b, c) = (n.x, n.y, n.z);
    [a*a*w, a*b*w, a*c*w, a*d*w, b*b*w, b*c*w, b*d*w, c*c*w, c*d*w, d*d*w]
}
fn qadd(q: &mut Quad, r: &Quad) { for i in 0..10 { q[i] += r[i]; } }
fn qerr(q: &Quad, p: DVec3) -> f64 {
    let (x, y, z) = (p.x, p.y, p.z);
    q[0]*x*x + 2.0*q[1]*x*y + 2.0*q[2]*x*z + 2.0*q[3]*x + q[4]*y*y + 2.0*q[5]*y*z + 2.0*q[6]*y + q[7]*z*z + 2.0*q[8]*z + q[9]
}
/// Position optimale (minimise l'erreur) ; repli sur le meilleur de a, b, milieu.
fn qopt(q: &Quad, a: DVec3, b: DVec3) -> DVec3 {
    let m = glam::DMat3::from_cols(DVec3::new(q[0], q[1], q[2]), DVec3::new(q[1], q[4], q[5]), DVec3::new(q[2], q[5], q[7]));
    let det = m.determinant();
    let scale = (q[0] + q[4] + q[7]).abs().max(1e-30);
    if det.abs() > 1e-9 * scale * scale * scale {
        let p = m.inverse() * DVec3::new(-q[3], -q[6], -q[8]);
        // la position optimale ne doit pas partir loin de l'arête (sinon pics)
        let len = (b - a).length();
        if (p - (a + b) * 0.5).length() <= len * 2.0 { return p; }
    }
    let mid = (a + b) * 0.5;
    [a, b, mid].into_iter().min_by(|x, y| qerr(q, *x).partial_cmp(&qerr(q, *y)).unwrap_or(Ordering::Equal)).unwrap()
}

#[derive(PartialEq)]
/// Candidat compact (20 octets) : coût en bits de f32 positif => comparaison ENTIÈRE (rapide).
struct Cand { cost: u32, a: u32, b: u32, sa: u32, sb: u32 }
impl Eq for Cand {}
impl Ord for Cand { fn cmp(&self, o: &Self) -> Ordering { o.cost.cmp(&self.cost) } }
impl PartialOrd for Cand { fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) } }

/// Voisins (anneau) d'un sommet.
fn ring(vt: &[Vec<u32>], tris: &[[u32; 3]], v: u32) -> Vec<u32> {
    let mut s: Vec<u32> = vt[v as usize].iter().flat_map(|&t| tris[t as usize]).filter(|&u| u != v).collect();
    s.sort_unstable(); s.dedup(); s
}

/// Fusion b -> a autorisée ? (arête manifold, condition de lien, aucune face retournée/écrasée)
/// Renvoie la quadrique combinée si oui.
/// Anneau dans un tampon réutilisé (pas d'allocation par fusion).
fn ring_into(vt: &[Vec<u32>], tris: &[[u32; 3]], v: u32, out: &mut Vec<u32>) {
    out.clear();
    for &t in &vt[v as usize] { for u in tris[t as usize] { if u != v { out.push(u); } } }
    out.sort_unstable(); out.dedup();
}
thread_local! { static SCRATCH: std::cell::RefCell<(Vec<u32>, Vec<u32>)> = std::cell::RefCell::new((Vec::with_capacity(64), Vec::with_capacity(64))); }

fn collapse_ok(m: &Mesh, vt: &[Vec<u32>], quad: &[Quad], a: u32, b: u32) -> Option<Quad> {
    let mut shared = [0u32; 2]; let mut ns = 0;
    for &t in &vt[a as usize] { if m.tris[t as usize].contains(&b) { if ns == 2 { return None; } shared[ns] = t; ns += 1; } }
    if ns == 0 { return None; }
    let shared = &shared[..ns];
    let common = SCRATCH.with(|sc| {
        let (na, nb) = &mut *sc.borrow_mut();
        ring_into(vt, &m.tris, a, na); ring_into(vt, &m.tris, b, nb);
        na.iter().filter(|u| nb.binary_search(u).is_ok()).count()
    });
    if common != shared.len() { return None; }
    let mut q = quad[a as usize]; qadd(&mut q, &quad[b as usize]);
    let p = qopt(&q, m.pos[a as usize], m.pos[b as usize]);
    for &v in &[a, b] {
        for &t in &vt[v as usize] {
            if shared.contains(&t) { continue; }
            let tri = m.tris[t as usize];
            let pts = tri.map(|u| m.pos[u as usize]);
            let n0 = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
            let npts = tri.map(|u| if u == a || u == b { p } else { m.pos[u as usize] });
            let n1 = (npts[1] - npts[0]).cross(npts[2] - npts[0]);
            if n1.length_squared() < 1e-24 || n0.normalize_or_zero().dot(n1.normalize_or_zero()) < 0.2 { return None; }
        }
    }
    Some(q)
}

/// Applique la fusion b -> a (position optimale de la quadrique combinée).
fn do_collapse(m: &mut Mesh, alive: &mut [bool], vt: &mut [Vec<u32>], quad: &mut [Quad], a: u32, b: u32, q: Quad, live: &mut usize) {
    let p = qopt(&q, m.pos[a as usize], m.pos[b as usize]);
    let mut sh = [0u32; 2]; let mut ns = 0;
    for &t in vt[a as usize].iter() { if ns < 2 && m.tris[t as usize].contains(&b) { sh[ns] = t; ns += 1; } }
    let shared = &sh[..ns];
    for &t in shared { alive[t as usize] = false; *live -= 1; }
    let bt = std::mem::take(&mut vt[b as usize]);
    for t in bt {
        if !alive[t as usize] { continue; }
        for u in m.tris[t as usize].iter_mut() { if *u == b { *u = a; } }
        vt[a as usize].push(t);
    }
    vt[a as usize].retain(|t| alive[*t as usize]);
    for &t in shared { for &u in &m.tris[t as usize] { if u != a && u != b { vt[u as usize].retain(|x| *x != t); } } }
    m.pos[a as usize] = p;
    quad[a as usize] = q;
}

/// Réduction quadrique jusqu'à `target` triangles (ou blocage si plus aucune fusion sûre).
fn qem_decimate(m: &mut Mesh, target: usize) {
    let tq = std::time::Instant::now();
    let nv = m.pos.len();
    let mut alive = vec![true; m.tris.len()];
    let mut vt: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (t, tri) in m.tris.iter().enumerate() { for &v in tri { vt[v as usize].push(t as u32); } }
    let mut quad: Vec<Quad> = vec![[0.0; 10]; nv];
    let mut ecount: HashMap<(u32, u32), u8> = HashMap::with_capacity_and_hasher(m.tris.len() * 2, Default::default());
    for tri in &m.tris {
        let (a, b, c) = (m.pos[tri[0] as usize], m.pos[tri[1] as usize], m.pos[tri[2] as usize]);
        let cr = (b - a).cross(c - a);
        let area = cr.length();
        if area <= 0.0 { continue; }
        let n = cr / area;
        let q = plane_quad(n, -n.dot(a), area);
        for &v in tri { qadd(&mut quad[v as usize], &q); }
        for e in 0..3 { *ecount.entry(ekey(tri[e], tri[(e + 1) % 3])).or_insert(0) += 1; }
    }
    // arêtes de bord : plan perpendiculaire fortement pondéré => le bord ne bouge pas
    for tri in &m.tris {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            if ecount[&ekey(a, b)] != 1 { continue; }
            let (pa, pb, pc) = (m.pos[a as usize], m.pos[b as usize], m.pos[tri[(e + 2) % 3] as usize]);
            let fnrm = (pb - pa).cross(pc - pa);
            let n = (pb - pa).cross(fnrm).normalize_or_zero();
            let q = plane_quad(n, -n.dot(pa), (pb - pa).length_squared() * 1000.0);
            qadd(&mut quad[a as usize], &q); qadd(&mut quad[b as usize], &q);
        }
    }
    drop(ecount);
    let mut live = m.tris.len();
    let (start, span) = (live, (live.saturating_sub(target)).max(1));
    if std::env::var_os("QEM_PROF").is_some() { eprintln!("  [simp] init quadriques : {:.0} ms", tq.elapsed().as_secs_f32() * 1000.0); }

    // --- Phase 1 : VAGUES parallèles tant que le maillage est gros (file de priorité trop lente
    //     au-delà de quelques centaines de milliers de triangles : défauts de cache) ---
    let batch_floor = (target * 2).max(200_000);
    let nthreads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    let mut touched = vec![0u32; nv];
    let mut pass = 0u32;
    while live > batch_floor {
        pass += 1;
        // arêtes vivantes (chaque arête intérieure une seule fois : sens u < v dans sa face)
        let mut edges: Vec<(u32, u32)> = Vec::with_capacity(live * 3 / 2 + 16);
        for (t, tri) in m.tris.iter().enumerate() {
            if !alive[t] { continue; }
            for e in 0..3 { let (u, v) = (tri[e], tri[(e + 1) % 3]); if u < v { edges.push((u, v)); } }
        }
        // coûts en parallèle sur tous les cœurs
        let mut cand: Vec<(u32, u32, u32)> = vec![(0, 0, 0); edges.len()];
        {
            let (quad, pos) = (&quad, &m.pos);
            let chunk = (edges.len() / nthreads).max(1);
            std::thread::scope(|sc| {
                for (es, cs) in edges.chunks(chunk).zip(cand.chunks_mut(chunk)) {
                    sc.spawn(move || {
                        for (&(a, b), c) in es.iter().zip(cs.iter_mut()) {
                            let mut q = quad[a as usize]; qadd(&mut q, &quad[b as usize]);
                            let p = qopt(&q, pos[a as usize], pos[b as usize]);
                            *c = ((qerr(&q, p).max(0.0) as f32).to_bits(), a, b);
                        }
                    });
                }
            });
        }
        drop(edges);
        // on ne traite que la moitié la moins coûteuse (reste proche de l'ordre glouton exact) :
        // sélection partielle puis tri de cette moitié seulement
        let take = (cand.len() / 2).max(1);
        if take < cand.len() { cand.select_nth_unstable_by_key(take, |c| c.0); }
        cand[..take].sort_unstable_by_key(|c| c.0);
        let mut done = 0usize;
        for &(_, a, b) in &cand[..take] {
            if live <= batch_floor { break; }
            if touched[a as usize] == pass || touched[b as usize] == pass { continue; }
            if let Some(q) = collapse_ok(m, &vt, &quad, a, b) {
                do_collapse(m, &mut alive, &mut vt, &mut quad, a, b, q, &mut live);
                done += 1;
                // anneau du sommet fusionné figé pour cette vague (données des tests restées valides)
                touched[a as usize] = pass;
                for u in ring(&vt, &m.tris, a) { touched[u as usize] = pass; }
            }
        }
        crate::progress::set((start - live) as f32 / span as f32);
        if std::env::var_os("QEM_PROF").is_some() { eprintln!("  vague {pass} : {} arêtes, {done} fusions, reste {live}", cand.len()); }
        if done == 0 { break; }
    }

    if std::env::var_os("QEM_PROF").is_some() { eprintln!("  [simp] fin des vagues : {live} triangles — {:.0} ms", tq.elapsed().as_secs_f32() * 1000.0); }
    // --- Phase 2 : file de priorité exacte (petit maillage) ---
    let mut stamp = vec![0u32; nv];
    let mut heap = BinaryHeap::with_capacity(live * 2);
    let push = |heap: &mut BinaryHeap<Cand>, quad: &[Quad], pos: &[DVec3], stamp: &[u32], a: u32, b: u32| {
        let mut q = quad[a as usize]; qadd(&mut q, &quad[b as usize]);
        let p = qopt(&q, pos[a as usize], pos[b as usize]);
        heap.push(Cand { cost: (qerr(&q, p).max(0.0) as f32).to_bits(), a, b, sa: stamp[a as usize], sb: stamp[b as usize] });
    };
    for (t, tri) in m.tris.iter().enumerate() {
        if !alive[t] { continue; }
        for e in 0..3 { let (u, v) = (tri[e], tri[(e + 1) % 3]); if u < v { push(&mut heap, &quad, &m.pos, &stamp, u, v); } }
    }
    let mut it = 0usize;
    while live > target {
        // purge des candidats périmés quand la file enfle (sinon l'extraction devient lente)
        if heap.len() > 6 * live + 1_000_000 { heap.retain(|c| stamp[c.a as usize] == c.sa && stamp[c.b as usize] == c.sb); }
        let Some(c) = heap.pop() else { break };
        it += 1;
        if it % 4096 == 0 { crate::progress::set((start - live) as f32 / span as f32); }
        let (a, b) = (c.a, c.b);
        if stamp[a as usize] != c.sa || stamp[b as usize] != c.sb { continue; }
        let Some(q) = collapse_ok(m, &vt, &quad, a, b) else { continue };
        do_collapse(m, &mut alive, &mut vt, &mut quad, a, b, q, &mut live);
        stamp[a as usize] += 1; stamp[b as usize] += 1;
        for u in ring(&vt, &m.tris, a) { push(&mut heap, &quad, &m.pos, &stamp, a, u); }
    }
    let tris = std::mem::take(&mut m.tris);
    m.tris = tris.into_iter().zip(alive).filter(|(_, ok)| *ok).map(|(t, _)| t).collect();
}

/// Ramène l'objet à environ `target` triangles (réduction ou augmentation lissée).
pub fn resample(verts: &[f32], target: usize) -> Vec<f32> {
    let target = target.clamp(4, MAX_TRIS);
    let prof = std::env::var_os("QEM_PROF").is_some();
    let t0 = std::time::Instant::now();
    let lap = |m: &str, t: &std::time::Instant| if prof { eprintln!("  [simp] {m} : {:.0} ms", t.elapsed().as_secs_f32() * 1000.0); };
    let mut m = weld(verts);
    lap("soudure", &t0); let t0 = std::time::Instant::now();
    if m.tris.is_empty() { return verts.to_vec(); }
    crate::progress::span(0.0, 0.3);
    let mut levels = 0;
    { let mut n = m.tris.len(); while n < target && n * 4 <= MAX_TRIS { n *= 4; levels += 1; } }
    for l in 0..levels { m = loop_subdivide(&m); crate::progress::set((l + 1) as f32 / levels as f32); }
    crate::progress::span(0.3, 0.9);
    if m.tris.len() > target { qem_decimate(&mut m, target); }
    lap("réduction", &t0); let t0 = std::time::Instant::now();
    crate::progress::span(0.9, 1.0);
    let out = to_verts(&m);
    // garantie 0 défaut : le lissage peut rapprocher des sommets presque confondus au point
    // qu'ils se soudent -> on repasse la réparation seulement si l'analyse trouve quelque chose
    let mut out = out;
    for _ in 0..5 {
        let d = crate::defects::analyze(&out);
        if d.holes == 0 && d.face_errors == 0 { break; }
        out = crate::defects::repair(&out);
    }
    lap("contrôle final", &t0);
    out
}
