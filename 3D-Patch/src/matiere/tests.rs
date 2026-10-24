use super::*;

fn boite(dx: f32, dy: f32, dz: f32) -> Vec<f32> {
    let c = |x: f32, y: f32, z: f32| [x * dx, y * dy, z * dz];
    let quads = [
        [c(0., 0., 0.), c(0., 1., 0.), c(1., 1., 0.), c(1., 0., 0.)], [c(0., 0., 1.), c(1., 0., 1.), c(1., 1., 1.), c(0., 1., 1.)],
        [c(0., 0., 0.), c(1., 0., 0.), c(1., 0., 1.), c(0., 0., 1.)], [c(1., 1., 0.), c(0., 1., 0.), c(0., 1., 1.), c(1., 1., 1.)],
        [c(1., 0., 0.), c(1., 1., 0.), c(1., 1., 1.), c(1., 0., 1.)], [c(0., 1., 0.), c(0., 0., 0.), c(0., 0., 1.), c(0., 1., 1.)],
    ];
    let mut v = Vec::new();
    for q in quads { for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] { for p in tri { v.extend_from_slice(&p); v.extend_from_slice(&[0., 0., 0.]); } } }
    v
}

fn calque(m: motifs::Motif, taille: f32, prof: f32, mode: projection::Mode) -> Calque {
    Calque {
        relief: source::Relief { source: source::Source::Motif(motifs::Params { motif: m, ..Default::default() }), inverser: false, contraste: 1.0 },
        proj: projection::Params { mode, taille_u: taille, taille_v: taille, ..Default::default() },
        profondeur: prof, sens: Sens::Sortant, melange: Melange::Ajouter, opacite: 1.0, visible: true,
    }
}

