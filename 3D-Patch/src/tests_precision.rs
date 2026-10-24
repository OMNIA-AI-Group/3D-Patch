//! Tests de précision : chaque calcul de l'appli comparé à une valeur exacte (analytique) ou à
//! l'objet de départ. Lancer : `cargo test --release tests_precision`.

use super::*;
use std::num::NonZeroU32;
use manifold_rust::manifold::Manifold;

// ---------- outils de mesure (en f64, indépendants du code testé) ----------

/// Volume signé (théorème de la divergence) : > 0 si les faces sont orientées vers l'extérieur.
fn volume(v: &[f32]) -> f64 {
    v.chunks_exact(18).map(|t| {
        let p = |k: usize| glam::DVec3::new(t[k * 6] as f64, t[k * 6 + 1] as f64, t[k * 6 + 2] as f64);
        p(0).dot(p(1).cross(p(2))) / 6.0
    }).sum()
}
fn aire(v: &[f32]) -> f64 {
    v.chunks_exact(18).map(|t| {
        let p = |k: usize| glam::DVec3::new(t[k * 6] as f64, t[k * 6 + 1] as f64, t[k * 6 + 2] as f64);
        (p(1) - p(0)).cross(p(2) - p(0)).length() * 0.5
    }).sum()
}
fn boite(v: &[f32]) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for c in v.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
    (lo, hi)
}
/// Normales stockées cohérentes avec l'ordre des sommets (produit vectoriel), en proportion.
fn normales_coherentes(v: &[f32]) -> f64 {
    let mut ok = 0usize; let mut n = 0usize;
    for t in v.chunks_exact(18) {
        let p = |k: usize| Vec3::new(t[k * 6], t[k * 6 + 1], t[k * 6 + 2]);
        let g = (p(1) - p(0)).cross(p(2) - p(0));
        if g.length() < 1e-9 { continue; }
        n += 1;
        if g.normalize().dot(Vec3::new(t[3], t[4], t[5])) > 0.5 { ok += 1; }
    }
    ok as f64 / n.max(1) as f64
}
/// Triangles sous forme canonique (positions seules, rotation du triangle qui commence au plus
/// petit sommet, liste triée) : pour comparer deux maillages au bit près, ordre ignoré.
fn canon(v: &[f32]) -> Vec<[u32; 9]> {
    let mut out: Vec<[u32; 9]> = v.chunks_exact(18).map(|t| {
        let p: [[u32; 3]; 3] = std::array::from_fn(|k| std::array::from_fn(|d| t[k * 6 + d].to_bits()));
        let m = (0..3).min_by_key(|&k| p[k]).unwrap();
        let q = [p[m], p[(m + 1) % 3], p[(m + 2) % 3]];
        [q[0][0], q[0][1], q[0][2], q[1][0], q[1][1], q[1][2], q[2][0], q[2][1], q[2][2]]
    }).collect();
    out.sort_unstable();
    out
}
fn ecart_rel(a: f64, b: f64) -> f64 { (a - b).abs() / b.abs().max(1e-12) }

