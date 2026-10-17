//! Format 3MF (archive zip + XML « 3D Manufacturing Format »).
//! Lecture : unités, objets multiples, assemblages (components + transformations), objets rangés
//! dans des sous-fichiers (Bambu Studio / OrcaSlicer : p:path), couleurs (basematerials,
//! colorgroup). Écriture : un objet maillé en millimètres.

use std::collections::HashMap;
use std::io::{Read, Write};

/// Attributs d'une balise XML (analyse minimale, suffisante pour le 3MF).
fn attrs(tag: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() && !b[i].is_ascii_whitespace() { i += 1; } // nom de la balise
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') { i += 1; }
        let k0 = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() { i += 1; }
        let key = tag[k0..i].to_string();
        while i < b.len() && b[i] != b'"' && b[i] != b'\'' { i += 1; }
        if i >= b.len() { break; }
        let q = b[i]; i += 1; let v0 = i;
        while i < b.len() && b[i] != q { i += 1; }
        m.insert(key, tag[v0..i.min(b.len())].to_string());
        i += 1;
    }
    m
}
/// Nom de balise sans préfixe d'espace de noms (« p:component » -> « component »).
fn nom(tag: &str) -> &str {
    let n = tag.split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("");
    n.rsplit(':').next().unwrap_or(n)
}
fn val<'a>(a: &'a HashMap<String, String>, k: &str) -> Option<&'a String> {
    a.get(k).or_else(|| a.iter().find(|(kk, _)| kk.rsplit(':').next() == Some(k)).map(|(_, v)| v))
}
fn couleur(s: &str) -> [f32; 3] {
    let h = s.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("cc"), 16).unwrap_or(204) as f32 / 255.0;
    [p(0), p(2), p(4)]
}

type M43 = [f32; 12]; // m00 m01 m02 m10 m11 m12 m20 m21 m22 m30 m31 m32 (convention 3MF : p' = p·M)
const ID: M43 = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];
fn mat(s: Option<&String>) -> M43 {
    let v: Vec<f32> = s.map(|t| t.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    if v.len() == 12 { let mut m = [0.0; 12]; m.copy_from_slice(&v); m } else { ID }
}
fn appl(m: &M43, p: [f32; 3]) -> [f32; 3] {
    [p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9], p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10], p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11]]
}
fn mul(a: &M43, b: &M43) -> M43 { // a puis b
    let r = |i: usize, j: usize| a[i * 3] * b[j] + a[i * 3 + 1] * b[3 + j] + a[i * 3 + 2] * b[6 + j];
    let mut m = [0.0; 12];
    for i in 0..3 { for j in 0..3 { m[i * 3 + j] = r(i, j); } }
    let t = appl(b, [a[9], a[10], a[11]]);
    m[9] = t[0]; m[10] = t[1]; m[11] = t[2];
    m
}

#[derive(Default)]
struct Objet {
    v: Vec<[f32; 3]>,
    t: Vec<([u32; 3], Option<[[f32; 3]; 3]>)>, // triangle + couleurs par sommet éventuelles
    comps: Vec<(String, String, M43)>,           // (fichier, id, transformation)
    pid: Option<String>, pindex: usize,
}
#[derive(Default)]
struct Modele { objets: HashMap<String, Objet>, couleurs: HashMap<String, Vec<[f32; 3]>>, unite: f32, build: Vec<(String, String, M43)> }

