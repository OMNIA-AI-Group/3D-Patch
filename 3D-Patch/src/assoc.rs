//! « 3D-Patch en visionneuse 3D » : 3D-Patch devient l'application par défaut des formats 3D
//! (double-clic sur un fichier => ouverture en mode visionneuse). Les associations précédentes sont
//! mémorisées pour être restaurées quand l'option est décochée.

use std::path::PathBuf;
use std::process::Command;

/// Types MIME des formats 3D (Linux). PLY volontairement absent : souvent détecté comme text/plain.
#[cfg(not(windows))]
const MIMES: &[&str] = &[
    "model/stl", "model/x.stl-ascii", "model/x.stl-binary", "application/sla",
    "model/obj", "model/step", "application/x-wine-extension-step", "application/x-wine-extension-stp",
    "model/iges", "image/vnd.dxf", "application/x-blender", "application/x-extension-fcstd",
    "model/3mf", "application/vnd.ms-package.3dmanufacturing-3dmodel+xml",
    "application/x-3ds", "image/x-3ds", "model/vnd.3dm", "application/x-3dm", "application/x-wine-extension-3dm", "application/vnd.sketchup.skp", "model/vnd.collada+xml", "model/gltf-binary", "model/gltf+json",
];
/// Extensions (Windows).
#[cfg(windows)]
const EXTS: &[&str] = &["stl", "obj", "ply", "3mf", "3dm", "skp", "3ds", "dae", "fbx", "glb", "gltf", "step", "stp", "iges", "igs", "dxf", "blend", "fcstd"];

fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) { std::env::var_os("APPDATA").map(PathBuf::from) }
               else { std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))) }?;
    Some(base.join("3d-patch"))
}
fn backup_file() -> Option<PathBuf> { config_dir().map(|d| d.join("associations_precedentes")) }

/// Exécutable à associer : l'AppImage elle-même si on tourne depuis une AppImage.
fn exe() -> String {
    std::env::var("APPIMAGE").ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
}

// ------------------------------------------------------------------ Linux (xdg)
#[cfg(not(windows))]
const DESKTOP: &str = "3d-patch.desktop";

#[cfg(not(windows))]
fn query_default(mime: &str) -> String {
    Command::new("xdg-mime").args(["query", "default", mime]).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// L'option est-elle active ? = choix EXPLICITE « model/stl=3d-patch.desktop » dans mimeapps.list
/// (pas le choix implicite du système, qui ne dépend que de l'ordre des applications installées).
#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(&home).join(".config"));
    let t = std::fs::read_to_string(cfg.join("mimeapps.list")).unwrap_or_default();
    let mut section = "";
    for l in t.lines() {
        if l.starts_with('[') { section = l; continue; }
        if section == "[Default Applications]" && l.trim() == format!("model/stl={DESKTOP}") { return true; }
    }
    false
}

/// Entrée de menu + icône. `formats` : déclarer (ou non) les types 3D. Sans déclaration, le système
/// ne choisit jamais 3D-Patch tout seul pour ces fichiers (« Ouvrir avec » reste possible à la main).
#[cfg(not(windows))]
fn ensure_desktop_entry(formats: bool) -> Result<(), String> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME absent")?);
    let apps = home.join(".local/share/applications");
    let icons = home.join(".local/share/icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&icons).map_err(|e| e.to_string())?;
    let icon = icons.join("3d-patch.png");
    if let Ok(img) = image::load_from_memory(include_bytes!("../assets/logo.png")) {
        let _ = img.resize(256, 256, image::imageops::FilterType::Lanczos3).save(&icon);
    }
    let txt = format!("[Desktop Entry]\nType=Application\nName=3D-Patch\nComment=Modeleur et visionneuse 3D — cœur OpenCASCADE\n\
Exec=\"{}\" %f\nIcon={}\nTerminal=false\nCategories=Graphics;3DGraphics;Engineering;\nStartupNotify=true\n{}",
        exe(), icon.display(), if formats { format!("MimeType={};\n", MIMES.join(";")) } else { String::new() });
    std::fs::write(apps.join(DESKTOP), txt).map_err(|e| e.to_string())?;
    // SketchUp n'a pas de type MIME sur Linux : on le déclare (*.skp, *.skb) pour pouvoir l'associer
    let mime = home.join(".local/share/mime");
    let pk = mime.join("packages/3d-patch-sketchup.xml");
    if !pk.exists() && std::fs::create_dir_all(mime.join("packages")).is_ok() {
        let _ = std::fs::write(&pk, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n  \
<mime-type type=\"application/vnd.sketchup.skp\">\n    <comment>Modèle SketchUp</comment>\n    <glob pattern=\"*.skp\"/>\n    <glob pattern=\"*.skb\"/>\n  </mime-type>\n</mime-info>\n");
        let _ = Command::new("update-mime-database").arg(&mime).status();
    }
    let _ = Command::new("update-desktop-database").arg(&apps).status();
    Ok(())
}