/// Objet de la scène sans contexte GL (identifiants factices : jamais dessiné ici).
fn obj(verts: Vec<f32>) -> Obj {
    let id = NonZeroU32::new(1).unwrap();
    let mut o = Obj { vao: glow::NativeVertexArray(id), vbo: glow::NativeBuffer(id), cbo: glow::NativeBuffer(id),
                      cols: Vec::new(), verts, tris: Vec::new(), nverts: 0, min: Vec3::ZERO, max: Vec3::ZERO };
    o.refresh();
    o
}
fn prim(p: occt::Prim) -> Vec<f32> {
    let mut m = occt::primitive(p, occt::DEFLEXION).expect("primitive OCCT");
    clean_prim(&mut m);
    m.verts
}
/// Boîte décalée à des coordonnées « non rondes » (les erreurs d'arrondi s'y voient).
fn boite_test() -> Vec<f32> {
    let mut o = obj(prim(occt::Prim::Box { dx: 40.0, dy: 30.0, dz: 20.0 }));
    o.translate(Vec3::new(0.123456, -7.654321, 3.3333333));
    o.verts
}
fn dossier() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("3dpatch-tests-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ---------- primitives OCCT ----------

#[test]
fn primitives_exactes() {
    use std::f64::consts::PI;
    // (primitive, volume exact, boîte exacte, tolérance relative sur le volume)
    let cas: Vec<(occt::Prim, f64, Vec3, f64)> = vec![
        (occt::Prim::Box { dx: 40.0, dy: 30.0, dz: 20.0 }, 24000.0, Vec3::new(40.0, 30.0, 20.0), 1e-6),
        (occt::Prim::Sphere { r: 10.0 }, 4.0 / 3.0 * PI * 1000.0, Vec3::splat(20.0), 0.005),
        (occt::Prim::Cylinder { r: 10.0, h: 25.0 }, PI * 100.0 * 25.0, Vec3::new(20.0, 20.0, 25.0), 0.005),
        (occt::Prim::Cone { r1: 10.0, r2: 4.0, h: 15.0 }, PI * 15.0 / 3.0 * (100.0 + 40.0 + 16.0), Vec3::new(20.0, 20.0, 15.0), 0.005),
        (occt::Prim::Torus { big: 20.0, small: 5.0 }, 2.0 * PI * PI * 20.0 * 25.0, Vec3::new(50.0, 50.0, 10.0), 0.005),
    ];
    for (p, vol, taille, tol) in cas {
        let v = prim(p);
        let d = defects::analyze(&v);
        let (lo, hi) = boite(&v);
        let vv = volume(&v);
        assert_eq!((d.holes, d.face_errors), (0, 0), "défauts sur une primitive");
        assert!(vv > 0.0, "faces retournées (volume {vv})");
        assert!(ecart_rel(vv, vol) < tol, "volume {vv} au lieu de {vol}");
        // les sommets ne tombent pas forcément sur les extrêmes d'une surface courbe : écart ≤ 2 × flèche
        assert!(((hi - lo) - taille).abs().max_element() <= 2.0 * occt::DEFLEXION as f32 + 1e-4, "boîte {:?} au lieu de {taille:?}", hi - lo);
        assert!(normales_coherentes(&v) > 0.999, "normales incohérentes");
    }
}

#[test]
fn import_step_boite_congee() {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../poc-occt/sortie/boite_congee.step");
    if !std::path::Path::new(p).exists() { return; }
    let (m, _) = load_model(&p).expect("STEP");
    let (lo, hi) = boite(&m.verts);
    assert!(((hi - lo) - Vec3::new(40.0, 30.0, 20.0)).abs().max_element() < 1e-3, "boîte STEP {:?}", hi - lo);
    let d = defects::analyze(&m.verts);
    assert_eq!((d.holes, d.face_errors), (0, 0));
    // boîte 40×30×20 congée r = 5 : volume exact = boîte − matière enlevée par les congés
    // (arêtes : (4−π)r² par unité de longueur sur les arêtes droites ; coins : (8 − 4π/3)r³ …
    //  formule fermée : V = abc − (4−π)r²(a+b+c−6r)·… ) → on vérifie seulement l'encadrement
    let vv = volume(&m.verts);
    assert!(vv < 24000.0 && vv > 24000.0 * 0.9, "volume STEP {vv}");
}

// ---------- allers-retours d'export / import ----------

#[test]
fn export_import_sans_perte() {
    let v = boite_test();
    let m = stl::Mesh { min: boite(&v).0, max: boite(&v).1, verts: v.clone() };
    let d = dossier();
    let attendu = canon(&v);
    for ext in ["stl", "obj", "ply", "3mf"] {
        let p = d.join(format!("essai.{ext}"));
        match ext {
            "obj" => stl::write_obj(&p, &m).unwrap(),
            "ply" => stl::write_ply(&p, &m).unwrap(),
            "3mf" => threemf::write(&p, &m.verts).unwrap(),
            _ => write_stl(&p, &m).unwrap(),
        }
        let (r, _) = load_model(p.to_str().unwrap()).expect(ext);
        assert_eq!(canon(&r.verts), attendu, "{ext} : positions modifiées par l'aller-retour");
        assert!(normales_coherentes(&r.verts) > 0.999, "{ext} : normales");
    }
}

#[test]
fn export_import_formats_tiers() {
    if assimp::export_id("glb").is_none() { return; }
    let v = boite_test();
    let d = dossier();
    let (vol, ar, (lo, hi)) = (volume(&v), aire(&v), boite(&v));
    let mut essais = vec![];
    for ext in ["gltf", "glb", "dae", "x3d", "3ds", "fbx"] { essais.push(ext); }
    for ext in essais {
        let p = d.join(format!("essai.{ext}"));
        if assimp::export(&p, ext, &v, None).is_err() { continue; } // version sans Assimp
        let (r, _) = load_model(p.to_str().unwrap()).unwrap_or_else(|e| panic!("{ext} : {e}"));
        let (l2, h2) = boite(&r.verts);
        // f32 dans ces formats : écart ≤ 1e-4 mm ; l'exportateur X3D d'Assimp écrit 3 décimales (≤ 5e-4 mm)
        let tol = if ext == "x3d" { 6e-4 } else { 1e-4 };
        assert!((l2 - lo).abs().max_element() < tol && (h2 - hi).abs().max_element() < tol, "{ext} : boîte {l2:?} {h2:?}");
        assert!(ecart_rel(volume(&r.verts), vol) < tol as f64, "{ext} : volume {} / {vol}", volume(&r.verts));
        assert!(ecart_rel(aire(&r.verts), ar) < tol as f64, "{ext} : aire");
        assert_eq!(r.verts.len(), v.len(), "{ext} : nombre de triangles");
    }
}

// ---------- transformations ----------

#[test]
fn deplacement_exact() {
    let v = boite_test();
    let mut o = obj(v.clone());
    let d = Vec3::new(12.5, -3.25, 0.75);
    o.translate(d);
    o.translate(-d);
    assert!(o.verts.iter().zip(&v).all(|(a, b)| (a - b).abs() < 1e-5), "aller-retour de déplacement");
    assert_eq!(o.tris.len(), v.len() / 18);
    let (lo, hi) = boite(&o.verts);
    assert!((lo - o.min).abs().max_element() < 1e-5 && (hi - o.max).abs().max_element() < 1e-5, "boîte mémorisée fausse");
}

#[test]
fn redimensionnement_exact() {
    let mut o = obj(boite_test());
    let vol0 = volume(&o.verts);
    let c = (o.min + o.max) * 0.5;
    let f = Vec3::new(2.0, 0.5, 1.25);
    o.scale_about(c, f);
    let (lo, hi) = boite(&o.verts);
    assert!(((hi - lo) - Vec3::new(80.0, 15.0, 25.0)).abs().max_element() < 1e-3, "taille {:?}", hi - lo);
    assert!(((lo + hi) * 0.5 - c).abs().max_element() < 1e-3, "centre déplacé");
    assert!(ecart_rel(volume(&o.verts), vol0 * 1.25) < 1e-5, "volume après mise à l'échelle");
    assert!(normales_coherentes(&o.verts) > 0.999);
}

#[test]
fn rotation_exacte() {
    let mut o = obj(boite_test());
    let vol0 = volume(&o.verts);
    let z0 = o.min.z;
    let c = (o.min + o.max) * 0.5;
    rotate_obj(&mut o, c, glam::Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let (lo, hi) = boite(&o.verts);
    assert!(((hi - lo) - Vec3::new(30.0, 40.0, 20.0)).abs().max_element() < 1e-3, "90° autour de Z : {:?}", hi - lo);
    assert!((o.min.z - z0).abs() < 1e-4, "l'objet ne reste pas posé");
    assert!(ecart_rel(volume(&o.verts), vol0) < 1e-5);
    // quatre quarts de tour = retour au départ
    let mut o2 = obj(boite_test());
    let c2 = (o2.min + o2.max) * 0.5;
    for _ in 0..4 { rotate_obj(&mut o2, c2, glam::Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2)); }
    let ref_ = boite_test();
    let err = o2.verts.chunks_exact(6).zip(ref_.chunks_exact(6)).map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max)).fold(0.0, f32::max);
    assert!(err < 1e-3, "4 × 90° : écart {err} mm");
    assert!(normales_coherentes(&o2.verts) > 0.999);
}

