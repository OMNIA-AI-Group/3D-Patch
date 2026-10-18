//! Conception SolidCraft (.solidcraft, CAO paramétrique libre) : lecture et écriture directes, sans
//! SolidCraft installé. Format décrit dans docs/formats/design-container.md du projet SolidCraft :
//! archive zip = container.json (« solidcraft-container/1 ») + document.json (historique
//! paramétrique « solidcraft/N ») + blobs/<sha256>.<mesh|step> ; ou JSON seul (tout en ligne).
//!
//! Lecture : maillages importés (fonctions `mesh_import`, blobs SCMESH01), STEP importés (fonctions
//! `import`, lus par OpenCASCADE), placés par les occurrences de leur composant ; maillages d'affichage
//! enregistrés (cache/preview.*) pour les pièces construites dans SolidCraft. Une pièce purement
//! paramétrique (esquisses, extrusions…) sans géométrie enregistrée ne peut pas être recalculée ici :
//! si SolidCraft est installé, son outil en ligne de commande la calcule (en 3MF) en secours.
//! Écriture : chaque objet devient une fonction `mesh_import` dans son composant, comme un import
//! de STL dans SolidCraft.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const MESH_MAGIC: &[u8; 8] = b"SCMESH01";
const FORMAT_DOC: &str = "solidcraft/26";
const FORMAT_CONTENEUR: &str = "solidcraft-container/1";

/// Objets lus : (sommets [x y z nx ny nz] par sommet, triangles à plat ; couleurs RVB par sommet).
pub type Objets = Vec<(Vec<f32>, Option<Vec<f32>>)>;

/// Matrice SolidCraft : rangée par colonnes, translation dans m[3].
type Mat = [[f64; 4]; 4];
const IDENTITE: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn mat_mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [[0.0; 4]; 4];
    for (c, col) in r.iter_mut().enumerate() {
        for (row, cell) in col.iter_mut().enumerate() { *cell = (0..4).map(|k| a[k][row] * b[c][k]).sum(); }
    }
    r
}
fn appliquer(m: &Mat, p: [f64; 3]) -> [f64; 3] {
    [m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
     m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
     m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2]]
}

fn illisible(detail: impl std::fmt::Display) -> String { format!("{} ({detail})", crate::trc("sc_illisible")) }

/// Maillage SCMESH01 : u64 nb de coordonnées, u64 nb d'indices, puis f64 puis u32 (petit-boutiste).
fn decoder_maillage(b: &[u8]) -> Result<(Vec<f64>, Vec<u32>), String> {
    if b.len() < 24 || &b[..8] != MESH_MAGIC { return Err(illisible("SCMESH01")); }
    let nc = u64::from_le_bytes(b[8..16].try_into().unwrap()) as usize;
    let ni = u64::from_le_bytes(b[16..24].try_into().unwrap()) as usize;
    if nc % 3 != 0 || ni % 3 != 0 || nc.checked_mul(8).and_then(|x| x.checked_add(ni.checked_mul(4)?)).and_then(|x| x.checked_add(24)) != Some(b.len()) {
        return Err(illisible("taille du maillage"));
    }
    let pos: Vec<f64> = b[24..24 + nc * 8].chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
    let idx: Vec<u32> = b[24 + nc * 8..].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    if pos.iter().any(|v| !v.is_finite()) || idx.iter().any(|&i| i as usize >= nc / 3) { return Err(illisible("maillage invalide")); }
    Ok((pos, idx))
}
fn encoder_maillage(pos: &[f64], idx: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(24 + pos.len() * 8 + idx.len() * 4);
    b.extend_from_slice(MESH_MAGIC);
    b.extend_from_slice(&(pos.len() as u64).to_le_bytes());
    b.extend_from_slice(&(idx.len() as u64).to_le_bytes());
    for v in pos { b.extend_from_slice(&v.to_le_bytes()); }
    for i in idx { b.extend_from_slice(&i.to_le_bytes()); }
    b
}