fn volume(v: &[f32]) -> f64 {
    v.chunks_exact(18).map(|t| {
        let (a, b, c) = ([t[0] as f64, t[1] as f64, t[2] as f64], [t[6] as f64, t[7] as f64, t[8] as f64], [t[12] as f64, t[13] as f64, t[14] as f64]);
        (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
    }).sum()
}

#[test]
fn cube_texture_etanche() {
    let v = boite(20.0, 20.0, 20.0);
    let faces = vec![1u32; v.len() / 18];
    for m in [motifs::Motif::Briques, motifs::Motif::Hexagones, motifs::Motif::Pierres] {
        let r = calculer(&v, &[calque(m, 8.0, 0.6, projection::Mode::Triplanaire)], &faces, &Reglages { resolution: 0.3, tolerance: 0.03, ..Default::default() }, Mode::Final).unwrap();
        let d = crate::defects::analyze(&r.verts);
        assert_eq!((d.holes, d.face_errors), (0, 0), "{m:?} : défauts");
        assert!(volume(&r.verts) > 8000.0, "{m:?} : le relief sortant doit ajouter du volume");
        eprintln!("{m:?} : {} → {} triangles", r.avant, r.apres);
    }
}

#[test]
fn masque_dessus_intact() {
    let v = boite(20.0, 20.0, 10.0);
    let gel = masque_angle(&v, 10.0, 0.0);
    let faces: Vec<u32> = gel.iter().map(|&g| if g { 0 } else { 1 }).collect();
    let r = calculer(&v, &[calque(motifs::Motif::Dalles, 5.0, 1.0, projection::Mode::Cube)], &faces, &Reglages::default(), Mode::Final).unwrap();
    let zmax = r.verts.chunks_exact(6).map(|q| q[2]).fold(f32::MIN, f32::max);
    assert!((zmax - 10.0).abs() < 1e-4, "le dessus masqué a bougé : {zmax}");
    let d = crate::defects::analyze(&r.verts);
    assert_eq!((d.holes, d.face_errors), (0, 0));
}

/// Fidélité : sur une plaque (relief vers +Z, projection à plat), la hauteur obtenue sous chaque point
/// suit la hauteur voulue à la tolérance près, avec bien moins de triangles qu'un découpage uniforme.
#[test]
fn fidelite_et_economie() {
    let v = boite(40.0, 40.0, 2.0);
    let gel = masque_angle(&v, 0.0, 10.0); // dessous intact
    let faces: Vec<u32> = gel.iter().map(|&g| if g { 0 } else { 1 }).collect();
    let mut c = calque(motifs::Motif::Vagues, 10.0, 1.0, projection::Mode::Plan);
    c.proj.axe = [0.0, 0.0, 1.0];
    let reg = Reglages { resolution: 0.1, tolerance: 0.01, ..Default::default() };
    let r = calculer(&v, &[c.clone()], &faces, &reg, Mode::Final).unwrap();
    // triangles du dessus déplacés : on les retrouve par leur projection XY
    let dessus: Vec<[[f32; 3]; 3]> = r.verts.chunks_exact(18).map(|t| [[t[0], t[1], t[2]], [t[6], t[7], t[8]], [t[12], t[13], t[14]]])
        .filter(|t| t.iter().all(|q| q[2] > 1.5)).collect();
    let pr = projection::Projection::new(c.proj);
    let voulu = |x: f32, y: f32| 2.0 + pr.hauteur([x, y, 2.0], [0.0, 0.0, 1.0], &|u, v| c.relief.lire(u, v));
    let mut pire = 0.0f32;
    let mut s = 12345u32;
    let mut alea = || { s ^= s << 13; s ^= s >> 17; s ^= s << 5; (s % 10000) as f32 / 10000.0 };
    for _ in 0..3000 {
        let (x, y) = (2.0 + alea() * 36.0, 2.0 + alea() * 36.0);
        for t in &dessus {
            let (a, b, cc) = (t[0], t[1], t[2]);
            let d = (b[1] - cc[1]) * (a[0] - cc[0]) + (cc[0] - b[0]) * (a[1] - cc[1]);
            if d.abs() < 1e-12 { continue; }
            let l1 = ((b[1] - cc[1]) * (x - cc[0]) + (cc[0] - b[0]) * (y - cc[1])) / d;
            let l2 = ((cc[1] - a[1]) * (x - cc[0]) + (a[0] - cc[0]) * (y - cc[1])) / d;
            let l3 = 1.0 - l1 - l2;
            if l1 >= -1e-5 && l2 >= -1e-5 && l3 >= -1e-5 {
                let z = l1 * a[2] + l2 * b[2] + l3 * cc[2];
                pire = pire.max((z - voulu(x, y)).abs());
                break;
            }
        }
    }
    eprintln!("FIDÉLITÉ : écart max {pire:.4} mm (tolérance {}), {} triangles", reg.tolerance, r.apres);
    assert!(pire < reg.tolerance * 2.5, "écart {pire} mm");
}

/// Économie : un motif à grands plateaux (dalles) n'est découpé finement qu'au bord des joints.
#[test]
fn economie_sur_plateaux() {
    let v = boite(40.0, 40.0, 2.0);
    let gel = masque_angle(&v, 0.0, 10.0);
    let faces: Vec<u32> = gel.iter().map(|&g| if g { 0 } else { 1 }).collect();
    let mut c = calque(motifs::Motif::Dalles, 20.0, 1.0, projection::Mode::Plan);
    c.proj.axe = [0.0, 0.0, 1.0];
    let reg = Reglages { resolution: 0.1, tolerance: 0.01, ..Default::default() };
    let r = calculer(&v, &[c], &faces, &reg, Mode::Final).unwrap();
    let uniforme = (40.0 * 40.0 / (reg.resolution * reg.resolution) * 2.0) as usize;
    eprintln!("ÉCONOMIE : {} triangles pour le dessus et les côtés (dessus seul en découpage uniforme ≈ {uniforme})", r.apres);
    assert!(r.apres < uniforme / 4, "pas assez économe : {} triangles", r.apres);
    let d = crate::defects::analyze(&r.verts);
    assert_eq!((d.holes, d.face_errors), (0, 0));
}

/// Banc d'essai sur un vrai fichier :
/// MAT_BENCH=objet.stl cargo test --release matiere::tests::banc -- --ignored --nocapture
#[test]
#[ignore]
fn banc() {
    let p = std::env::var("MAT_BENCH").expect("MAT_BENCH=fichier");
    let mut v = crate::charger_objets(std::path::Path::new(&p)).unwrap().remove(0).0;
    let t = std::time::Instant::now();
    let d0 = crate::defects::analyze(&v);
    if d0.holes + d0.face_errors > 0 { v = crate::defects::repair(&v); }
    eprintln!("objet d'origine : {} trous, {} faces → réparé en {:.2} s", d0.holes, d0.face_errors, t.elapsed().as_secs_f32());
    let (lo, hi) = (0..3).fold(([f32::MAX; 3], [f32::MIN; 3]), |(mut lo, mut hi), d| { for q in v.chunks_exact(6) { lo[d] = lo[d].min(q[d]); hi[d] = hi[d].max(q[d]); } (lo, hi) });
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    let faces = vec![1u32; v.len() / 18];
    let taille = diag / 10.0;
    let mut c = calque(motifs::Motif::Damier, taille, diag / 300.0, projection::Mode::Triplanaire);
    c.proj.centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let reg = Reglages { resolution: resolution_conseillee(diag, taille), tolerance: diag / 3000.0, max_triangles: 750_000, adoucir: 0.0, plancher: None };
    for mode in [Mode::Apercu, Mode::Final] {
        let t = std::time::Instant::now();
        let r = calculer(&v, &[c.clone()], &faces, &reg, mode).unwrap();
        eprintln!("{mode:?} : {} → {} → {} triangles en {:.2} s  {:?}", r.avant, r.subdivises, r.apres, t.elapsed().as_secs_f32(), r.etapes);
        if mode == Mode::Final {
            let d = crate::defects::analyze(&r.verts);
            eprintln!("défauts : {} trous, {} faces", d.holes, d.face_errors);
            // ce que l'appli livre : réparation automatique si besoin
            let t = std::time::Instant::now();
            let mut w = r.verts.clone();
            for _ in 0..5 { let d = crate::defects::analyze(&w); if d.holes + d.face_errors == 0 { break; } w = crate::defects::repair(&w); }
            let d = crate::defects::analyze(&w);
            eprintln!("après réparation automatique : {} trous, {} faces, {} triangles ({:.2} s)", d.holes, d.face_errors, w.len() / 18, t.elapsed().as_secs_f32());
            // même calcul sans réduction : d'où viennent les défauts ?
            let r2 = calculer(&v, &[c.clone()], &faces, &Reglages { max_triangles: usize::MAX, ..reg }, mode).unwrap();
            let d2 = crate::defects::analyze(&r2.verts);
            eprintln!("sans réduction : {} triangles, {} trous, {} faces", r2.apres, d2.holes, d2.face_errors);
            let mm = maillage::souder(&r2.verts);
            let ext = (0..3).map(|d| hi[d] - lo[d]).fold(0.0f32, f32::max);
            let cell = ext * 2e-5;
            let mut k: Vec<[i64; 3]> = mm.p.iter().map(|q| [(q[0] / cell).round() as i64, (q[1] / cell).round() as i64, (q[2] / cell).round() as i64]).collect();
            k.sort();
            eprintln!("  sommets distincts confondus par l'analyse (case {cell:.4} mm) : {}", k.windows(2).filter(|w| w[0] == w[1]).count());
        }
    }
}

#[test]
#[ignore]
fn diag_cube() {
    let v = boite(20.0, 20.0, 20.0);
    let faces = vec![1u32; v.len() / 18];
    for prof in [0.001f32, 0.6] {
        for mode in [projection::Mode::Triplanaire, projection::Mode::Cube] {
            let r = calculer(&v, &[calque(motifs::Motif::Briques, 8.0, prof, mode)], &faces, &Reglages { resolution: 0.3, tolerance: 0.03, ..Default::default() }, Mode::Final).unwrap();
            let d = crate::defects::analyze(&r.verts);
            let m = maillage::souder(&r.verts);
            let ar = maillage::aretes(&m.t);
            let mut hist = [0usize; 5];
            for e in 0..ar.ab.len() { hist[((ar.debut[e + 1] - ar.debut[e]) as usize).min(4)] += 1; }
            eprintln!("prof {prof} {mode:?} : {} tri, défauts {} trous {} faces, arêtes par nb de triangles {:?}", r.apres, d.holes, d.face_errors, hist);
            for sg in d.errors.chunks_exact(6).take(4) { eprintln!("   erreur : ({:.4} {:.4} {:.4}) – ({:.4} {:.4} {:.4})", sg[0], sg[1], sg[2], sg[3], sg[4], sg[5]); }
            // sommets distincts très proches (< 0,001 mm)
            let mut pts: Vec<([i64; 3], usize)> = m.p.iter().enumerate().map(|(i, q)| ([(q[0] * 1000.0).round() as i64, (q[1] * 1000.0).round() as i64, (q[2] * 1000.0).round() as i64], i)).collect();
            pts.sort();
            let proches = pts.windows(2).filter(|w| w[0].0 == w[1].0).count();
            if proches > 0 { let w = pts.windows(2).find(|w| w[0].0 == w[1].0).unwrap(); eprintln!("   {proches} paires proches, ex. {:?} {:?} normales {:?} {:?}", m.p[w[0].1], m.p[w[1].1], m.n[w[0].1], m.n[w[1].1]); }
        }
    }
}

/// Dessous lisse : avec un plancher, aucun point ne passe sous le plateau, même texturé partout.
#[test]
fn plancher_respecte() {
    let v = boite(20.0, 20.0, 10.0);
    let faces = vec![1u32; v.len() / 18];
    let reg = Reglages { plancher: Some(0.0), ..Default::default() };
    let r = calculer(&v, &[calque(motifs::Motif::Pois, 5.0, 1.0, projection::Mode::Triplanaire)], &faces, &reg, Mode::Final).unwrap();
    let zmin = r.verts.chunks_exact(6).map(|q| q[2]).fold(f32::MAX, f32::min);
    assert!(zmin >= -1e-5, "point sous le plateau : {zmin}");
    let d = crate::defects::analyze(&r.verts);
    assert_eq!(d.holes, 0);
}

#[test]
#[ignore]
fn diag_fin() {
    let v = crate::charger_objets(std::path::Path::new(&std::env::var("MAT_BENCH").unwrap())).unwrap().remove(0).0;
    let (lo, hi) = (0..3).fold(([f32::MAX; 3], [f32::MIN; 3]), |(mut lo, mut hi), d| { for q in v.chunks_exact(6) { lo[d] = lo[d].min(q[d]); hi[d] = hi[d].max(q[d]); } (lo, hi) });
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    let t: f32 = std::env::var("MAT_TAILLE").ok().and_then(|x| x.parse().ok()).unwrap_or(0.7);
    let gel = masque_angle(&v, 0.0, 20.0);
    let faces: Vec<u32> = gel.iter().map(|&g| if g { 0 } else { 1 }).collect();
    let mut c = calque(motifs::Motif::Quadrillage, t, 0.73, projection::Mode::Triplanaire);
    c.proj.centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let reg = Reglages { resolution: resolution_conseillee(diag, t), tolerance: 0.73 / 40.0, ..Default::default() };
    eprintln!("res conseillée {} tol {}", reg.resolution, reg.tolerance);
    let r = calculer(&v, &[c], &faces, &reg, Mode::Apercu).unwrap();
    // plus longue arête des triangles du dessus (faces vers +Z) dans le résultat
    let mut lmax = 0f32;
    for q in r.verts.chunks_exact(18) {
        let (a, b, cc) = ([q[0], q[1], q[2]], [q[6], q[7], q[8]], [q[12], q[13], q[14]]);
        if q[5] > 0.9 { for (x, y) in [(a, b), (b, cc), (cc, a)] { lmax = lmax.max(maillage::long(maillage::sub(x, y))); } }
    }
    eprintln!("{} → {} → {} triangles, arête max du dessus {lmax:.2} mm, étapes {:?}", r.avant, r.subdivises, r.apres, r.etapes);
}
