//! Maillage indexé du moteur MATIÈRE : soudure de la soupe de triangles, normales lissées,
//! subdivision adaptative (une arête n'est coupée que si le relief l'exige) et déplacement.
//! Les sommets sont partagés : le résultat reste étanche si l'objet l'était.

use rayon::prelude::*;

pub type V = [f32; 3];
#[inline] pub fn add(a: V, b: V) -> V { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
#[inline] pub fn sub(a: V, b: V) -> V { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
#[inline] pub fn mul(a: V, k: f32) -> V { [a[0] * k, a[1] * k, a[2] * k] }
#[inline] pub fn dot(a: V, b: V) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
#[inline] pub fn cross(a: V, b: V) -> V { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
#[inline] pub fn norm(a: V) -> V { let l = dot(a, a).sqrt(); if l > 1e-20 { mul(a, 1.0 / l) } else { [0.0, 0.0, 1.0] } }
#[inline] pub fn long(a: V) -> f32 { dot(a, a).sqrt() }

/// Maillage indexé. `src` = face d'origine de chaque triangle (masques et calques).
#[derive(Clone, Default)]
pub struct Maillage {
    pub p: Vec<V>,
    pub n: Vec<V>,
    pub t: Vec<[u32; 3]>,
    pub src: Vec<u32>,
    /// Sommets à re-tester à la prochaine passe (nouveaux ou voisins d'un découpage) : une arête dont
    /// aucun sommet n'a changé a déjà été jugée et n'est pas réévaluée.
    pub actif: Vec<bool>,
}

/// Soude une soupe de triangles (6 flottants par sommet : position + normale, 18 par triangle).
/// Les sommets confondus deviennent un seul sommet, avec la même tolérance que l'analyse des défauts de
/// 3D-Patch (2e-5 × la plus grande dimension) : ce que l'appli voit fermé l'est aussi pour le moteur.
pub fn souder(verts: &[f32]) -> Maillage {
    let nt = verts.len() / 18;
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for q in verts.chunks_exact(6) { for d in 0..3 { lo[d] = lo[d].min(q[d]); hi[d] = hi[d].max(q[d]); } }
    let case = ((hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6) * 2e-5) as f64;
    let q = |x: f32| (x as f64 / case).round() as i64;
    let mut cles: Vec<([i64; 3], u32)> = (0..nt * 3).into_par_iter().map(|i| {
        let o = i * 6;
        ([q(verts[o]), q(verts[o + 1]), q(verts[o + 2])], i as u32)
    }).collect();
    cles.par_sort_unstable_by_key(|c| c.0);
    let mut id_de = vec![0u32; nt * 3];
    let mut p: Vec<V> = Vec::new();
    let mut prec: Option<[i64; 3]> = None;
    for (k, i) in &cles {
        if prec != Some(*k) { let o = *i as usize * 6; p.push([verts[o], verts[o + 1], verts[o + 2]]); prec = Some(*k); }
        id_de[*i as usize] = (p.len() - 1) as u32;
    }
    let mut t = Vec::with_capacity(nt);
    let mut src = Vec::with_capacity(nt);
    for f in 0..nt {
        let tri = [id_de[f * 3], id_de[f * 3 + 1], id_de[f * 3 + 2]];
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] { t.push(tri); src.push(f as u32); }
    }
    let actif = vec![true; p.len()];
    let mut m = Maillage { p, n: Vec::new(), t, src, actif };
    m.n = normales(&m.p, &m.t);
    m
}

/// Normales aux sommets, pondérées par l'angle de chaque triangle au sommet (indépendant du découpage).
pub fn normales(p: &[V], t: &[[u32; 3]]) -> Vec<V> {
    let contrib: Vec<[V; 3]> = t.par_iter().map(|tr| {
        let (a, b, c) = (p[tr[0] as usize], p[tr[1] as usize], p[tr[2] as usize]);
        let nf = norm(cross(sub(b, a), sub(c, a)));
        let ang = |o: V, x: V, y: V| { let (u, v) = (norm(sub(x, o)), norm(sub(y, o))); dot(u, v).clamp(-1.0, 1.0).acos() };
        [mul(nf, ang(a, b, c)), mul(nf, ang(b, c, a)), mul(nf, ang(c, a, b))]
    }).collect();
    let mut n = vec![[0f32; 3]; p.len()];
    for (tr, c) in t.iter().zip(&contrib) { for k in 0..3 { let i = tr[k] as usize; n[i] = add(n[i], c[k]); } }
    n.par_iter_mut().for_each(|x| *x = norm(*x));
    n
}