#[test]
fn gravite_pose_a_plat() {
    // boîte penchée de 30° autour de X puis lâchée : elle doit finir posée sur une grande face
    let mut o = obj(boite_test());
    let c = (o.min + o.max) * 0.5;
    rotate_obj(&mut o, c, glam::Mat3::from_rotation_x(0.52) * glam::Mat3::from_rotation_y(0.2));
    o.translate(Vec3::new(0.0, 0.0, 15.0));
    let vol0 = volume(&o.verts);
    let mut a = GravAnim::new(0, &o).expect("gravité");
    a.t0 = std::time::Instant::now() - std::time::Duration::from_secs_f32(GravAnim::duree() + 30.0); // chute simulée : 12 s au plus
    assert!(a.appliquer(&mut o), "animation non terminée");
    assert!(o.min.z.abs() < 1e-4, "z min = {}", o.min.z);
    assert!(ecart_rel(volume(&o.verts), vol0) < 1e-5, "volume modifié");
    // face d'appui : au moins 4 sommets distincts à z = 0 (une face entière posée)
    let mut bas: Vec<[i64; 2]> = o.verts.chunks_exact(6).filter(|p| p[2].abs() < 1e-3)
        .map(|p| [(p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64]).collect();
    bas.sort(); bas.dedup();
    assert!(bas.len() >= 4, "objet posé sur {} point(s) seulement", bas.len());
    // l'état d'animation part du sol et y revient, sans jamais passer dessous
    for i in 0..=200 {
        let t = GravAnim::duree() * i as f32 / 200.0;
        let (_, z) = a.etat(t);
        assert!(z >= -1e-5, "passe sous le plan à t = {t}");
    }
}

#[test]
fn chute_physique_naturelle() {
    let tourne = |v: &[f32], r: glam::Mat3| { let mut v = v.to_vec(); for c in v.chunks_exact_mut(6) { let q = r * Vec3::new(c[0], c[1], c[2]); c[0] = q.x; c[1] = q.y; c[2] = q.z; } v };
    let haut_final = |v: &[f32]| { let (lo, hi) = boite(v); let c = gravite::simuler(v, ((hi - lo).length() * 0.4).max(lo.z)).unwrap(); (c.images.last().unwrap().0, c.images.len()) };
    // tige 6×6×100 debout penchée de 10° : le centre de gravité sort de sa base, elle tombe et se couche
    let r = glam::Mat3::from_rotation_x(10f32.to_radians());
    let (q, n) = haut_final(&tourne(&prim(occt::Prim::Box { dx: 6.0, dy: 6.0, dz: 100.0 }), r));
    assert!((q * (r * Vec3::Z)).z.abs() < 0.02, "tige non couchée");
    assert!(n < 60 * 12, "la chute ne s'arrête pas");
    // pavé trapu 30×30×20 à plat, penché de 10° : il retombe sur sa base
    let r = glam::Mat3::from_rotation_x(10f32.to_radians());
    let (q, _) = haut_final(&tourne(&prim(occt::Prim::Box { dx: 30.0, dy: 30.0, dz: 20.0 }), r));
    assert!((q * (r * Vec3::Z)).z > 0.999, "pavé pas revenu sur sa base");
}

// ---------- réparation, simplification, enveloppe, relief ----------

#[test]
fn defauts_detectes_et_repares() {
    let v = boite_test();
    // un trou : un triangle retiré
    let troue: Vec<f32> = v[18..].to_vec();
    let d = defects::analyze(&troue);
    assert_eq!(d.holes, 1, "trou non détecté");
    let r = defects::repair(&troue);
    let d2 = defects::analyze(&r);
    assert_eq!((d2.holes, d2.face_errors), (0, 0), "trou non rebouché");
    assert!(ecart_rel(volume(&r), volume(&v)) < 1e-4, "volume après rebouchage {} / {}", volume(&r), volume(&v));
    // une face retournée
    let mut inv = v.clone();
    let (a, b) = (inv[0..6].to_vec(), inv[6..12].to_vec());
    inv[0..6].copy_from_slice(&b); inv[6..12].copy_from_slice(&a);
    assert!(defects::analyze(&inv).face_errors > 0, "face retournée non détectée");
    let r = defects::repair(&inv);
    let d3 = defects::analyze(&r);
    assert_eq!((d3.holes, d3.face_errors), (0, 0), "face retournée non corrigée");
    assert!(ecart_rel(volume(&r), volume(&v)) < 1e-4, "volume après correction");
    assert!(normales_coherentes(&r) > 0.999);
}

#[test]
fn simplification_fidele() {
    let v = prim(occt::Prim::Sphere { r: 10.0 });
    let n0 = v.len() / 18;
    let cible = n0 / 4;
    let s = simplify::resample(&v, cible);
    let n = s.len() / 18;
    let d = defects::analyze(&s);
    assert_eq!((d.holes, d.face_errors), (0, 0), "simplification : objet ouvert");
    assert!(n <= cible + cible / 10 && n >= cible / 2, "{n} triangles pour une cible de {cible}");
    assert!(ecart_rel(volume(&s), volume(&v)) < 0.02, "volume {} / {}", volume(&s), volume(&v));
    let ((l0, h0), (l1, h1)) = (boite(&v), boite(&s));
    assert!((l0 - l1).abs().max_element() < 0.3 && (h0 - h1).abs().max_element() < 0.3, "forme déformée");
}

#[test]
fn enveloppe_fermee_et_fidele() {
    let v = boite_test();
    let e = remesh::envelope(&v, 160);
    let d = defects::analyze(&e);
    assert_eq!((d.holes, d.face_errors), (0, 0), "enveloppe ouverte");
    assert!(volume(&e) > 0.0);
    assert!(ecart_rel(volume(&e), volume(&v)) < 0.05, "volume enveloppe {} / {}", volume(&e), volume(&v));
    let ((l0, h0), (l1, h1)) = (boite(&v), boite(&e));
    assert!((l0 - l1).abs().max_element() < 1.0 && (h0 - h1).abs().max_element() < 1.0, "enveloppe : boîte {:?} {:?}", l1, h1);
}

#[test]
fn relief_reste_ferme() {
    let v = prim(occt::Prim::Cylinder { r: 15.0, h: 20.0 });
    let (l0, h0) = boite(&v);
    for motif in relief::PATTERNS {
        for mapping in [relief::Mapping::Cylindrical, relief::Mapping::Triplanar] {
            let pr = relief::Params { pattern: motif, mapping, height_mm: 0.6, scale_mm: 4.0, invert: false, mask_normal: None };
            let (r, _, _) = relief::apply(&v, &pr);
            let d = defects::analyze(&r);
            assert_eq!((d.holes, d.face_errors), (0, 0), "relief {} : objet ouvert", motif.key());
            assert!(volume(&r) > 0.0, "relief {} : faces retournées", motif.key());
            let (lo, hi) = boite(&r);
            assert!((lo - l0).abs().max_element() < 0.6 + 1e-3 && (hi - h0).abs().max_element() < 0.6 + 1e-3,
                    "relief {} plus haut que demandé : {lo:?} {hi:?}", motif.key());
        }
    }
}

// ---------- mesures affichées, unités, pick ----------

#[test]
fn mesures_affichees() {
    assert_eq!(fmt_measure(40.0, Unit::Mm), "40");
    assert_eq!(fmt_measure(40.0, Unit::Cm), "4");
    assert_eq!(fmt_measure(40.0, Unit::M), "0.04");
    assert_eq!(fmt_measure(25.4, Unit::In), "1");
    assert_eq!(fmt_measure(304.8, Unit::Ft), "1");
    assert_eq!(fmt_measure(914.4, Unit::Yd), "1");
    assert_eq!(fmt_measure(12.345, Unit::Mm), "12.35");
    assert_eq!(fmt_measure(0.01, Unit::M), "0.00001");
    // résolution ≈ 0,01 mm dans toutes les unités : deux longueurs distantes de 0,02 mm se distinguent
    for u in [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft, Unit::Yd] {
        assert_ne!(fmt_measure(100.0, u), fmt_measure(100.02, u), "{} : 0,02 mm invisible", u.code());
    }
    for (t, s) in [(0.7, 1.0), (1.0, 1.0), (1.2, 2.0), (3.0, 5.0), (7.0, 10.0), (45.0, 50.0), (0.013, 0.02)] {
        assert!((nice_step(t) - s).abs() < 1e-6 * s, "nice_step({t}) = {}", nice_step(t));
    }
    // pas de grille rond dans l'unité affichée
    for u in [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft, Unit::Yd] {
        let p = u.pas_grille(37.0) / u.factor();
        let m = p / 10f32.powf(p.log10().floor());
        assert!([1.0, 2.0, 5.0, 10.0].iter().any(|x| (m - x).abs() < 1e-3), "{} : pas {p}", u.code());
    }
}

#[test]
fn rayon_triangle_exact() {
    let t = [Vec3::new(0.0, 0.0, 5.0), Vec3::new(10.0, 0.0, 5.0), Vec3::new(0.0, 10.0, 5.0)];
    let d = ray_tri(Vec3::new(1.0, 1.0, 20.0), Vec3::new(0.0, 0.0, -1.0), &t).expect("impact");
    assert!((d - 15.0).abs() < 1e-5, "distance {d}");
    assert!(ray_tri(Vec3::new(9.0, 9.0, 20.0), Vec3::new(0.0, 0.0, -1.0), &t).is_none(), "impact hors triangle");
}


// ---------- booléens ----------

fn decale(v: &[f32], d: Vec3) -> Vec<f32> { let mut o = obj(v.to_vec()); o.translate(d); o.verts }

#[test]
fn booleens_exacts() {
    // deux cubes de 20 mm décalés de 10 mm en X (et 5 en Y) : volumes analytiques exacts
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 20.0, dz: 20.0 });
    let b = decale(&a, Vec3::new(10.0, 5.0, 0.0));
    let commun = 10.0 * 15.0 * 20.0;
    for (op, attendu) in [(booleen::Op::Union, 2.0 * 8000.0 - commun), (booleen::Op::Soustraction, 8000.0 - commun), (booleen::Op::Intersection, commun)] {
        let r = booleen::calculer(&a, &b, op).unwrap_or_else(|e| panic!("{op:?} : {e}"));
        let d = defects::analyze(&r);
        assert_eq!((d.holes, d.face_errors), (0, 0), "{op:?} : résultat ouvert");
        assert!(ecart_rel(volume(&r), attendu) < 1e-9, "{op:?} : volume {} au lieu de {attendu}", volume(&r));
        assert!(normales_coherentes(&r) > 0.999, "{op:?} : normales");
    }
    // perçage : cube − cylindre traversant (r = 5, h = 40), volume exact au maillage près
    let cyl = decale(&prim(occt::Prim::Cylinder { r: 5.0, h: 40.0 }), Vec3::new(10.0, 10.0, -10.0));
    let r = booleen::calculer(&a, &cyl, booleen::Op::Soustraction).unwrap();
    let d = defects::analyze(&r);
    assert_eq!((d.holes, d.face_errors), (0, 0), "perçage ouvert");
    let attendu = 8000.0 - std::f64::consts::PI * 25.0 * 20.0;
    assert!(ecart_rel(volume(&r), attendu) < 0.005, "perçage : volume {} au lieu de {attendu}", volume(&r));
    let (lo, hi) = boite(&r);
    assert!(((hi - lo) - Vec3::splat(20.0)).abs().max_element() < 1e-4, "perçage : boîte {:?}", hi - lo);
    // objets disjoints : l'intersection est vide (message), l'union garde les deux volumes
    let loin = decale(&a, Vec3::new(100.0, 0.0, 0.0));
    assert!(booleen::calculer(&a, &loin, booleen::Op::Intersection).is_err());
    let u = booleen::calculer(&a, &loin, booleen::Op::Union).unwrap();
    assert!(ecart_rel(volume(&u), 16000.0) < 1e-9);
    // objet ouvert (un triangle en moins) : réparé automatiquement avant l'opération, résultat exact
    let u = booleen::calculer(&a[18..], &b, booleen::Op::Union).expect("objet troué non réparé");
    assert!(ecart_rel(volume(&u), 2.0 * 8000.0 - commun) < 1e-6, "union après réparation : {}", volume(&u));
    let (p, q) = booleen::couper(&a[18..], &booleen::Cadre::new(Vec3::new(5.0, -10.0, 0.0), Vec3::new(5.0, 30.0, 0.0), 2)).expect("découpe d'un objet troué");
    assert!(ecart_rel(volume(&p) + volume(&q), 8000.0) < 1e-6);
}

