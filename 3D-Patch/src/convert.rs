//! Import des formats d'applications tierces (Blender, FreeCAD, Fusion 360, AutoCAD).
//! On s'appuie sur ce qui est réellement lisible hors-ligne :
//!  - .blend  : export STL via le Blender installé (ligne de commande),
//!  - .FCStd  : archive ZIP contenant des BREP OpenCASCADE -> lecture directe,
//!  - .dxf    : parseur minimal des faces 3DFACE (maillage ASCII),
//!  - .f3d / .dwg : formats propriétaires -> on tente d'extraire un format neutre
//!                  embarqué (STEP/IGES/BREP/STL), sinon message explicite.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::stl::{self, Mesh};
use crate::occt;

/// Dossier temporaire unique pour cette conversion.
fn tmp_dir(tag: &str) -> PathBuf {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let d = std::env::temp_dir().join(format!("3dpatch_{tag}_{t}"));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Fusionne plusieurs maillages en un seul (concatène les sommets, recalcule la boîte).
fn merge(meshes: Vec<Mesh>) -> Option<Mesh> {
    let mut verts: Vec<f32> = Vec::new();
    for m in meshes { verts.extend_from_slice(&m.verts); }
    if verts.len() < 18 { return None; }
    use glam::Vec3;
    let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for c in verts.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); mn = mn.min(p); mx = mx.max(p); }
    Some(Mesh { verts, min: mn, max: mx })
}

/// Point d'entrée : convertit un fichier d'appli tierce en maillage.
pub fn import_external(path: &str, ext: &str, defl: f64) -> Result<Mesh, String> {
    match ext {
        "blend" => from_blender(path),
        "fcstd" | "fcstd1" => from_fcstd(path, defl),
        "dxf" => from_dxf(path),
        "f3d" | "f3z" | "dwg" => from_zip_neutral(path, defl, ext),
        _ => Err(format!("{} : {ext}", crate::trc("fmt_non_gere"))),
    }
}

/// .blend -> STL via le Blender installé, puis lecture du STL.
fn from_blender(path: &str) -> Result<Mesh, String> {
    if which("blender").is_none() {
        return Err(crate::trc("blender_absent").into());
    }
    let out = tmp_dir("blend").join("export.stl");
    let outs = out.to_string_lossy().replace('\\', "/");
    // 4.0 : addon STL -> export_mesh.stl ; 4.1+ : wm.stl_export. On tente les deux.
    let expr = format!(
        "import bpy\n\
         p=r'{outs}'\n\
         try:\n import bpy; bpy.ops.export_mesh.stl(filepath=p)\n\
         except Exception:\n bpy.ops.wm.stl_export(filepath=p)\n");
    let st = Command::new("blender")
        .args(["-b", path, "--python-expr", &expr])
        .output().map_err(|e| format!("{} : {e}", crate::trc("blender_introuv")))?;
    if !out.exists() {
        let err = String::from_utf8_lossy(&st.stderr);
        return Err(format!("{}{}", crate::trc("blender_echec"), if err.is_empty() { String::new() } else { format!(" : {}", err.lines().last().unwrap_or("")) }));
    }
    let m = stl::load_any(&outs).map_err(|e| format!("{} : {e}", crate::trc("blender_stl")))?;
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
    Ok(m)
}

/// .FCStd (ZIP FreeCAD) -> lecture de tous les BREP contenus.
fn from_fcstd(path: &str, defl: f64) -> Result<Mesh, String> {
    let dir = tmp_dir("fcstd");
    let st = Command::new("unzip").args(["-o", path, "-d"]).arg(&dir)
        .output().map_err(|e| format!("{} : {e}", crate::trc("unzip_absent")))?;
    if !st.status.success() {
        return Err(crate::trc("fcstd_illisible").into());
    }
    let mut meshes = Vec::new();
    collect_brep(&dir, defl, &mut meshes);
    let _ = std::fs::remove_dir_all(&dir);
    merge(meshes).ok_or_else(|| "aucune forme BREP lisible dans le .FCStd".into())
}