/// Maillage indexé (repère monde) -> soupe de triangles à normales plates (+ couleur unie éventuelle).
fn vers_objet(pos: &[f64], idx: &[u32], m: &Mat, couleur: Option<[f32; 3]>) -> (Vec<f32>, Option<Vec<f32>>) {
    let mut v = Vec::with_capacity(idx.len() * 6);
    for t in idx.chunks_exact(3) {
        let p: Vec<glam::Vec3> = t.iter().map(|&i| { let q = appliquer(m, [pos[i as usize * 3], pos[i as usize * 3 + 1], pos[i as usize * 3 + 2]]); glam::Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) }).collect();
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        for q in p { v.extend_from_slice(&[q.x, q.y, q.z, n.x, n.y, n.z]); }
    }
    let cols = couleur.map(|c| c.repeat(v.len() / 6));
    (v, cols)
}

fn couleur_de(v: &Value) -> Option<[f32; 3]> {
    let a = v.get("color")?.as_array()?;
    if a.len() != 3 { return None; }
    let c: Vec<f32> = a.iter().filter_map(|x| x.as_f64()).map(|x| x.clamp(0.0, 1.0) as f32).collect();
    (c.len() == 3).then(|| [c[0], c[1], c[2]])
}

/// Placement d'un composant dans le monde : sa première occurrence, composée avec celles de ses parents.
fn placement(doc: &Value, composant: u64) -> Mat {
    let occs = doc.get("occurrences").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut m = IDENTITE;
    let mut c = composant;
    for _ in 0..1000 {
        if c == 0 { break; }
        let Some(o) = occs.iter().find(|o| o.get("component").and_then(Value::as_u64) == Some(c)) else { break };
        let t: Option<Mat> = serde_json::from_value(o.get("transform").cloned().unwrap_or(Value::Null)).ok();
        m = mat_mul(&t.unwrap_or(IDENTITE), &m);
        c = o.get("parent").and_then(Value::as_u64).unwrap_or(0);
    }
    m
}

/// Accès aux parties du conteneur (ou rien pour un document JSON seul).
struct Paquet { zip: Option<zip::ZipArchive<std::io::Cursor<Vec<u8>>>> }
impl Paquet {
    fn partie(&mut self, nom: &str) -> Option<Vec<u8>> {
        let z = self.zip.as_mut()?;
        let mut f = z.by_name(nom).ok()?;
        if f.size() > 512 << 20 { return None; }
        let mut b = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut b).ok()?;
        Some(b)
    }
    /// Blob référencé, contrôlé par son empreinte (nom = sha256 du contenu).
    fn blob(&mut self, nom: &str) -> Result<Vec<u8>, String> {
        let b = self.partie(nom).ok_or_else(|| illisible(format!("{nom} absent")))?;
        let attendu = nom.rsplit('/').next().and_then(|n| n.split('.').next()).unwrap_or("");
        if hex(&Sha256::digest(&b)) != attendu { return Err(illisible(format!("{nom} altéré"))); }
        Ok(b)
    }
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

