//! PRÉPARATION À L'IMPRESSION : vérifications avant impression (étanchéité, pose, dimensions,
//! surplombs à soutenir, parois trop fines, matière),
//! orientation conseillée, trancheurs installés et envoi direct du fichier.

use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Matières : nom et masse volumique (g/cm³).
pub const MATIERES: &[(&str, f32)] = &[("PLA", 1.24), ("PETG", 1.27), ("ABS", 1.04), ("ASA", 1.07), ("TPU", 1.21), ("Nylon (PA)", 1.14), ("PC", 1.20)];

// ---------------------------------------------------------------- analyse

/// Résultat des vérifications avant impression.
#[derive(Default, Clone)]
pub struct Analyse {
    pub trous: usize, pub erreurs: usize,
    pub zmin: f32,
    pub taille: Vec3,
    pub surplombs: Vec<[Vec3; 3]>, pub aire_surplombs: f32,
    pub fines: Vec<[Vec3; 3]>, pub aire_fines: f32,
    pub aire: f32, pub volume: f32,
}

fn aire_tri(t: &[Vec3; 3]) -> f32 { (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5 }

/// Grille de recherche des triangles (cases cubiques).
struct Grille { pas: f32, cases: HashMap<(i32, i32, i32), Vec<u32>> }
impl Grille {
    fn new(tris: &[[Vec3; 3]], pas: f32) -> Grille {
        let mut cases: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        let k = |x: f32| (x / pas).floor() as i32;
        for (i, t) in tris.iter().enumerate() {
            let (lo, hi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
            for x in k(lo.x)..=k(hi.x) { for y in k(lo.y)..=k(hi.y) { for z in k(lo.z)..=k(hi.z) { cases.entry((x, y, z)).or_default().push(i as u32); } } }
        }
        Grille { pas, cases }
    }
}

fn rayon_tri(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 { return None; }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(-1e-6..=1.0 + 1e-6).contains(&u) { return None; }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-6 || u + v > 1.0 + 1e-6 { return None; }
    let t_ = e2.dot(q) * inv;
    if t_ > 0.0 { Some(t_) } else { None }
}

/// Vérifications : `tris` = triangles de toutes les pièces à imprimer (coordonnées du monde),
/// `angle` = angle maximal imprimable sans support (depuis la verticale, en degrés),
/// `paroi_min` = épaisseur minimale (mm).
pub fn analyser(tris: &[[Vec3; 3]], soupe: &[f32], angle: f32, paroi_min: f32) -> Analyse {
    let mut a = Analyse::default();
    if tris.is_empty() { return a; }
    let d = crate::defects::analyze(soupe);
    a.trous = d.holes; a.erreurs = d.face_errors;
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for t in tris { for p in t { lo = lo.min(*p); hi = hi.max(*p); } }
    a.zmin = lo.z; a.taille = hi - lo;
    // surplombs : face tournée vers le bas au-delà de l'angle (sauf celles posées sur le plateau)
    let seuil = angle.to_radians().sin();
    let mut vol = 0.0f64;
    let mut normales = Vec::with_capacity(tris.len());
    for t in tris {
        let c = (t[1] - t[0]).cross(t[2] - t[0]);
        let ar = c.length() * 0.5;
        a.aire += ar;
        vol += t[0].as_dvec3().dot(t[1].as_dvec3().cross(t[2].as_dvec3())) / 6.0;
        let n = c.normalize_or_zero();
        normales.push(n);
        let bas = t[0].z.min(t[1].z).min(t[2].z);
        if -n.z > seuil && bas > lo.z + 0.2 { a.surplombs.push(*t); a.aire_surplombs += ar; }
    }
    a.volume = vol.abs() as f32;
    // parois fines : depuis chaque face, vers l'intérieur, une autre face (tournée à l'opposé) est
    // rencontrée avant `paroi_min`
    if paroi_min > 0.0 {
        let pas = (paroi_min * 2.0).max(a.taille.max_element() / 200.0).max(0.05);
        let g = Grille::new(tris, pas);
        let k = |x: f32| (x / pas).floor() as i32;
        let mut vus: Vec<u32> = Vec::new();
        for (i, t) in tris.iter().enumerate() {
            let n = normales[i];
            if n == Vec3::ZERO { continue; }
            let c = (t[0] + t[1] + t[2]) / 3.0;
            let o = c - n * 1e-4;
            let fin_ = o - n * paroi_min;
            vus.clear();
            let (l0, l1) = (o.min(fin_), o.max(fin_));
            let mut fine = false;
            'cases: for x in k(l0.x)..=k(l1.x) { for y in k(l0.y)..=k(l1.y) { for z in k(l0.z)..=k(l1.z) {
                let Some(l) = g.cases.get(&(x, y, z)) else { continue };
                for &j in l {
                    if j as usize == i || vus.contains(&j) { continue; }
                    vus.push(j);
                    if normales[j as usize].dot(n) > -0.2 { continue; } // face opposée seulement
                    if let Some(dd) = rayon_tri(o, -n, &tris[j as usize]) { if dd > 1e-3 && dd < paroi_min { fine = true; break 'cases; } }
                }
            }}}
            if fine { a.fines.push(*t); a.aire_fines += aire_tri(t); }
        }
    }
    a
}

/// Orientation conseillée : parmi les faces de pose possibles, celle qui laisse le moins de
/// surplombs à soutenir. Renvoie (normale extérieure de la face à poser en bas, aire de surplombs).
pub fn orientation(soupe: &[f32], angle: f32) -> Option<(Vec3, f32, f32)> {
    let tris: Vec<[Vec3; 3]> = soupe.chunks_exact(18).map(|t| [Vec3::new(t[0], t[1], t[2]), Vec3::new(t[6], t[7], t[8]), Vec3::new(t[12], t[13], t[14])]).collect();
    let seuil = angle.to_radians().sin();
    let cout = |q: Quat| -> f32 {
        let mut zmin = f32::INFINITY;
        for t in &tris { for p in t { zmin = zmin.min((q * *p).z); } }
        let mut s = 0.0;
        for t in &tris {
            let r = [q * t[0], q * t[1], q * t[2]];
            let c = (r[1] - r[0]).cross(r[2] - r[0]);
            let n = c.normalize_or_zero();
            if -n.z > seuil && r[0].z.min(r[1].z).min(r[2].z) > zmin + 0.2 { s += c.length() * 0.5; }
        }
        s
    };
    let actuel = cout(Quat::IDENTITY);
    let mut faces: Vec<Vec3> = crate::gravite::faces_de_pose(soupe).iter().map(|f| f.n).collect();
    for n in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] { if !faces.iter().any(|f| f.dot(n) > 0.999) { faces.push(n); } }
    let mut best: Option<(Vec3, f32)> = None;
    for n in faces {
        let q = Quat::from_rotation_arc(n, -Vec3::Z);
        let c = cout(q);
        if best.map_or(true, |b| c < b.1 - 1e-3) { best = Some((n, c)); }
    }
    best.map(|(n, c)| (n, c, actuel))
}