fn collect_brep(dir: &Path, defl: f64, out: &mut Vec<Mesh>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { collect_brep(&p, defl, out); continue; }
            let is_brep = p.extension().and_then(|x| x.to_str())
                .map(|x| { let x = x.to_ascii_lowercase(); x == "brp" || x == "brep" }).unwrap_or(false);
            if is_brep {
                if let Ok(m) = occt::import_brep(&p.to_string_lossy(), defl) { out.push(m); }
            }
        }
    }
}

/// .f3d / .dwg : on tente d'extraire un format neutre (STEP/IGES/BREP/STL) embarqué.
fn from_zip_neutral(path: &str, defl: f64, ext: &str) -> Result<Mesh, String> {
    let dir = tmp_dir("zipn");
    let st = Command::new("unzip").args(["-o", path, "-d"]).arg(&dir).output();
    let ok = st.map(|s| s.status.success()).unwrap_or(false);
    if ok {
        let mut meshes = Vec::new();
        collect_neutral(&dir, defl, &mut meshes);
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(m) = merge(meshes) { return Ok(m); }
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Err(format!("« {} » : {}", ext.to_uppercase(), crate::trc("fmt_propri")))
}

fn collect_neutral(dir: &Path, defl: f64, out: &mut Vec<Mesh>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { collect_neutral(&p, defl, out); continue; }
            let ext = p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).unwrap_or_default();
            let ps = p.to_string_lossy().to_string();
            let m = match ext.as_str() {
                "step" | "stp" => occt::import_cad(&ps, false, defl).ok(),
                "iges" | "igs" => occt::import_cad(&ps, true, defl).ok(),
                "brp" | "brep" => occt::import_brep(&ps, defl).ok(),
                "stl" | "obj" | "ply" => stl::load_any(&ps).ok(),
                _ => None,
            };
            if let Some(m) = m { out.push(m); }
        }
    }
}

/// Parseur DXF minimal : récupère les entités 3DFACE (quadrilatères -> 2 triangles).
fn from_dxf(path: &str) -> Result<Mesh, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{} : {e}", crate::trc("dxf_illisible")))?;
    let lines: Vec<&str> = txt.lines().map(|l| l.trim()).collect();
    let mut verts: Vec<f32> = Vec::new();
    let mut i = 0usize;
    // Format DXF : paires (code, valeur) sur deux lignes successives.
    while i + 1 < lines.len() {
        if lines[i] == "0" && lines[i + 1].eq_ignore_ascii_case("3DFACE") {
            // Coins : codes 10/20/30, 11/21/31, 12/22/32, 13/23/33.
            let mut corners = [[0f32; 3]; 4];
            let mut j = i + 2;
            while j + 1 < lines.len() && lines[j] != "0" {
                if let Ok(code) = lines[j].parse::<i32>() {
                    if let Ok(v) = lines[j + 1].parse::<f32>() {
                        match code {
                            10..=13 => corners[(code - 10) as usize][0] = v,
                            20..=23 => corners[(code - 20) as usize][1] = v,
                            30..=33 => corners[(code - 30) as usize][2] = v,
                            _ => {}
                        }
                    }
                }
                j += 2;
            }
            // 2 triangles (0,1,2) et (0,2,3). Normale calculée pour chaque.
            push_tri(&mut verts, corners[0], corners[1], corners[2]);
            push_tri(&mut verts, corners[0], corners[2], corners[3]);
            i = j;
        } else { i += 1; }
    }
    if verts.len() < 18 {
        return Err(crate::trc("dxf_sans_face").into());
    }
    use glam::Vec3;
    let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for c in verts.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); mn = mn.min(p); mx = mx.max(p); }
    Ok(Mesh { verts, min: mn, max: mx })
}

fn push_tri(verts: &mut Vec<f32>, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
    use glam::Vec3;
    let (va, vb, vc) = (Vec3::from(a), Vec3::from(b), Vec3::from(c));
    if va.distance(vb) < 1e-6 || vb.distance(vc) < 1e-6 || va.distance(vc) < 1e-6 { return; }
    let n = (vb - va).cross(vc - va).normalize_or_zero();
    for v in [va, vb, vc] {
        verts.extend_from_slice(&[v.x, v.y, v.z, n.x, n.y, n.z]);
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for p in std::env::split_paths(&paths) {
        let f = p.join(bin);
        if f.is_file() { return Some(f); }
    }
    None
}
