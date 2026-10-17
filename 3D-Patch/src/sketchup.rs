//! Format SketchUp .skp (et .skb, sa sauvegarde automatique) via OpenSKP (licence MIT).
//! Disponible si la passerelle a été compilée (cfg has_skp).

#[cfg(has_skp)]
mod ffi {
    use std::os::raw::{c_char, c_int};
    extern "C" {
        pub fn skp_import(path: *const c_char, verts: *mut *mut f32, nv: *mut usize, cols: *mut *mut f32, has_col: *mut c_int, e: *mut c_char, en: c_int) -> c_int;
        pub fn skp_free(p: *mut f32);
        pub fn skp_export(path: *const c_char, verts: *const f32, ntri: usize, cols: *const f32, e: *mut c_char, en: c_int) -> c_int;
    }
}

#[cfg(has_skp)]
pub fn import(path: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> {
    let cp = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
    let (mut v, mut nv, mut c, mut hc) = (std::ptr::null_mut(), 0usize, std::ptr::null_mut(), 0);
    let mut e = vec![0 as std::os::raw::c_char; 512];
    let rc = unsafe { ffi::skp_import(cp.as_ptr(), &mut v, &mut nv, &mut c, &mut hc, e.as_mut_ptr(), 512) };
    if rc != 0 { return Err(unsafe { std::ffi::CStr::from_ptr(e.as_ptr()) }.to_string_lossy().into_owned()); }
    let verts = unsafe { std::slice::from_raw_parts(v, nv) }.to_vec();
    let cols = unsafe { std::slice::from_raw_parts(c, nv / 2) }.to_vec();
    unsafe { ffi::skp_free(v); ffi::skp_free(c); }
    Ok((verts, if hc != 0 { Some(cols) } else { None }))
}
#[cfg(has_skp)]
pub fn export(path: &std::path::Path, verts: &[f32], cols: Option<&[f32]>) -> Result<(), String> {
    let cp = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let mut e = vec![0 as std::os::raw::c_char; 512];
    let rc = unsafe { ffi::skp_export(cp.as_ptr(), verts.as_ptr(), verts.len() / 18, cols.map_or(std::ptr::null(), |c| c.as_ptr()), e.as_mut_ptr(), 512) };
    if rc != 0 { return Err(unsafe { std::ffi::CStr::from_ptr(e.as_ptr()) }.to_string_lossy().into_owned()); }
    Ok(())
}
#[cfg(not(has_skp))]
pub fn import(_: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> { Err(format!("{} (SketchUp)", crate::trc("fmt_indispo"))) }
#[cfg(not(has_skp))]
pub fn export(_: &std::path::Path, _: &[f32], _: Option<&[f32]>) -> Result<(), String> { Err(format!("{} (SketchUp)", crate::trc("fmt_indispo"))) }