#[cfg(not(windows))]
pub fn enable() -> Result<String, String> {
    // relève les associations AVANT de déclarer les formats (sinon le système nous renverrait nous-mêmes)
    ensure_desktop_entry(false)?;
    let mut sauve = Vec::new();
    for m in MIMES {
        let d = query_default(m);
        if d != DESKTOP { sauve.push(format!("{m}={d}")); }
    }
    if let (Some(f), false) = (backup_file(), sauve.is_empty()) {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(f, sauve.join("\n"));
    }
    ensure_desktop_entry(true)?;
    let mut ok = 0;
    for m in MIMES {
        if Command::new("xdg-mime").args(["default", DESKTOP, m]).status().map_or(false, |s| s.success()) { ok += 1; }
    }
    Ok(format!("{} ({ok} {})", crate::trc("assoc_ok"), crate::trc("types")))
}

#[cfg(not(windows))]
pub fn disable() -> Result<String, String> {
    let prec: Vec<(String, String)> = backup_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default()
        .lines().filter_map(|l| l.split_once('=').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
    let mut rest = 0;
    for m in MIMES {
        match prec.iter().find(|(k, _)| k == m).map(|(_, v)| v.clone()).filter(|v| !v.is_empty()) {
            Some(app) => { if Command::new("xdg-mime").args(["default", &app, m]).status().map_or(false, |s| s.success()) { rest += 1; } }
            None => {} // pas d'application avant : on retire seulement la ligne 3D-Patch ci-dessous
        }
    }
    // retire les choix « 3D-Patch par défaut » restants (types qui n'avaient pas d'application avant)
    if let Some(home) = std::env::var_os("HOME") {
        let f = PathBuf::from(home).join(".config/mimeapps.list");
        if let Ok(t) = std::fs::read_to_string(&f) {
            let mut section = String::new();
            let garde: Vec<&str> = t.lines().filter(|l| {
                if l.starts_with('[') { section = l.to_string(); return true; }
                !(section == "[Default Applications]" && l.ends_with(&format!("={DESKTOP}")))
            }).collect();
            let _ = std::fs::write(&f, garde.join("\n") + "\n");
        }
    }
    // plus de déclaration des formats : le système revient de lui-même aux autres applications
    ensure_desktop_entry(false)?;
    Ok(format!("{} ({rest} {})", crate::trc("assoc_rest"), crate::trc("types")))
}

// ------------------------------------------------------------------ Windows (registre HKCU)
#[cfg(windows)]
const PROGID: &str = "3DPatch.Modele3D";
#[cfg(windows)]
fn reg(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("reg").args(args).creation_flags(0x0800_0000).status().map_or(false, |s| s.success())
}
#[cfg(windows)]
fn reg_query(key: &str) -> String {
    use std::os::windows::process::CommandExt;
    Command::new("reg").args(["query", key, "/ve"]).creation_flags(0x0800_0000).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().find(|l| l.contains("REG_SZ")).and_then(|l| l.split("REG_SZ").nth(1)).unwrap_or("").trim().to_string())
        .unwrap_or_default()
}
#[cfg(windows)]
pub fn is_enabled() -> bool { reg_query(r"HKCU\Software\Classes\.stl") == PROGID }
#[cfg(windows)]
pub fn enable() -> Result<String, String> {
    let cmd = format!("\"{}\" \"%1\"", exe());
    let base = format!(r"HKCU\Software\Classes\{PROGID}");
    reg(&["add", &base, "/ve", "/d", "Modèle 3D (3D-Patch)", "/f"]);
    reg(&["add", &format!(r"{base}\DefaultIcon"), "/ve", "/d", &format!("\"{}\",0", exe()), "/f"]);
    reg(&["add", &format!(r"{base}\shell\open\command"), "/ve", "/d", &cmd, "/f"]);
    let mut sauve = Vec::new();
    for e in EXTS {
        let k = format!(r"HKCU\Software\Classes\.{e}");
        let avant = reg_query(&k);
        if avant != PROGID { sauve.push(format!("{e}={avant}")); }
        reg(&["add", &k, "/ve", "/d", PROGID, "/f"]);
        reg(&["add", &format!(r"{k}\OpenWithProgids"), "/v", PROGID, "/t", "REG_NONE", "/f"]);
    }
    if let (Some(f), false) = (backup_file(), sauve.is_empty()) {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(f, sauve.join("\n"));
    }
    Ok(crate::trc("assoc_ok_win").into())
}
#[cfg(windows)]
pub fn disable() -> Result<String, String> {
    let prec: Vec<(String, String)> = backup_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default()
        .lines().filter_map(|l| l.split_once('=').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
    for e in EXTS {
        let k = format!(r"HKCU\Software\Classes\.{e}");
        match prec.iter().find(|(x, _)| x == e).map(|(_, v)| v.clone()).filter(|v| !v.is_empty()) {
            Some(v) => { reg(&["add", &k, "/ve", "/d", &v, "/f"]); }
            None => { reg(&["delete", &k, "/ve", "/f"]); }
        }
    }
    Ok(crate::trc("assoc_rest").into())
}