pub fn lire(path: &str) -> Result<Objets, String> {
    let octets = std::fs::read(path).map_err(|e| e.to_string())?;
    let empreinte = hex(&Sha256::digest(&octets));
    let (doc, mut paquet): (Value, Paquet) = if octets.starts_with(b"PK\x03\x04") {
        let mut p = Paquet { zip: Some(zip::ZipArchive::new(std::io::Cursor::new(octets)).map_err(illisible)?) };
        let conteneur: Value = serde_json::from_slice(&p.partie("container.json").ok_or_else(|| illisible("container.json"))?).map_err(illisible)?;
        let f = conteneur.get("format").and_then(Value::as_str).unwrap_or("");
        if !f.starts_with("solidcraft-container/") && !f.starts_with("solvecraft-container/") { return Err(illisible(f)); }
        let doc = serde_json::from_slice(&p.partie("document.json").ok_or_else(|| illisible("document.json"))?).map_err(illisible)?;
        (doc, p)
    } else {
        (serde_json::from_slice(&octets).map_err(illisible)?, Paquet { zip: None })
    };
    if !doc.get("format").and_then(Value::as_str).map_or(false, |f| f.starts_with("solidcraft/") || f.starts_with("solvecraft/")) {
        return Err(illisible("document.json"));
    }
    let features = doc.get("features").and_then(Value::as_array).cloned().unwrap_or_default();
    let actives: Vec<&Value> = features.iter().filter(|f| !f.get("suppressed").and_then(Value::as_bool).unwrap_or(false)).collect();
    let parametriques = actives.iter().filter(|f| !matches!(f.get("type").and_then(Value::as_str), Some("mesh_import") | Some("import"))).count();

    // pièce construite dans SolidCraft : ses maillages d'affichage enregistrés (déjà en repère monde)
    if parametriques > 0 {
        if let (Some(man), Some(data)) = (paquet.partie("cache/preview.json"), paquet.partie("cache/preview.bin")) {
            let man: Value = serde_json::from_slice(&man).map_err(illisible)?;
            let mut objs = Objets::new();
            for e in man.get("meshes").and_then(Value::as_array).cloned().unwrap_or_default() {
                let (o, l) = (e.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize, e.get("length").and_then(Value::as_u64).unwrap_or(0) as usize);
                let tranche = data.get(o..o.saturating_add(l)).ok_or_else(|| illisible("cache/preview.bin"))?;
                let (pos, idx) = decoder_maillage(tranche)?;
                objs.push(vers_objet(&pos, &idx, &IDENTITE, couleur_de(&e)));
            }
            if !objs.is_empty() { return Ok(objs); }
        }
    }

    // pièce purement paramétrique : SolidCraft (s'il est installé) recalcule sa géométrie — une fois par
    // version du fichier (résultat gardé en cache, d'après son empreinte)
    if parametriques > 0 {
        if let Some(o) = par_solidcraft(path, &empreinte) { return Ok(o); }
    }

    let mut objs = Objets::new();
    for f in &actives {
        let m = placement(&doc, f.get("component").and_then(Value::as_u64).unwrap_or(0));
        match f.get("type").and_then(Value::as_str) {
            Some("mesh_import") => for mesh in f.get("meshes").and_then(Value::as_array).cloned().unwrap_or_default() {
                let (pos, idx) = match mesh.get("mesh_blob").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    Some(nom) => decoder_maillage(&paquet.blob(nom)?)?,
                    None => (serde_json::from_value(mesh.get("positions").cloned().unwrap_or(Value::Null)).map_err(illisible)?,
                             serde_json::from_value(mesh.get("triangles").cloned().unwrap_or(Value::Null)).map_err(illisible)?),
                };
                if idx.iter().any(|&i| i as usize * 3 + 2 >= pos.len()) { return Err(illisible("maillage invalide")); }
                objs.push(vers_objet(&pos, &idx, &m, couleur_de(&mesh)));
            },
            Some("import") => {
                // STEP gardé dans la conception (en ligne ou en blob) : lu par OpenCASCADE via un fichier temporaire
                let texte = match f.get("step_blob").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    Some(nom) => paquet.blob(nom)?,
                    None => f.get("step").and_then(Value::as_str).unwrap_or("").as_bytes().to_vec(),
                };
                if texte.is_empty() { continue; }
                let tmp = std::env::temp_dir().join(format!("3d-patch-solidcraft-{}-{}.step", std::process::id(), objs.len()));
                std::fs::write(&tmp, &texte).map_err(|e| e.to_string())?;
                let r = crate::occt::import_cad(tmp.to_str().unwrap_or(""), false, crate::occt::DEFLEXION);
                let _ = std::fs::remove_file(&tmp);
                let mesh = r.map_err(|rc| format!("{} (STEP, code {rc})", crate::trc("cao_illisible")))?;
                // sommets OCCT -> repère du composant
                let mut v = mesh.verts;
                for c in v.chunks_exact_mut(6) {
                    let q = appliquer(&m, [c[0] as f64, c[1] as f64, c[2] as f64]);
                    c[0] = q[0] as f32; c[1] = q[1] as f32; c[2] = q[2] as f32;
                }
                for t in v.chunks_exact_mut(18) {
                    let p = |k: usize| glam::Vec3::new(t[k * 6], t[k * 6 + 1], t[k * 6 + 2]);
                    let n = (p(1) - p(0)).cross(p(2) - p(0)).normalize_or_zero();
                    for k in 0..3 { t[k * 6 + 3] = n.x; t[k * 6 + 4] = n.y; t[k * 6 + 5] = n.z; }
                }
                objs.push((v, None));
            }
            _ => {}
        }
    }
    if objs.is_empty() {
        return Err(if parametriques > 0 { crate::trc("sc_parametrique").to_string() } else { crate::trc("sc_vide").to_string() });
    }
    Ok(objs)
}