#[test]
fn booleens_plusieurs_objets() {
    // comme 3D Builder : 3 cubes de 20 mm alignés en X, chevauchement de 10 mm entre voisins
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 20.0, dz: 20.0 });
    let b = decale(&a, Vec3::new(10.0, 0.0, 0.0));
    let c = decale(&a, Vec3::new(20.0, 0.0, 0.0));
    let objs: [&[f32]; 3] = [&a, &b, &c];
    // fusion : 40 × 20 × 20
    let u = booleen::calculer_n(&objs, booleen::Op::Union).unwrap();
    assert!(ecart_rel(volume(&u), 16000.0) < 1e-9, "fusion : {}", volume(&u));
    // intersection : A ∩ B ∩ (A décalé de 5) -> x ∈ [10, 20] : 4000
    let c5 = decale(&a, Vec3::new(5.0, 0.0, 0.0));
    let i = booleen::calculer_n(&[&a, &b, &c5], booleen::Op::Intersection).unwrap();
    assert!(ecart_rel(volume(&i), 10.0 * 400.0) < 1e-9, "intersection : {}", volume(&i));
    // soustraction : le dernier (C) est retiré de A et B réunis -> x ∈ [0, 20] : 8000
    let s = booleen::calculer_n(&objs, booleen::Op::Soustraction).unwrap();
    assert!(ecart_rel(volume(&s), 8000.0) < 1e-9, "soustraction : {}", volume(&s));
    let d = defects::analyze(&s);
    assert_eq!((d.holes, d.face_errors), (0, 0));
    // un seul objet : refus
    assert!(booleen::calculer_n(&[&a], booleen::Op::Union).is_err());
}

#[test]
fn lignes_d_intersection() {
    // A = [0,20]³, B = A décalé de (10,5,5) : la courbe où les surfaces se croisent mesure 80 mm
    let tris = |v: &[f32]| v.chunks_exact(18).map(|t| [Vec3::new(t[0], t[1], t[2]), Vec3::new(t[6], t[7], t[8]), Vec3::new(t[12], t[13], t[14])]).collect::<Vec<_>>();
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 20.0, dz: 20.0 });
    let b = decale(&a, Vec3::new(10.0, 5.0, 5.0));
    let sg = booleen::intersections(&[tris(&a), tris(&b)]);
    let l: f32 = sg.iter().map(|s| (s[1] - s[0]).length()).sum();
    assert!((l - 80.0).abs() < 0.5, "longueur {l} ({} segments)", sg.len());
    // objets éloignés : aucune ligne
    let c = decale(&a, Vec3::new(100.0, 0.0, 0.0));
    assert!(booleen::intersections(&[tris(&a), tris(&c)]).is_empty());
}