// ---------------------------------------------------------------- trancheurs

/// Comment lancer un trancheur.
#[derive(Clone, Debug, PartialEq)]
pub enum Lancement { Flatpak(String), Programme(PathBuf) }

/// Trancheur connu : nom, identifiants Flatpak, noms de programme (Linux), chemins Windows.
pub struct Trancheur { pub nom: &'static str, flatpak: &'static [&'static str], prog: &'static [&'static str], windows: &'static [&'static str] }

pub const TRANCHEURS: &[Trancheur] = &[
    Trancheur { nom: "Bambu Studio", flatpak: &["com.bambulab.BambuStudio"], prog: &["bambu-studio", "BambuStudio"], windows: &["Bambu Studio\\bambu-studio.exe"] },
    Trancheur { nom: "PrusaSlicer", flatpak: &["com.prusa3d.PrusaSlicer"], prog: &["prusa-slicer", "PrusaSlicer"], windows: &["Prusa3D\\PrusaSlicer\\prusa-slicer.exe"] },
    Trancheur { nom: "OrcaSlicer", flatpak: &["com.orcaslicer.OrcaSlicer", "io.github.softfever.OrcaSlicer"], prog: &["orca-slicer", "OrcaSlicer"], windows: &["OrcaSlicer\\orca-slicer.exe"] },
    Trancheur { nom: "UltiMaker Cura", flatpak: &["com.ultimaker.cura"], prog: &["cura", "UltiMaker-Cura"], windows: &["UltiMaker Cura*\\UltiMaker-Cura.exe", "Ultimaker Cura*\\Cura.exe"] },
    Trancheur { nom: "SuperSlicer", flatpak: &["com.superslicer.SuperSlicer"], prog: &["superslicer", "SuperSlicer"], windows: &["SuperSlicer\\superslicer.exe"] },
    Trancheur { nom: "Creality Print", flatpak: &[], prog: &["CrealityPrint", "creality-print"], windows: &["Creality Print*\\CrealityPrint.exe", "Creality\\Creality Print*\\CrealityPrint.exe"] },
    // ajouté en fin de liste : le choix mémorisé (numéro dans la liste) des autres ne change pas
    Trancheur { nom: "Simplify3D", flatpak: &[], prog: &["simplify3d", "Simplify3D"], windows: &["Simplify3D*\\Simplify3D.exe", "Simplify3D Software*\\Simplify3D.exe"] },
];

fn dans_path(nom: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for d in std::env::split_paths(&path) {
        let p = d.join(nom);
        if p.is_file() { return Some(p); }
    }
    None
}

/// Chemin Windows avec un éventuel joker « * » dans un dossier (versions de Cura…).
fn chemin_windows(motif: &str) -> Option<PathBuf> {
    for base in [std::env::var("ProgramFiles").ok(), std::env::var("ProgramFiles(x86)").ok(), std::env::var("LOCALAPPDATA").ok().map(|l| format!("{l}\\Programs"))].into_iter().flatten() {
        let mut cands = vec![PathBuf::from(&base)];
        for part in motif.split('\\') {
            let mut suiv = Vec::new();
            for c in &cands {
                if let Some(pref) = part.strip_suffix('*') {
                    if let Ok(rd) = std::fs::read_dir(c) {
                        for e in rd.flatten() { if e.file_name().to_string_lossy().to_lowercase().starts_with(&pref.to_lowercase()) { suiv.push(e.path()); } }
                    }
                } else { suiv.push(c.join(part)); }
            }
            cands = suiv;
        }
        if let Some(p) = cands.into_iter().filter(|p| p.is_file()).max() { return Some(p); }
    }
    None
}

/// AppImage d'un trancheur dans les dossiers habituels (Applications, Téléchargements, Bureau…).
fn appimage(nom: &str) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let cle = nom.to_lowercase().replace(' ', "");
    for d in ["Applications", "Téléchargements", "Downloads", "Bureau", "Desktop", "bin", ".local/bin"] {
        let Ok(rd) = std::fs::read_dir(home.join(d)) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase().replace([' ', '_', '-'], "");
            if n.ends_with(".appimage") && n.contains(&cle.replace('-', "")) { return Some(e.path()); }
        }
    }
    None
}

/// Trancheurs installés : pour chacun de `TRANCHEURS`, son lancement s'il est trouvé.
pub fn detecter() -> Vec<Option<Lancement>> {
    let flatpaks: String = if cfg!(windows) { String::new() } else {
        std::process::Command::new("flatpak").args(["list", "--app", "--columns=application"]).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    };
    TRANCHEURS.iter().map(|t| {
        if cfg!(windows) {
            return t.windows.iter().find_map(|m| chemin_windows(m)).map(Lancement::Programme);
        }
        t.prog.iter().find_map(|p| dans_path(p)).map(Lancement::Programme)
            .or_else(|| t.flatpak.iter().find(|id| flatpaks.lines().any(|l| l.trim() == **id)).map(|id| Lancement::Flatpak(id.to_string())))
            .or_else(|| appimage(t.nom).map(Lancement::Programme))
    }).collect()
}

/// Ouvre le fichier dans le trancheur (processus détaché : l'appli reste utilisable).
pub fn envoyer(l: &Lancement, fichier: &Path) -> Result<(), String> {
    let mut c = match l {
        // le portail de documents donne au trancheur en bac à sable l'accès au fichier
        Lancement::Flatpak(id) => { let mut c = std::process::Command::new("flatpak"); c.args(["run", "--file-forwarding", id, "@@"]).arg(fichier).arg("@@"); c }
        Lancement::Programme(p) => { let mut c = std::process::Command::new(p); c.arg(fichier); c }
    };
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Fichier d'échange envoyé au trancheur (dossier de l'utilisateur, accessible aux applis en bac à sable).
pub fn fichier_echange() -> PathBuf {
    let base = if cfg!(windows) { std::env::temp_dir() } else {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(".cache").join("3d-patch")
    };
    let _ = std::fs::create_dir_all(&base);
    base.join("3D-Patch_a_imprimer.3mf")
}