/// Dossier du cache des pièces calculées par SolidCraft (les 20 plus récentes sont gardées).
fn dossier_cache() -> std::path::PathBuf {
    let base = if cfg!(windows) { std::env::temp_dir().join("3d-patch") } else {
        std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join(".cache").join("3d-patch")
    };
    base.join("solidcraft")
}

/// Secours pour une conception paramétrique : `solidcraft-cli run --in <conception> --out <3mf>` (corps séparés,
/// couleurs), lu comme un 3MF. Le calcul de SolidCraft peut être long (congés : plusieurs secondes chacun) :
/// le résultat est gardé sous l'empreinte du fichier, une réouverture du même fichier est immédiate.
/// None si SolidCraft n'est pas installé, échoue ou dépasse 5 minutes.
fn par_solidcraft(path: &str, empreinte: &str) -> Option<Objets> {
    let dossier = dossier_cache();
    let cache = dossier.join(format!("{empreinte}.3mf"));
    if let Ok(o) = crate::threemf::read_objets(cache.to_str()?) {
        if !o.is_empty() { return Some(o); }
    }
    let cli = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join(if cfg!(windows) { "solidcraft-cli.exe" } else { "solidcraft-cli" }))
        .find(|c| c.is_file())?;
    let _ = std::fs::create_dir_all(&dossier);
    let sortie = dossier.join(format!("{empreinte}.{}.part.3mf", std::process::id()));
    let mut fils = std::process::Command::new(cli).args(["run", "--in", path, "--out"]).arg(&sortie).arg("--quiet")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let debut = std::time::Instant::now();
    let fini = loop {
        match fils.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) if debut.elapsed().as_secs() < 300 => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => { let _ = fils.kill(); break false; }
        }
    };
    let objs = if fini { crate::threemf::read_objets(sortie.to_str()?).ok().filter(|o| !o.is_empty()) } else { None };
    if objs.is_some() && std::fs::rename(&sortie, &cache).is_ok() {
        // cache borné : les plus anciennes pièces calculées sont oubliées
        if let Ok(rd) = std::fs::read_dir(&dossier) {
            let mut f: Vec<(std::time::SystemTime, std::path::PathBuf)> = rd.flatten()
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
            f.sort();
            for (_, p) in f.iter().rev().skip(20) { let _ = std::fs::remove_file(p); }
        }
    } else { let _ = std::fs::remove_file(&sortie); }
    objs
}

