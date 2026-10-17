//! Dossiers de l'application : Bureau par défaut, mémoire du dernier fichier ouvert/sauvegardé.

use std::path::{Path, PathBuf};

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Bureau de l'utilisateur (XDG `DESKTOP` sous Linux, `%USERPROFILE%\Desktop` sous Windows).
pub fn desktop() -> PathBuf {
    let h = home().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(not(windows)) {
        // ~/.config/user-dirs.dirs : XDG_DESKTOP_DIR="$HOME/Bureau"
        if let Ok(t) = std::fs::read_to_string(h.join(".config/user-dirs.dirs")) {
            for l in t.lines() {
                if let Some(v) = l.strip_prefix("XDG_DESKTOP_DIR=") {
                    let p = PathBuf::from(v.trim_matches('"').replace("$HOME", &h.to_string_lossy()));
                    if p.is_dir() { return p; }
                }
            }
        }
    }
    for name in ["Desktop", "Bureau"] { let p = h.join(name); if p.is_dir() { return p; } }
    h
}

fn config_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))
    }?;
    Some(base.join("3d-patch").join("dernier_fichier"))
}

/// Dernier fichier ouvert ou sauvegardé (s'il existe encore, sinon son dossier).
pub fn last_file() -> Option<PathBuf> {
    let p = PathBuf::from(std::fs::read_to_string(config_file()?).ok()?.trim());
    if p.as_os_str().is_empty() { None } else { Some(p) }
}

pub fn remember(p: &Path) {
    if let Some(f) = config_file() {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(f, p.to_string_lossy().as_bytes());
    }
}

/// Réglage booléen mémorisé (fichier « <nom> » contenant 1/0 dans le dossier de config).
pub fn flag(name: &str) -> bool {
    config_file().and_then(|f| std::fs::read_to_string(f.with_file_name(name)).ok()).map_or(false, |t| t.trim() == "1")
}
pub fn set_flag(name: &str, on: bool) {
    if let Some(f) = config_file() {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(f.with_file_name(name), if on { "1" } else { "0" });
    }
}

/// Préférences de l'utilisateur (fichier « preferences », lignes `clé=valeur`).
pub fn prefs() -> std::collections::HashMap<String, String> {
    config_file().and_then(|f| std::fs::read_to_string(f.with_file_name("preferences")).ok()).unwrap_or_default()
        .lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}
pub fn save_prefs(text: &str) {
    if let Some(f) = config_file() {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        // écriture atomique : fichier temporaire puis renommage (pas de fichier tronqué si coupure)
        let tmp = f.with_file_name("preferences.tmp");
        if std::fs::write(&tmp, text).is_ok() { let _ = std::fs::rename(&tmp, f.with_file_name("preferences")); }
    }
}

/// Réglage texte mémorisé (fichier « <nom> » dans le dossier de config).
pub fn setting(name: &str) -> Option<String> {
    config_file().and_then(|f| std::fs::read_to_string(f.with_file_name(name)).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}
pub fn set_setting(name: &str, val: &str) {
    if let Some(f) = config_file() {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(f.with_file_name(name), val);
    }
}

/// Dossier où ouvrir le sélecteur : celui du dernier fichier, sinon le Bureau.
pub fn start_dir() -> PathBuf {
    last_file().and_then(|p| p.parent().map(Path::to_path_buf)).filter(|d| d.is_dir()).unwrap_or_else(desktop)
}