fn lire_modele(xml: &str, fichier: &str) -> Modele {
    let mut md = Modele { unite: 1.0, ..Default::default() };
    let mut cur: Option<(String, Objet)> = None;
    let mut groupe: Option<(String, Vec<[f32; 3]>)> = None;
    let mut i = 0;
    let b = xml.as_bytes();
    while let Some(o) = xml[i..].find('<') {
        let s = i + o + 1;
        let Some(e) = xml[s..].find('>') else { break };
        let tag = &xml[s..s + e];
        i = s + e + 1;
        if tag.starts_with('?') || tag.starts_with('!') { continue; }
        let fin = tag.starts_with('/');
        let t = tag.trim_start_matches('/');
        let n = nom(t);
        let auto = tag.ends_with('/');
        let _ = b;
        if fin {
            match n {
                "object" => if let Some((id, ob)) = cur.take() { md.objets.insert(id, ob); },
                "basematerials" | "colorgroup" => if let Some((id, c)) = groupe.take() { md.couleurs.insert(id, c); },
                _ => {}
            }
            continue;
        }
        let a = attrs(t);
        match n {
            "model" => {
                md.unite = match val(&a, "unit").map(|s| s.as_str()) {
                    Some("micron") => 0.001, Some("centimeter") => 10.0, Some("meter") => 1000.0,
                    Some("inch") => 25.4, Some("foot") => 304.8, _ => 1.0,
                };
            }
            "object" => {
                let ob = Objet { pid: val(&a, "pid").cloned(), pindex: val(&a, "pindex").and_then(|s| s.parse().ok()).unwrap_or(0), ..Default::default() };
                let id = val(&a, "id").cloned().unwrap_or_default();
                if auto { md.objets.insert(id, ob); } else { cur = Some((id, ob)); }
            }
            "basematerials" | "colorgroup" => {
                groupe = Some((val(&a, "id").cloned().unwrap_or_default(), Vec::new()));
                if auto { if let Some((id, c)) = groupe.take() { md.couleurs.insert(id, c); } }
            }
            "base" => if let Some((_, c)) = groupe.as_mut() { c.push(couleur(val(&a, "displaycolor").map_or("#cccccc", |s| s.as_str()))); },
            "color" => if let Some((_, c)) = groupe.as_mut() { c.push(couleur(val(&a, "color").map_or("#cccccc", |s| s.as_str()))); },
            "vertex" => if let Some((_, ob)) = cur.as_mut() {
                let f = |k: &str| val(&a, k).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                ob.v.push([f("x"), f("y"), f("z")]);
            },
            "triangle" => if let Some((_, ob)) = cur.as_mut() {
                let u = |k: &str| val(&a, k).and_then(|s| s.parse::<u32>().ok());
                if let (Some(v1), Some(v2), Some(v3)) = (u("v1"), u("v2"), u("v3")) {
                    // couleur : pid/p1..p3 du triangle, sinon couleur par défaut de l'objet
                    let pid = val(&a, "pid").cloned().or_else(|| ob.pid.clone());
                    let p1 = u("p1").map(|x| x as usize).unwrap_or(ob.pindex);
                    let p2 = u("p2").map(|x| x as usize).unwrap_or(p1);
                    let p3 = u("p3").map(|x| x as usize).unwrap_or(p1);
                    let c = pid.and_then(|p| md.couleurs.get(&p)).map(|g| {
                        let k = |i: usize| g.get(i).copied().unwrap_or([0.8, 0.8, 0.8]);
                        [k(p1), k(p2), k(p3)]
                    });
                    ob.t.push(([v1, v2, v3], c));
                }
            },
            "component" => if let Some((_, ob)) = cur.as_mut() {
                let chemin = val(&a, "path").cloned().unwrap_or_else(|| fichier.to_string());
                ob.comps.push((chemin, val(&a, "objectid").cloned().unwrap_or_default(), mat(val(&a, "transform"))));
            },
            "item" => md.build.push((fichier.to_string(), val(&a, "objectid").cloned().unwrap_or_default(), mat(val(&a, "transform")))),
            _ => {}
        }
    }
    md
}

/// Lit un 3MF : (sommets [pos nrm]×3×triangles en mm, couleurs [r g b] par sommet si présentes),
/// tous les objets réunis.
pub fn read(path: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> {
    let objs = read_objets(path)?;
    let a_couleur = objs.iter().any(|o| o.1.is_some());
    let (mut v, mut c) = (Vec::new(), Vec::new());
    for (ov, oc) in objs {
        if a_couleur { match oc { Some(cc) => c.extend_from_slice(&cc), None => for _ in 0..ov.len() / 6 { c.extend_from_slice(&[0.93, 0.80, 0.33]); } } }
        v.extend_from_slice(&ov);
    }
    Ok((v, if a_couleur { Some(c) } else { None }))
}

/// Lit un 3MF objet par objet (un par élément de la plaque, composants développés, positions
/// conservées) : [(sommets, couleurs éventuelles)].
pub fn read_objets(path: &str) -> Result<Vec<(Vec<f32>, Option<Vec<f32>>)>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("{} : {e}", crate::trc("tmf_illisible")))?;
    let mut modeles: HashMap<String, Modele> = HashMap::new();
    let mut principal = String::new();
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let n = format!("/{}", e.name().trim_start_matches('/'));
        if !n.to_ascii_lowercase().ends_with(".model") { continue; }
        let mut s = String::new();
        e.read_to_string(&mut s).map_err(|e| e.to_string())?;
        if principal.is_empty() || n.eq_ignore_ascii_case("/3D/3dmodel.model") { principal = n.clone(); }
        modeles.insert(n.clone(), lire_modele(&s, &n));
    }
    let md = modeles.get(&principal).ok_or("aucun modèle 3D dans l'archive")?;
    let unite = md.unite;
    // développe récursivement objets / composants avec leurs transformations
    fn emit(modeles: &HashMap<String, Modele>, fichier: &str, id: &str, m: &M43, prof: u32,
            v: &mut Vec<f32>, c: &mut Vec<f32>, a_couleur: &mut bool) {
        if prof > 32 { return; }
        let Some(md) = modeles.get(fichier) else { return };
        let Some(ob) = md.objets.get(id) else { return };
        for (tri, col) in &ob.t {
            let p = tri.map(|k| appl(m, ob.v.get(k as usize).copied().unwrap_or([0.0; 3])));
            let n = {
                let (u, w) = ([p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]], [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]]);
                let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-20);
                [n[0] / l, n[1] / l, n[2] / l]
            };
            for k in 0..3 {
                v.extend_from_slice(&[p[k][0], p[k][1], p[k][2], n[0], n[1], n[2]]);
                let cc = col.map(|cs| cs[k]).unwrap_or([0.93, 0.80, 0.33]);
                if col.is_some() { *a_couleur = true; }
                c.extend_from_slice(&cc);
            }
        }
        for (f, cid, cm) in &ob.comps {
            let fic = if modeles.contains_key(f) { f.as_str() } else { fichier };
            emit(modeles, fic, cid, &mul(cm, m), prof + 1, v, c, a_couleur);
        }
    }
    let items: Vec<(String, String, M43)> = if md.build.is_empty() { md.objets.keys().map(|k| (principal.clone(), k.clone(), ID)).collect() } else { md.build.clone() };
    let mut objs = Vec::new();
    for (fic, id, m) in &items {
        let (mut v, mut c, mut a_couleur) = (Vec::new(), Vec::new(), false);
        emit(&modeles, fic, id, m, 0, &mut v, &mut c, &mut a_couleur);
        if v.is_empty() { continue; }
        if unite != 1.0 { for s in v.chunks_exact_mut(6) { s[0] *= unite; s[1] *= unite; s[2] *= unite; } }
        objs.push((v, if a_couleur { Some(c) } else { None }));
    }
    if objs.is_empty() { return Err(crate::trc("tmf_vide").into()); }
    Ok(objs)
}