/// Arêtes uniques : (sommet a < sommet b, premier emplacement « triangle × 3 + côté » de la liste triée).
/// `occ` = emplacements (tri*3+côté) triés par arête ; `debut[e]..debut[e+1]` = occurrences de l'arête e.
pub struct Aretes { pub ab: Vec<(u32, u32)>, pub occ: Vec<u32>, pub debut: Vec<u32>, pub de_cote: Vec<u32> }

pub fn aretes(t: &[[u32; 3]]) -> Aretes {
    let mut k: Vec<(u64, u32)> = (0..t.len() * 3).into_par_iter().map(|s| {
        let tr = t[s / 3];
        let (a, b) = (tr[s % 3], tr[(s + 1) % 3]);
        (((a.min(b) as u64) << 32) | a.max(b) as u64, s as u32)
    }).collect();
    k.par_sort_unstable_by_key(|x| x.0);
    let mut ab = Vec::new();
    let mut debut = Vec::new();
    let mut de_cote = vec![0u32; t.len() * 3];
    let mut prec = u64::MAX;
    for (i, (cle, s)) in k.iter().enumerate() {
        if *cle != prec { ab.push(((cle >> 32) as u32, *cle as u32)); debut.push(i as u32); prec = *cle; }
        de_cote[*s as usize] = (ab.len() - 1) as u32;
    }
    debut.push(k.len() as u32);
    Aretes { ab, occ: k.into_iter().map(|x| x.1).collect(), debut, de_cote }
}

/// Une passe de subdivision par bissection du plus long côté (méthode de Rivara) : les arêtes pour
/// lesquelles `couper(arête)` est vrai reçoivent un sommet en leur milieu, et tout triangle touché est
/// toujours coupé d'abord par son plus long côté (la règle se propage jusqu'à ce que le maillage reste
/// conforme). Les triangles ne s'effilent jamais, même le long d'une arête vive où le relief change de
/// direction, et le découpage s'arrête toujours. Renvoie le nombre d'arêtes coupées.
pub fn passe(m: &mut Maillage, couper: &(dyn Fn(&Maillage, &Aretes, usize) -> bool + Sync)) -> usize {
    let chrono = std::time::Instant::now();
    let trace = std::env::var_os("MAT_TRACE").is_some();
    let mut top = |nom: &str| if trace { eprintln!("      {nom} : {:.0} ms", chrono.elapsed().as_secs_f32() * 1000.0); };
    let ar = aretes(&m.t);
    top("arêtes");
    let mut marques: Vec<bool> = (0..ar.ab.len()).into_par_iter().map(|e| {
        let (a, b) = ar.ab[e];
        (m.actif[a as usize] || m.actif[b as usize]) && couper(m, &ar, e)
    }).collect();
    if !marques.iter().any(|&x| x) { m.actif.iter_mut().for_each(|x| *x = false); return 0; }
    top("jugement");
    // plus long côté de chaque triangle (égalité départagée par le numéro d'arête : même choix partout)
    let p = &m.p;
    let longueur = |e: u32| { let (a, b) = ar.ab[e as usize]; long(sub(p[a as usize], p[b as usize])) };
    let plus_long: Vec<u8> = (0..m.t.len()).into_par_iter().map(|f| {
        let e = [ar.de_cote[f * 3], ar.de_cote[f * 3 + 1], ar.de_cote[f * 3 + 2]];
        let l = [longueur(e[0]), longueur(e[1]), longueur(e[2])];
        let mut k = 0usize;
        for j in 1..3 { if l[j] > l[k] || (l[j] == l[k] && e[j] < e[k]) { k = j; } }
        k as u8
    }).collect();
    // propagation : un triangle dont un côté est coupé doit aussi être coupé par son plus long côté
    loop {
        let ajout: Vec<u32> = (0..m.t.len()).into_par_iter().filter_map(|f| {
            let e = [ar.de_cote[f * 3], ar.de_cote[f * 3 + 1], ar.de_cote[f * 3 + 2]];
            let lg = e[plus_long[f] as usize];
            (!marques[lg as usize] && e.iter().any(|&x| marques[x as usize])).then_some(lg)
        }).collect();
        if ajout.is_empty() { break; }
        for e in ajout { marques[e as usize] = true; }
    }
    top("plus long + propagation");
    // nouveaux sommets : un par arête coupée
    let mut mil = vec![u32::MAX; ar.ab.len()];
    let mut nv = m.p.len() as u32;
    let mut coupees = Vec::new();
    for (e, &k) in marques.iter().enumerate() { if k { mil[e] = nv; nv += 1; coupees.push(e); } }
    // normale du nouveau point = somme des normales des faces qui partagent l'arête (chacune pèse un
    // angle plat) : exactement ce que donnerait le recalcul complet ; les sommets existants gardent la
    // leur (couper un triangle à plat ne change pas la somme des angles autour d'eux)
    let nouv: Vec<(V, V)> = coupees.par_iter().map(|&e| {
        let (a, b) = (ar.ab[e].0 as usize, ar.ab[e].1 as usize);
        let mut n = [0f32; 3];
        for s in &ar.occ[ar.debut[e] as usize..ar.debut[e + 1] as usize] {
            let tr = m.t[*s as usize / 3];
            let (x, y, z) = (m.p[tr[0] as usize], m.p[tr[1] as usize], m.p[tr[2] as usize]);
            n = add(n, norm(cross(sub(y, x), sub(z, x))));
        }
        (mul(add(m.p[a], m.p[b]), 0.5), norm(n))
    }).collect();
    for (p, n) in nouv { m.p.push(p); m.n.push(n); }
    // redécoupage : coupe par le plus long côté a-b (milieu M), puis chaque moitié par son côté coupé
    let morceaux: Vec<([[u32; 3]; 4], u8)> = (0..m.t.len()).into_par_iter().map(|f| {
        let tr = m.t[f];
        let mm = [mil[ar.de_cote[f * 3] as usize], mil[ar.de_cote[f * 3 + 1] as usize], mil[ar.de_cote[f * 3 + 2] as usize]];
        let mut o = [[0u32; 3]; 4];
        let k = plus_long[f] as usize;
        if mm[k] == u32::MAX { o[0] = tr; return (o, 1); } // intact (propagation : aucun côté coupé)
        let (a, b, c) = (tr[k], tr[(k + 1) % 3], tr[(k + 2) % 3]);
        let (mab, mbc, mca) = (mm[k], mm[(k + 1) % 3], mm[(k + 2) % 3]);
        let mut n = 0;
        // moitié (M, b, c) : coupée par b-c si besoin
        if mbc != u32::MAX { o[n] = [mab, b, mbc]; o[n + 1] = [mab, mbc, c]; n += 2; } else { o[n] = [mab, b, c]; n += 1; }
        // moitié (a, M, c) : coupée par c-a si besoin
        if mca != u32::MAX { o[n] = [a, mab, mca]; o[n + 1] = [mab, c, mca]; n += 2; } else { o[n] = [a, mab, c]; n += 1; }
        (o, n as u8)
    }).collect();
    let mut t = Vec::with_capacity(m.t.len() * 2);
    let mut src = Vec::with_capacity(m.t.len() * 2);
    let mut actif = vec![false; m.p.len()];
    for (f, (o, n)) in morceaux.iter().enumerate() {
        for k in 0..*n as usize {
            t.push(o[k]); src.push(m.src[f]);
            if *n > 1 { for &v in &o[k] { actif[v as usize] = true; } } // triangle redécoupé : ses sommets ont bougé de normale
        }
    }
    m.t = t; m.src = src; m.actif = actif;
    top("redécoupage");
    coupees.len()
}