#[test]
fn creuser_coque_reguliere() {
    // cube de 40 mm, paroi de 2 mm : il reste 40³ − 36³ de matière (coins de la cavité à peine arrondis)
    let a = prim(occt::Prim::Box { dx: 40.0, dy: 40.0, dz: 40.0 });
    let t = std::time::Instant::now();
    let (r, eco) = booleen::creuser(&a, 2.0, None, 200).unwrap();
    let d = defects::analyze(&r);
    assert_eq!((d.holes, d.face_errors), (0, 0), "coque ouverte");
    let attendu = 64000.0 - 36.0f64.powi(3);
    eprintln!("CREUX cube : volume {:.0} (attendu {attendu:.0}), économie {:.0} %, {:.2} s", volume(&r), eco * 100.0, t.elapsed().as_secs_f32());
    assert!(ecart_rel(volume(&r), attendu) < 0.04, "volume {} au lieu de {attendu}", volume(&r));
    // l'extérieur est intact (même boîte englobante)
    let (lo, hi) = boite(&r);
    assert!(((hi - lo) - Vec3::splat(40.0)).abs().max_element() < 1e-3);
    // sphère r = 20, paroi 3 mm + trou d'évacuation Ø 4 : coque fermée, ouverte par le seul trou
    let s = prim(occt::Prim::Sphere { r: 20.0 });
    let (r, _) = booleen::creuser(&s, 3.0, Some(4.0), 200).unwrap();
    let d = defects::analyze(&r);
    assert_eq!((d.holes, d.face_errors), (0, 0), "sphère creuse percée : défauts");
    let attendu = 4.0 / 3.0 * std::f64::consts::PI * (20.0f64.powi(3) - 17.0f64.powi(3));
    eprintln!("CREUX sphère : volume {:.0} (attendu ≈ {attendu:.0} moins le trou)", volume(&r));
    assert!(ecart_rel(volume(&r), attendu) < 0.06);
    // paroi plus épaisse que l'objet : refus clair
    assert!(booleen::creuser(&a, 25.0, None, 200).is_err());
}

#[test]
fn trois_mf_multi_objets() {
    // 3 objets séparés (dont un coloré) -> 3MF -> relus : 3 objets, positions au bit près, couleurs à 1/255
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 20.0, dz: 20.0 });
    let b = decale(&prim(occt::Prim::Sphere { r: 8.0 }), Vec3::new(40.0, 5.0, 0.0));
    let c = decale(&prim(occt::Prim::Cylinder { r: 4.0, h: 30.0 }), Vec3::new(-30.0, 10.0, 2.5));
    let cols_b: Vec<f32> = (0..b.len() / 6).flat_map(|k| if k % 2 == 0 { [1.0, 0.0, 0.0] } else { [0.2, 0.4, 1.0] }).collect();
    let p = std::env::temp_dir().join("patch3d_multi.3mf");
    threemf::write_objets(&p, &[(&a, None), (&b, Some(&cols_b)), (&c, None)]).unwrap();
    let objs = threemf::read_objets(p.to_str().unwrap()).unwrap();
    assert_eq!(objs.len(), 3, "objets séparés perdus");
    for (orig, (lu, _)) in [&a, &b, &c].iter().zip(&objs) {
        assert_eq!(canon(orig), canon(lu), "géométrie ou position modifiée");
    }
    assert!(objs[0].1.is_none() && objs[2].1.is_none());
    let lu = objs[1].1.as_ref().expect("couleurs perdues");
    assert_eq!(lu.len(), cols_b.len());
    assert!(lu.iter().zip(&cols_b).all(|(x, y)| (x - y).abs() <= 0.5 / 255.0 + 1e-6), "couleurs modifiées");
    // lecture « tout réuni » : même total de triangles
    let (tout, _) = threemf::read(p.to_str().unwrap()).unwrap();
    assert_eq!(tout.len(), a.len() + b.len() + c.len());
    std::fs::remove_file(&p).ok();
}

#[test]
fn mise_en_forme_exacte() {
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 10.0, dz: 5.0 });
    let vol = volume(&a);
    // miroir X : volume conservé (positif = faces vers l'extérieur), boîte reflétée, 0 défaut
    let mut m = a.clone(); let mut cols = Vec::new();
    forme::miroir(&mut m, &mut cols, 0, Vec3::ZERO);
    assert!(ecart_rel(volume(&m), vol) < 1e-9, "miroir retourné : {}", volume(&m));
    let (lo, hi) = forme::bornes(&m);
    assert!((lo - Vec3::new(-20.0, 0.0, 0.0)).abs().max_element() < 1e-5 && (hi - Vec3::new(0.0, 10.0, 5.0)).abs().max_element() < 1e-5);
    let d = defects::analyze(&m); assert_eq!((d.holes, d.face_errors), (0, 0));
    assert!(normales_coherentes(&m) > 0.999);
    // rotation 90° autour de Z : dimensions échangées, volume conservé
    let mut r = a.clone();
    forme::tourner(&mut r, Vec3::ZERO, glam::Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let (lo, hi) = forme::bornes(&r);
    assert!(((hi - lo) - Vec3::new(10.0, 20.0, 5.0)).abs().max_element() < 1e-4);
    assert!(ecart_rel(volume(&r), vol) < 1e-6);
    // échelle 200 % / 50 % / 100 % : volume × 1
    let mut e = a.clone();
    forme::echelle(&mut e, Vec3::ZERO, Vec3::new(2.0, 0.5, 1.0));
    assert!(ecart_rel(volume(&e), vol) < 1e-6);
    let (lo, hi) = forme::bornes(&e);
    assert!(((hi - lo) - Vec3::new(40.0, 5.0, 5.0)).abs().max_element() < 1e-4);
    // répartition : 3 boîtes de 10 entre 0 et 100 -> début en 0, 45, 90
    let x = forme::repartir(&[(0.0, 10.0), (70.0, 10.0), (90.0, 10.0)]);
    assert!((x[1] - 45.0).abs() < 1e-5 && x[0] == 0.0 && x[2] == 90.0, "{x:?}");
}

