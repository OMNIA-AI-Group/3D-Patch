//! Format Rhino 3DM (openNURBS). Disponible si la passerelle a été compilée (cfg has_3dm).

#[cfg(has_3dm)]
mod ffi {
    use std::os::raw::{c_char, c_int};
    extern "C" {
        pub fn on_import(path: *const c_char, verts: *mut *mut f32, nv: *mut usize, cols: *mut *mut f32, sans: *mut c_int, e: *mut c_char, en: c_int) -> c_int;
        pub fn on_free(p: *mut f32);
        pub fn on_export(path: *const c_char, verts: *const f32, ntri: usize, cols: *const f32, version: c_int, e: *mut c_char, en: c_int) -> c_int;
    }
}

#[cfg(has_3dm)]
pub fn import(path: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> {
    let cp = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
    let (mut v, mut nv, mut c, mut sans) = (std::ptr::null_mut(), 0usize, std::ptr::null_mut(), 0);
    let mut e = vec![0 as std::os::raw::c_char; 512];
    let rc = unsafe { ffi::on_import(cp.as_ptr(), &mut v, &mut nv, &mut c, &mut sans, e.as_mut_ptr(), 512) };
    if rc != 0 { return Err(unsafe { std::ffi::CStr::from_ptr(e.as_ptr()) }.to_string_lossy().into_owned()); }
    let verts = unsafe { std::slice::from_raw_parts(v, nv) }.to_vec();
    let cols = unsafe { std::slice::from_raw_parts(c, nv / 2) }.to_vec();
    unsafe { ffi::on_free(v); ffi::on_free(c); }
    if sans > 0 { eprintln!("3DM : {sans} objet(s) sans maillage de rendu ignoré(s)"); }
    if std::env::var_os("PATCH3D_COULEURS").is_some() {
        let mut h = std::collections::BTreeMap::new();
        for c in cols.chunks_exact(3) { *h.entry(format!("{:.2} {:.2} {:.2}", c[0], c[1], c[2])).or_insert(0usize) += 1; }
        eprintln!("couleurs : {h:?}");
    }
    Ok((verts, Some(cols)))
}
#[cfg(has_3dm)]
/// `version` : 5 (MoI, Rhino 5+) ou 8 (Rhino 8).
pub fn export(path: &std::path::Path, verts: &[f32], cols: Option<&[f32]>, version: i32) -> Result<(), String> {
    let cp = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let mut e = vec![0 as std::os::raw::c_char; 512];
    let rc = unsafe { ffi::on_export(cp.as_ptr(), verts.as_ptr(), verts.len() / 18, cols.map_or(std::ptr::null(), |c| c.as_ptr()), version, e.as_mut_ptr(), 512) };
    if rc != 0 { return Err(unsafe { std::ffi::CStr::from_ptr(e.as_ptr()) }.to_string_lossy().into_owned()); }
    Ok(())
}
#[cfg(not(has_3dm))]
pub fn import(_: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> { Err(format!("{} (3DM)", crate::trc("fmt_indispo"))) }
#[cfg(not(has_3dm))]
pub fn export(_: &std::path::Path, _: &[f32], _: Option<&[f32]>, _: i32) -> Result<(), String> { Err(format!("{} (3DM)", crate::trc("fmt_indispo"))) }