/// Soupe de triangles (position + normale de face, 18 flottants par triangle).
pub fn soupe(p: &[V], t: &[[u32; 3]]) -> Vec<f32> {
    let mut out = vec![0f32; t.len() * 18];
    out.par_chunks_mut(18).zip(t.par_iter()).for_each(|(o, tr)| {
        let (a, b, c) = (p[tr[0] as usize], p[tr[1] as usize], p[tr[2] as usize]);
        let n = norm(cross(sub(b, a), sub(c, a)));
        for (k, q) in [a, b, c].iter().enumerate() { o[k * 6..k * 6 + 3].copy_from_slice(q); o[k * 6 + 3..k * 6 + 6].copy_from_slice(&n); }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Vec<f32> {
        let c = |x: f32, y: f32, z: f32| [x * 10.0, y * 10.0, z * 10.0];
        let quads = [
            [c(0., 0., 0.), c(0., 1., 0.), c(1., 1., 0.), c(1., 0., 0.)], [c(0., 0., 1.), c(1., 0., 1.), c(1., 1., 1.), c(0., 1., 1.)],
            [c(0., 0., 0.), c(1., 0., 0.), c(1., 0., 1.), c(0., 0., 1.)], [c(1., 1., 0.), c(0., 1., 0.), c(0., 1., 1.), c(1., 1., 1.)],
            [c(1., 0., 0.), c(1., 1., 0.), c(1., 1., 1.), c(1., 0., 1.)], [c(0., 1., 0.), c(0., 0., 0.), c(0., 0., 1.), c(0., 1., 1.)],
        ];
        let mut v = Vec::new();
        for q in quads { for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] { for p in tri { v.extend_from_slice(&p); v.extend_from_slice(&[0., 0., 0.]); } } }
        v
    }

    #[test]
    fn soudure_et_subdivision_etanches() {
        let mut m = souder(&cube());
        assert_eq!((m.p.len(), m.t.len()), (8, 12));
        // coupe toutes les arêtes plus longues que 3 mm : le cube reste fermé (chaque arête = 2 triangles)
        while passe(&mut m, &|m, ar, e| { let (a, b) = ar.ab[e]; long(sub(m.p[a as usize], m.p[b as usize])) > 3.0 }) > 0 {}
        let ar = aretes(&m.t);
        assert!((0..ar.ab.len()).all(|e| ar.debut[e + 1] - ar.debut[e] == 2), "arête non partagée par 2 triangles");
        let d = crate::defects::analyze(&soupe(&m.p, &m.t));
        assert_eq!((d.holes, d.face_errors), (0, 0));
        // coupe partielle (1 ou 2 côtés seulement) : toujours fermé
        let mut m2 = souder(&cube());
        passe(&mut m2, &|m, ar, e| { let (a, b) = ar.ab[e]; m.p[a as usize][2] > 5.0 || m.p[b as usize][2] > 5.0 });
        let d = crate::defects::analyze(&soupe(&m2.p, &m2.t));
        assert_eq!((d.holes, d.face_errors), (0, 0));
    }
}

/// Élimine les triangles en aiguille (hauteur < `seuil` mm sous leur plus long côté) posés à plat contre
/// leur voisin, en retournant ce côté : les deux nouveaux triangles couvrent exactement la même surface,
/// sans aiguille. Sans cela, couper le long côté placerait un point
/// presque sur le sommet opposé. Renvoie le nombre de retournements.
pub fn aiguilles(m: &mut Maillage, seuil: f32) -> usize {
    let mut total = 0;
    for _ in 0..3 {
        let ar = aretes(&m.t);
        let p = &m.p;
        // aiguilles : (triangle, côté le plus long) dont le sommet opposé se projette dans le côté
        let cand: Vec<(u32, usize)> = (0..m.t.len()).into_par_iter().filter_map(|f| {
            let tr = m.t[f];
            let l = |k: usize| long(sub(p[tr[(k + 1) % 3] as usize], p[tr[k] as usize]));
            let k = (0..3).max_by(|&x, &y| l(x).total_cmp(&l(y))).unwrap();
            let (a, b, c) = (p[tr[k] as usize], p[tr[(k + 1) % 3] as usize], p[tr[(k + 2) % 3] as usize]);
            let ab = sub(b, a);
            let lab = long(ab);
            if lab < 1e-12 { return None; }
            let h = long(cross(ab, sub(c, a))) / lab;
            let t = dot(sub(c, a), ab) / (lab * lab);
            (h < seuil && t > 0.02 && t < 0.98).then_some((f as u32, k))
        }).collect();
        if cand.is_empty() { break; }
        let mut pris = vec![false; m.t.len()];
        let mut n = 0;
        for (f, k) in cand {
            let f = f as usize;
            if pris[f] { continue; }
            let e = ar.de_cote[f * 3 + k] as usize;
            let occ = &ar.occ[ar.debut[e] as usize..ar.debut[e + 1] as usize];
            if occ.len() != 2 { continue; }
            let g = occ.iter().map(|s| *s as usize / 3).find(|&x| x != f).unwrap();
            if pris[g] { continue; }
            let tr = m.t[f];
            let (a, b, c) = (tr[k], tr[(k + 1) % 3], tr[(k + 2) % 3]);
            let d = *m.t[g].iter().find(|&&v| v != a && v != b).unwrap();
            if d == c { continue; }
            // le côté c-d ne doit pas déjà exister
            let existe = {
                let (lo, hi) = (c.min(d), c.max(d));
                ar.ab.binary_search(&(lo, hi)).is_ok()
            };
            if existe { continue; }
            // nouveaux triangles (a, d, c) et (d, b, c) : ni plats ni retournés par rapport à l'ancien couple
            let nf = |x: u32, y: u32, z: u32| cross(sub(m.p[y as usize], m.p[x as usize]), sub(m.p[z as usize], m.p[x as usize]));
            // seulement entre deux triangles à plat l'un de l'autre (exports CAO) : sur une surface courbe,
            // retourner changerait la forme
            if dot(norm(nf(a, b, c)), norm(nf(b, a, d))) < 0.999 { continue; }
            let ref_n = add(nf(a, b, c), nf(b, a, d));
            let (n1, n2) = (nf(a, d, c), nf(d, b, c));
            if dot(n1, ref_n) <= 0.0 || dot(n2, ref_n) <= 0.0 { continue; }
            let lim = 1e-9 * dot(ref_n, ref_n).sqrt();
            if long(n1) < lim || long(n2) < lim { continue; }
            m.t[f] = [a, d, c];
            m.t[g] = [d, b, c];
            pris[f] = true; pris[g] = true;
            n += 1;
        }
        if n == 0 { break; }
        total += n;
    }
    if total > 0 { m.n = normales(&m.p, &m.t); }
    total
}