#[test]
fn texte_relief_et_gravure() {
    let chemin = std::path::PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
    if !chemin.exists() { return; }
    let pol = texte::Police { nom: "DejaVu Sans".into(), chemin, index: 0 };
    let (polys, w, h) = texte::contours(&pol, "AB", 10.0, 1, 0.0).unwrap();
    assert!(w > 10.0 && w < 20.0 && h > 9.0 && h < 14.0, "bloc {w} × {h}");
    // aire des lettres (aperçu triangulé) : trous de « A » et « B » respectés
    let aire: f64 = texte::triangles(&polys).iter().map(|t| (((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0])) * 0.5).abs() as f64).sum();
    assert!(aire > 20.0 && aire < 80.0, "aire {aire}");
    let boite = decale(&prim(occt::Prim::Box { dx: 60.0, dy: 30.0, dz: 10.0 }), Vec3::new(-30.0, -15.0, 0.0));
    let v0 = volume(&boite);
    let base = |relief: bool, suivre: bool| booleen::ParamsTexte { polys: polys.clone(), u: Vec3::X, v: Vec3::Y, n: Vec3::Z,
        centre: Vec3::ZERO, angle: 0.3, prof: 2.0, relief, suivre };
    // relief à plat : + aire × 2 mm
    let r = booleen::texte(&boite, &base(true, false)).unwrap();
    let d = defects::analyze(&r); assert_eq!((d.holes, d.face_errors), (0, 0));
    eprintln!("TEXTE relief : +{:.1} mm³ (attendu {:.1})", volume(&r) - v0, aire * 2.0);
    assert!(ecart_rel(volume(&r) - v0, aire * 2.0) < 0.02);
    assert!((forme::bornes(&r).1.z - 12.0).abs() < 1e-3, "hauteur du relief");
    // gravure : − aire × 2 mm
    let g = booleen::texte(&boite, &base(false, false)).unwrap();
    let d = defects::analyze(&g); assert_eq!((d.holes, d.face_errors), (0, 0));
    assert!(ecart_rel(v0 - volume(&g), aire * 2.0) < 0.02);
    // suivre la forme sur une sphère : sans défaut, le texte dépasse de ~2 mm au sommet
    let sph = prim(occt::Prim::Sphere { r: 25.0 });
    let (lo, hi) = boite_de(&sph);
    let c = (lo + hi) * 0.5;
    let mut ps = base(true, true); ps.centre = c; ps.angle = 0.0;
    let s = booleen::texte(&sph, &ps).unwrap();
    let d = defects::analyze(&s); assert_eq!((d.holes, d.face_errors), (0, 0), "sphère + texte : défauts");
    let top = forme::bornes(&s).1.z;
    eprintln!("TEXTE sur sphère : sommet {top:.2} (sphère {:.2})", hi.z);
    assert!(top > hi.z + 1.5 && top < hi.z + 2.2);
    // cadre à côté de l'objet : refus clair
    let mut loin = base(true, false); loin.centre = Vec3::new(500.0, 0.0, 0.0);
    assert!(booleen::texte(&boite, &loin).is_err());
}
fn boite_de(v: &[f32]) -> (Vec3, Vec3) { forme::bornes(v) }

#[test]
fn image_vectorisee_relief_et_gravure() {
    use crate::vectorise;
    // anneau noir (R 80, r 40 px) + carré rouge sur fond blanc, avec un peu de transparence
    let (w, h) = (300u32, 200u32);
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 - 100.0, y as f32 - 100.0);
        let d = (dx * dx + dy * dy).sqrt();
        if d < 80.0 && d >= 40.0 { image::Rgba([10, 10, 10, 255]) }
        else if (210..270).contains(&x) && (70..130).contains(&y) { image::Rgba([200, 30, 30, 255]) }
        else { image::Rgba([0, 0, 0, 0]) } // transparent = fond blanc
    });
    let f = std::env::temp_dir().join("patch3d_test_vecto.png");
    img.save(&f).unwrap();
    let mut im = vectorise::Image::charger(&f).unwrap();
    // seuil automatique : anneau (extérieur + trou) et carré rouge (plus sombre que le blanc)
    let mut prm = vectorise::Params { tolerance: 0.8, ..Default::default() };
    im.maj(&mut prm);
    let aire_px: f64 = im.polys.iter().map(|c| vectorise::aire(c)).sum();
    let attendu = std::f64::consts::PI * (80.0f64.powi(2) - 40.0f64.powi(2)) + 60.0 * 60.0;
    eprintln!("IMAGE : {} contours, aire {aire_px:.0} px² (attendu {attendu:.0})", im.polys.len());
    assert_eq!(im.polys.len(), 3);
    assert!(ecart_rel(aire_px, attendu) < 0.02);
    // par couleurs : seule la couleur la plus sombre (noir) en plein → l'anneau seul
    let mut pc = vectorise::Params { mode: vectorise::Mode::Couleurs, n_coul: 3, ..prm.clone() };
    im.maj(&mut pc);
    pc.pleines = (0..im.palette().len()).map(|i| i == 0).collect();
    im.maj(&mut pc);
    assert_eq!(im.polys.len(), 2, "couleurs : anneau seul");
    // inversé : le fond devient plein (cadre de l'image percé de l'anneau et du carré)
    let mut pi = vectorise::Params { inverser: true, ..prm.clone() };
    im.maj(&mut pi);
    assert!(im.polys.iter().any(|c| vectorise::aire(c) < 0.0));
    // une autre image chargée ne reprend jamais le numéro de vectorisation de la précédente
    let mut autre = vectorise::Image::charger(&f).unwrap();
    autre.maj(&mut vectorise::Params::default());
    assert!(autre.generation > im.generation, "numéro de vectorisation réutilisé");
    // incrustation dans une plaque, comme un texte : 50 mm de large, relief et gravure de 2 mm
    im.maj(&mut prm);
    let (polys, lx, ly) = im.contours_mm(50.0);
    assert!((lx - 50.0).abs() < 1e-4 && ly > 25.0 && ly < 35.0, "cadre {lx} × {ly}");
    let k = 50.0 / 250.0; // largeur du dessin : 20..270 px
    let aire_mm = attendu * k * k;
    let boite = decale(&prim(occt::Prim::Box { dx: 70.0, dy: 50.0, dz: 10.0 }), Vec3::new(-35.0, -25.0, 0.0));
    let v0 = volume(&boite);
    let p = |relief: bool| booleen::ParamsTexte { polys: polys.clone(), u: Vec3::X, v: Vec3::Y, n: Vec3::Z, centre: Vec3::ZERO, angle: 0.0, prof: 2.0, relief, suivre: false };
    let r = booleen::texte(&boite, &p(true)).unwrap();
    let d = defects::analyze(&r); assert_eq!((d.holes, d.face_errors), (0, 0), "image en relief : défauts");
    eprintln!("IMAGE relief : +{:.1} mm³ (attendu {:.1})", volume(&r) - v0, aire_mm * 2.0);
    assert!(ecart_rel(volume(&r) - v0, aire_mm * 2.0) < 0.03);
    let g = booleen::texte(&boite, &p(false)).unwrap();
    let d = defects::analyze(&g); assert_eq!((d.holes, d.face_errors), (0, 0), "image gravée : défauts");
    assert!(ecart_rel(v0 - volume(&g), aire_mm * 2.0) < 0.03);
}