/// Écrit les objets dans une conception SolidCraft : un composant et un maillage importé par objet
/// (sommets fusionnés, couleur moyenne s'il en a), blobs SCMESH01 nommés par leur empreinte.
pub fn ecrire(path: &std::path::Path, objs: &[(&[f32], Option<&[f32]>)]) -> std::io::Result<()> {
    let nom_doc = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "3D-Patch".into());
    let (mut composants, mut occurrences, mut features, mut blobs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut id = 1u64;
    for (k, (v, cols)) in objs.iter().enumerate() {
        if v.len() < 18 { continue; }
        // fusion des sommets identiques : maillage indexé, sans perte de précision
        let mut vus: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
        let (mut pos, mut idx) = (Vec::new(), Vec::with_capacity(v.len() / 6));
        for c in v.chunks_exact(6) {
            let cle = [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()];
            let i = *vus.entry(cle).or_insert_with(|| { pos.extend_from_slice(&[c[0] as f64, c[1] as f64, c[2] as f64]); (pos.len() / 3 - 1) as u32 });
            idx.push(i);
        }
        let b = encoder_maillage(&pos, &idx);
        let blob = format!("blobs/{}.mesh", hex(&Sha256::digest(&b)));
        let nom = if objs.len() == 1 { nom_doc.clone() } else { format!("{nom_doc} {}", k + 1) };
        let mut maillage = json!({ "name": nom, "mesh_blob": blob, "positions": [], "triangles": [] });
        if let Some(c) = cols.filter(|c| c.len() * 2 == v.len() && !c.is_empty()) {
            let n = (c.len() / 3) as f32;
            let s = c.chunks_exact(3).fold([0f32; 3], |a, x| [a[0] + x[0], a[1] + x[1], a[2] + x[2]]);
            maillage["color"] = json!([(s[0] / n).clamp(0.0, 1.0), (s[1] / n).clamp(0.0, 1.0), (s[2] / n).clamp(0.0, 1.0)]);
        }
        let (c_id, o_id, f_id) = (id, id + 1, id + 2);
        id += 3;
        composants.push(json!({ "id": c_id, "name": nom, "parent": 0 }));
        occurrences.push(json!({ "component": c_id, "grounded": false, "id": o_id, "name": format!("{nom}:1"), "parent": 0, "transform": IDENTITE }));
        features.push(json!({ "body_names": [nom], "component": c_id, "file": format!("{nom}.stl"), "id": f_id, "meshes": [maillage],
                              "name": nom, "suppressed": false, "type": "mesh_import" }));
        if !blobs.iter().any(|(n, _): &(String, Vec<u8>)| *n == blob) { blobs.push((blob, b)); }
    }
    let doc = json!({ "components": composants, "features": features, "format": FORMAT_DOC, "name": nom_doc, "next_id": id,
                      "occurrences": occurrences, "params": [], "units": "mm" });
    let err = |e: zip::result::ZipError| std::io::Error::new(std::io::ErrorKind::Other, e.to_string());
    // fichier temporaire puis renommage : une conception existante n'est jamais laissée à moitié écrite
    let tmp = path.with_extension("solidcraft.part");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&tmp)?);
        let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let brut = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(false);
        for (nom, b) in &blobs { z.start_file(nom.as_str(), brut).map_err(err)?; z.write_all(b)?; }
        z.start_file("document.json", deflate).map_err(err)?;
        z.write_all(&serde_json::to_vec(&doc).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?)?;
        z.start_file("container.json", deflate).map_err(err)?;
        z.write_all(json!({ "format": FORMAT_CONTENEUR }).to_string().as_bytes())?;
        z.finish().map_err(err)?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(o: [f32; 3], c: f32) -> Vec<f32> {
        let v = |x: f32, y: f32, z: f32| [o[0] + x * c, o[1] + y * c, o[2] + z * c];
        let q = [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.), v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)];
        let f = [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
        f.iter().flat_map(|t| t.iter().flat_map(|&i| [q[i][0], q[i][1], q[i][2], 0.0, 0.0, 0.0])).collect()
    }

    #[test]
    fn aller_retour() {
        // deux objets (dont un coloré) : écrits puis relus à l'identique, sommets fusionnés dans le blob
        let (a, b) = (cube([0.0, 0.0, 0.0], 10.0), cube([20.0, 0.5, 0.25], 5.0));
        let cb: Vec<f32> = [0.2f32, 0.4, 0.6].repeat(b.len() / 6);
        let chemin = std::env::temp_dir().join("3d-patch-test.solidcraft");
        ecrire(&chemin, &[(&a, None), (&b, Some(&cb))]).unwrap();
        let objs = lire(chemin.to_str().unwrap()).unwrap();
        assert_eq!(objs.len(), 2);
        for ((v, _), src) in objs.iter().zip([&a, &b]) {
            assert_eq!(v.len(), src.len());
            for (p, q) in v.chunks_exact(6).zip(src.chunks_exact(6)) { assert_eq!(&p[..3], &q[..3]); }
        }
        assert!(objs[0].1.is_none());
        let c = objs[1].1.as_ref().unwrap();
        assert!((c[0] - 0.2).abs() < 1e-6 && (c[2] - 0.6).abs() < 1e-6);
        // blob indexé : 8 sommets par cube, pas 36
        let mut z = zip::ZipArchive::new(std::fs::File::open(&chemin).unwrap()).unwrap();
        let nom = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).find(|n| n.ends_with(".mesh")).unwrap();
        let mut blob = Vec::new();
        z.by_name(&nom).unwrap().read_to_end(&mut blob).unwrap();
        assert_eq!(u64::from_le_bytes(blob[8..16].try_into().unwrap()), 24);
    }

    #[test]
    fn occurrence_placee() {
        // document JSON seul (tout en ligne) : maillage d'un composant placé par son occurrence (translation
        // dans m[3]) ; une fonction supprimée est ignorée
        let doc = json!({ "format": "solidcraft/26", "units": "mm",
            "components": [{ "id": 1, "name": "P", "parent": 0 }],
            "occurrences": [{ "id": 2, "component": 1, "parent": 0, "name": "P:1",
                              "transform": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [5.0, 6.0, 7.0, 1.0]] }],
            "features": [
                { "id": 3, "name": "M", "type": "mesh_import", "component": 1, "file": "m.stl",
                  "meshes": [{ "name": "M", "positions": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], "triangles": [0, 1, 2] }] },
                { "id": 4, "name": "X", "type": "mesh_import", "suppressed": true, "file": "x.stl",
                  "meshes": [{ "name": "X", "positions": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], "triangles": [0, 1, 2] }] }] });
        let chemin = std::env::temp_dir().join("3d-patch-test-json.solidcraft");
        std::fs::write(&chemin, doc.to_string()).unwrap();
        let objs = lire(chemin.to_str().unwrap()).unwrap();
        assert_eq!(objs.len(), 1);
        assert_eq!(&objs[0].0[..3], &[5.0, 6.0, 7.0]);
        assert_eq!(&objs[0].0[6..9], &[6.0, 6.0, 7.0]);
        assert_eq!(&objs[0].0[3..6], &[0.0, 0.0, 1.0]); // normale plate recalculée
    }

    #[test]
    fn apercu_enregistre() {
        // pièce paramétrique (une boîte) enregistrée avec ses maillages d'affichage : ceux-ci sont lus
        let tri = encoder_maillage(&[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0], &[0, 1, 2]);
        let man = json!({ "format": "solidcraft-preview/1", "meshes": [{ "name": "Body1", "color": [1.0, 0.0, 0.0], "offset": 0, "length": tri.len() }] });
        let doc = json!({ "format": "solidcraft/26", "units": "mm", "features": [{ "id": 1, "name": "Box1", "type": "box", "length": "2", "width": "3", "height": "1" }] });
        let chemin = std::env::temp_dir().join("3d-patch-test-apercu.solidcraft");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&chemin).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in [("container.json", json!({ "format": FORMAT_CONTENEUR }).to_string().into_bytes()), ("document.json", doc.to_string().into_bytes()),
                       ("cache/preview.json", man.to_string().into_bytes()), ("cache/preview.bin", tri)] {
            z.start_file(n, o).unwrap(); z.write_all(&b).unwrap();
        }
        z.finish().unwrap();
        let objs = lire(chemin.to_str().unwrap()).unwrap();
        assert_eq!(objs.len(), 1);
        assert_eq!(&objs[0].0[6..9], &[2.0, 0.0, 0.0]);
        assert_eq!(&objs[0].1.as_ref().unwrap()[..3], &[1.0, 0.0, 0.0]);
    }
}