/// Écrit un 3MF (un objet, en millimètres) à partir de sommets [pos nrm]×3×triangles.
pub fn write(path: &std::path::Path, verts: &[f32]) -> std::io::Result<()> {
    write_objets(path, &[(verts, None)])
}

/// Écrit un 3MF à plusieurs objets séparés (en millimètres, à leurs positions), chacun avec ses
/// couleurs par sommet s'il en a (groupe de couleurs par face, extension « matériaux » du 3MF,
/// lu par PrusaSlicer, Bambu Studio, OrcaSlicer…).
pub fn write_objets(path: &std::path::Path, objs: &[(&[f32], Option<&[f32]>)]) -> std::io::Result<()> {
    let mut res = String::new();
    let mut build = String::new();
    for (k, (verts, cols)) in objs.iter().enumerate() {
        let oid = k + 1;
        // soudure exacte des positions (indices partagés, comme l'exigent les slicers)
        let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
        let mut vx = String::new();
        let mut tr = String::new();
        let mut idx = Vec::with_capacity(verts.len() / 6);
        for c in verts.chunks_exact(6) {
            let key = [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()];
            let n = ids.len() as u32;
            let id = *ids.entry(key).or_insert_with(|| { vx += &format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", c[0], c[1], c[2]); n });
            idx.push(id);
        }
        // couleurs : une teinte par sommet de triangle, regroupées (#RRGGBB)
        let cols = cols.filter(|c| c.len() / 3 == verts.len() / 6);
        let mut teintes: HashMap<[u8; 3], usize> = HashMap::new();
        let mut groupe = String::new();
        let gid = 1000 + oid;
        let mut teinte = |c: &[f32]| -> usize {
            let q = [c[0], c[1], c[2]].map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
            let n = teintes.len();
            *teintes.entry(q).or_insert_with(|| { groupe += &format!("<m:color color=\"#{:02X}{:02X}{:02X}\"/>\n", q[0], q[1], q[2]); n })
        };
        for (t, tri) in idx.chunks_exact(3).enumerate() {
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] { continue; }
            match cols {
                Some(cc) => {
                    let p: Vec<usize> = (0..3).map(|j| teinte(&cc[(t * 3 + j) * 3..(t * 3 + j) * 3 + 3])).collect();
                    tr += &format!("<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"{gid}\" p1=\"{}\" p2=\"{}\" p3=\"{}\"/>\n", tri[0], tri[1], tri[2], p[0], p[1], p[2]);
                }
                None => tr += &format!("<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n", tri[0], tri[1], tri[2]),
            }
        }
        if cols.is_some() { res += &format!("<m:colorgroup id=\"{gid}\">\n{groupe}</m:colorgroup>\n"); }
        res += &format!("<object id=\"{oid}\" name=\"Objet {oid}\" type=\"model\">\n<mesh>\n<vertices>\n{vx}</vertices>\n<triangles>\n{tr}</triangles>\n</mesh>\n</object>\n");
        build += &format!("<item objectid=\"{oid}\"/>\n");
    }
    let modele = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"fr-FR\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" xmlns:m=\"http://schemas.microsoft.com/3dmanufacturing/material/2015/02\">\n<metadata name=\"Application\">3D-Patch</metadata>\n<resources>\n{res}</resources>\n<build>\n{build}</build>\n</model>\n");
    let f = std::fs::File::create(path)?;
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let err = |e: zip::result::ZipError| std::io::Error::new(std::io::ErrorKind::Other, e.to_string());
    z.start_file("[Content_Types].xml", o).map_err(err)?;
    z.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>\n")?;
    z.start_file("_rels/.rels", o).map_err(err)?;
    z.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/></Relationships>\n")?;
    z.start_file("3D/3dmodel.model", o).map_err(err)?;
    z.write_all(modele.as_bytes())?;
    z.finish().map_err(err)?;
    Ok(())
}