#[test]
fn verifications_impression() {
    let tris = |v: &[f32]| v.chunks_exact(18).map(|t| [Vec3::new(t[0], t[1], t[2]), Vec3::new(t[6], t[7], t[8]), Vec3::new(t[12], t[13], t[14])]).collect::<Vec<_>>();
    // table : plateau 40×40 épais de 4 sur un pied 10×10 haut de 20 -> dessous du plateau en surplomb
    let pied = prim(occt::Prim::Box { dx: 10.0, dy: 10.0, dz: 20.0 });
    let plateau = decale(&prim(occt::Prim::Box { dx: 40.0, dy: 40.0, dz: 4.0 }), Vec3::new(-15.0, -15.0, 20.0));
    let table = booleen::calculer(&pied, &plateau, booleen::Op::Union).unwrap();
    let a = impression::analyser(&tris(&table), &table, 45.0, 0.8);
    assert_eq!((a.trous, a.erreurs), (0, 0));
    assert!(a.zmin.abs() < 1e-4);
    assert!((a.aire_surplombs - (1600.0 - 100.0)).abs() < 1.0, "surplombs {}", a.aire_surplombs);
    assert!(a.fines.is_empty(), "pas de paroi fine");
    assert!((a.volume - (2000.0 + 6400.0)).abs() < 1.0);
    // plaque de 0,5 mm : paroi trop fine (dessus et dessous)
    let plaque = prim(occt::Prim::Box { dx: 30.0, dy: 30.0, dz: 0.5 });
    let b = impression::analyser(&tris(&plaque), &plaque, 45.0, 0.8);
    assert!((b.aire_fines - 1800.0).abs() < 1.0, "parois fines {}", b.aire_fines);
    // orientation conseillée de la table : à l'envers (plateau en bas), plus aucun surplomb
    let (n, c, actuel) = impression::orientation(&table, 45.0).unwrap();
    assert!(c < 1.0 && actuel > 1000.0 && n.z > 0.99, "orientation {n:?} {c} {actuel}");
}

#[test]
fn decoupe_par_cadre() {
    // cube de 20 mm, ligne tracée en travers à x = 5 (de y = -10 à 30) : cadre vertical
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 20.0, dz: 20.0 });
    let c = booleen::Cadre::new(Vec3::new(5.0, -10.0, 0.0), Vec3::new(5.0, 30.0, 0.0), 2);
    let (p, q) = booleen::couper(&a, &c).unwrap();
    for m in [&p, &q] { let d = defects::analyze(m); assert_eq!((d.holes, d.face_errors), (0, 0)); }
    let mut v = [volume(&p), volume(&q)]; v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert!(ecart_rel(v[0], 5.0 * 400.0) < 1e-9 && ecart_rel(v[1], 15.0 * 400.0) < 1e-9, "volumes {v:?}");
    // ligne en biais (45°) passant par le centre : deux moitiés égales
    let c = booleen::Cadre::new(Vec3::new(-10.0, -10.0, 0.0), Vec3::new(30.0, 30.0, 0.0), 2);
    let (p, q) = booleen::couper(&a, &c).unwrap();
    assert!(ecart_rel(volume(&p), 4000.0) < 1e-6 && ecart_rel(volume(&q), 4000.0) < 1e-6);
    // ligne trop courte (emporte-pièce) : n'entame que l'emprise de la ligne, de y = 0 à 10
    let c = booleen::Cadre::new(Vec3::new(5.0, 10.0, 0.0), Vec3::new(5.0, -10.0, 0.0), 2);
    let (p, q) = booleen::couper(&a, &c).unwrap();
    assert!(ecart_rel(volume(&p) + volume(&q), 8000.0) < 1e-9);
    // cadre élevé le long de Y (tracé en vue de face) : coupe horizontale à z = 5
    let c = booleen::Cadre::new(Vec3::new(-10.0, 0.0, 5.0), Vec3::new(30.0, 0.0, 5.0), 1);
    let (p, q) = booleen::couper(&a, &c).unwrap();
    let mut v = [volume(&p), volume(&q)]; v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert!(ecart_rel(v[0], 5.0 * 400.0) < 1e-9, "coupe horizontale {v:?}");
    // cadre incliné de 45° autour d'un trait le long de Y à x = 10 (plan x = z + 10) : prisme de 1000 mm³
    // détaché ; à 135° (plan x + z = 10) idem de l'autre côté
    for incl in [45.0, 135.0] {
        let mut c = booleen::Cadre::new(Vec3::new(10.0, -10.0, 0.0), Vec3::new(10.0, 30.0, 0.0), 2);
        c.incl = incl;
        let (p, q) = booleen::couper(&a, &c).unwrap();
        for m in [&p, &q] { let d = defects::analyze(m); assert_eq!((d.holes, d.face_errors), (0, 0)); }
        let mut v = [volume(&p), volume(&q)]; v.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert!(ecart_rel(v[0], 1000.0) < 1e-6 && ecart_rel(v[1], 7000.0) < 1e-6, "coupe inclinée {incl}° : {v:?}");
    }
    // cadre à côté de l'objet : refus clair
    let c = booleen::Cadre::new(Vec3::new(50.0, -10.0, 0.0), Vec3::new(50.0, 30.0, 0.0), 2);
    assert!(booleen::couper(&a, &c).is_err());
}

#[test]
fn booleen_sur_maillage_reel() {
    // sphère facettée (posée sur le plateau : z de 0 à 24) moins le cube qui couvre x ≥ 0
    let s = prim(occt::Prim::Sphere { r: 12.0 });
    let c = decale(&prim(occt::Prim::Box { dx: 30.0, dy: 30.0, dz: 30.0 }), Vec3::new(0.0, -15.0, -3.0));
    let r = booleen::calculer(&s, &c, booleen::Op::Soustraction).unwrap();
    let d = defects::analyze(&r);
    assert_eq!((d.holes, d.face_errors), (0, 0), "demi-sphère ouverte");
    assert!(ecart_rel(volume(&r), volume(&s) * 0.5) < 0.01, "demi-sphère : {} / {}", volume(&r), volume(&s) * 0.5);
    assert!(boite(&r).1.x.abs() < 1e-4, "la coupe doit être exactement à x = 0");
}
/// Gros maillage de test (fichier .obj) : chemin donné par la variable PATCH3D_GROS_OBJ.
fn gros_maillage() -> Option<String> {
    std::env::var("PATCH3D_GROS_OBJ").ok().filter(|p| std::path::Path::new(p).exists())
}

#[test]
#[ignore]
fn perf_booleen_gros_maillage() {
    let Some(p) = gros_maillage() else { return; };
    let (mut m, _) = load_model(&p).unwrap();
    let t0 = std::time::Instant::now();
    for _ in 0..25 { m.verts = defects::repair(&m.verts); let d = defects::analyze(&m.verts); if d.holes == 0 && d.face_errors == 0 { break; } }
    let d = defects::analyze(&m.verts);
    eprintln!("PERF réparation {:.1} s : trous {} err {}", t0.elapsed().as_secs_f32(), d.holes, d.face_errors);
    let (lo, hi) = boite(&m.verts);
    let c = decale(&prim(occt::Prim::Box { dx: 2000.0, dy: 2000.0, dz: 2000.0 }), Vec3::new(-1000.0, -1000.0, (lo.z + hi.z) * 0.5));
    let t = std::time::Instant::now();
    let r = booleen::calculer(&m.verts, &c, booleen::Op::Soustraction);
    match r { Ok(v) => { let d = defects::analyze(&v); eprintln!("PERF {} tri -> {} tri en {:.2} s, trous {} err {}", m.verts.len()/18, v.len()/18, t.elapsed().as_secs_f32(), d.holes, d.face_errors);
                         let r2 = defects::repair(&v); let d2 = defects::analyze(&r2); eprintln!("PERF après réparation : trous {} err {}, vol {} -> {}", d2.holes, d2.face_errors, volume(&v), volume(&r2));
                         let m2 = Manifold::from_mesh_gl64(&booleen::test_vers(&m.verts)); eprintln!("PERF entrée stricte : {:?}", m2.status()); }
              Err(e) => eprintln!("PERF erreur {e} en {:.2} s", t.elapsed().as_secs_f32()) }
}

#[test]
#[ignore]
fn soustraction_forme_complexe_sans_defaut() {
    // Femme (456k triangles, ouverte et défectueuse) moins un cylindre qui la traverse :
    // réparation auto avant, résultat sans aucun défaut (pincements retirés), volume cohérent
    let Some(p) = gros_maillage() else { return; };
    let (m, _) = load_model(&p).unwrap();
    let (lo, hi) = boite(&m.verts);
    let c = (lo + hi) * 0.5;
    let r = ((hi - lo).x.min((hi - lo).y) * 0.15) as f64;
    let cyl = decale(&prim(occt::Prim::Cylinder { r, h: ((hi - lo).z * 2.0) as f64 }), Vec3::new(c.x, c.y, lo.z - (hi - lo).z * 0.5));
    let v = booleen::calculer(&m.verts, &cyl, booleen::Op::Soustraction).unwrap();
    let d = defects::analyze(&v);
    eprintln!("Femme − cylindre : {} tri, trous {} err {}", v.len() / 18, d.holes, d.face_errors);
    assert_eq!((d.holes, d.face_errors), (0, 0));
    // découpe de la même forme : deux morceaux sans défaut
    let cadre = booleen::Cadre::new(Vec3::new(c.x, lo.y - 10.0, 0.0), Vec3::new(c.x, hi.y + 10.0, 0.0), 2);
    let (a, b) = booleen::couper(&m.verts, &cadre).unwrap();
    for (k, x) in [&a, &b].iter().enumerate() { let d = defects::analyze(x); eprintln!("morceau {k} : trous {} err {}", d.holes, d.face_errors); assert_eq!((d.holes, d.face_errors), (0, 0)); }
}


#[test]
#[ignore]
fn chute_femme_poses() {
    // lâchée dans 8 positions : temps de calcul et pose finale (dos/haut d'origine et avant)
    let Some(p) = gros_maillage() else { return; };
    let (m, _) = load_model(&p).unwrap();
    let liste: Vec<(&str, glam::Mat3)> = match std::env::var("CHUTE_CAS").as_deref() { Ok("1") => vec![("Femme 50° Y", glam::Mat3::from_rotation_y(0.87))],
        _ => vec![("Femme droite", glam::Mat3::IDENTITY), ("Femme 25° X", glam::Mat3::from_rotation_x(0.436)), ("Femme 50° Y", glam::Mat3::from_rotation_y(0.87)),
                  ("Femme 90° Y", glam::Mat3::from_rotation_y(1.5708)), ("Femme 90° X", glam::Mat3::from_rotation_x(1.5708)), ("Femme 180° X", glam::Mat3::from_rotation_x(3.1416)),
                  ("Femme 130° X", glam::Mat3::from_rotation_x(2.27)), ("Femme -90° X", glam::Mat3::from_rotation_x(-1.5708))] };
    for (nom, r) in liste {
        let mut v = m.verts.clone();
        for c in v.chunks_exact_mut(6) { let q = r * Vec3::new(c[0], c[1], c[2]); c[0] = q.x; c[1] = q.y; c[2] = q.z; }
        let (lo, hi) = boite(&v);
        let t = std::time::Instant::now();
        let c = gravite::simuler(&v, ((hi - lo).length() * 0.4).max(lo.z)).unwrap();
        let (qf, _) = *c.images.last().unwrap();
        let haut = qf * (r * Vec3::Z);
        let face = qf * (r * Vec3::NEG_Y); // le visage regarde vers -Y à l'origine
        eprintln!("CHUTE {nom:14} : {:.1} s simulées, calcul {:.2} s -> dos/haut d'origine ({:.2}, {:.2}, {:.2}), avant ({:.2}, {:.2}, {:.2})", c.images.len() as f32 / 60.0, t.elapsed().as_secs_f32(), haut.x, haut.y, haut.z, face.x, face.y, face.z);
    }
}


#[test]
#[ignore]
fn creuser_femme() {
    let Some(p) = gros_maillage() else { return; };
    let (m, _) = load_model(&p).unwrap();
    let t = std::time::Instant::now();
    let (r, eco) = booleen::creuser(&m.verts, 1.5, Some(3.0), crate::perf::liss_res()).unwrap();
    let d = defects::analyze(&r);
    eprintln!("CREUX Femme : {} tri, trous {} err {}, économie {:.0} %, {:.1} s", r.len() / 18, d.holes, d.face_errors, eco * 100.0, t.elapsed().as_secs_f32());
    assert_eq!((d.holes, d.face_errors), (0, 0));
}

#[test]
#[ignore]
fn fabrique_3mf_multi() {
    let a = prim(occt::Prim::Box { dx: 20.0, dy: 20.0, dz: 20.0 });
    let b = decale(&prim(occt::Prim::Sphere { r: 8.0 }), Vec3::new(40.0, 5.0, 0.0));
    let c = decale(&prim(occt::Prim::Cylinder { r: 4.0, h: 30.0 }), Vec3::new(-30.0, 10.0, 0.0));
    let cols_b: Vec<f32> = (0..b.len() / 6).flat_map(|_| [1.0, 0.1, 0.1]).collect();
    threemf::write_objets(&std::env::temp_dir().join("multi.3mf"), &[(&a, None), (&b, Some(&cols_b)), (&c, None)]).unwrap();
}

#[test]
#[ignore]
fn diag_trancheurs() {
    for (t, l) in impression::TRANCHEURS.iter().zip(impression::detecter()) { eprintln!("TRANCHEUR {} : {:?}", t.nom, l); }
}
