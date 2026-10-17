//! Formats via Assimp : 3DS, DAE (Collada — export de SketchUp), FBX, glTF/GLB, X3D…
//! Disponible si la passerelle a été compilée (cfg has_assimp), sinon message d'indisponibilité.

#[cfg(has_assimp)]
mod ffi {
    use std::os::raw::{c_char, c_int};
    extern "C" {
        pub fn as_import(path: *const c_char, verts: *mut *mut f32, nv: *mut usize, cols: *mut *mut f32, has_col: *mut c_int, e: *mut c_char, en: c_int) -> c_int;
        pub fn as_free(p: *mut f32);
        pub fn as_export(path: *const c_char, format: *const c_char, verts: *const f32, ntri: usize, cols: *const f32, e: *mut c_char, en: c_int) -> c_int;
    }
}

/// Extensions importées par Assimp (en plus des formats natifs de l'appli).
pub const EXT_IMPORT: &[&str] = &["3ds", "dae", "fbx", "gltf", "glb", "x3d", "lwo", "ac", "ms3d", "off", "amf"];
/// Extension -> identifiant d'export Assimp.
pub fn export_id(ext: &str) -> Option<&'static str> {
    Some(match ext { "3ds" => "3ds", "dae" => "collada", "fbx" => "fbx", "gltf" => "gltf2", "glb" => "glb2", "x3d" => "x3d", _ => return None })
}

#[cfg(has_assimp)]
pub fn import(path: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> {
    let cp = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
    let (mut v, mut nv, mut c, mut hc) = (std::ptr::null_mut(), 0usize, std::ptr::null_mut(), 0);
    let mut e = vec![0 as std::os::raw::c_char; 512];
    let rc = unsafe { ffi::as_import(cp.as_ptr(), &mut v, &mut nv, &mut c, &mut hc, e.as_mut_ptr(), 512) };
    if rc != 0 { return Err(unsafe { std::ffi::CStr::from_ptr(e.as_ptr()) }.to_string_lossy().into_owned()); }
    let verts = unsafe { std::slice::from_raw_parts(v, nv) }.to_vec();
    let cols = unsafe { std::slice::from_raw_parts(c, nv / 2) }.to_vec();
    unsafe { ffi::as_free(v); ffi::as_free(c); }
    Ok((verts, if hc != 0 { Some(cols) } else { None }))
}
#[cfg(has_assimp)]
pub fn export(path: &std::path::Path, ext: &str, verts: &[f32], cols: Option<&[f32]>) -> Result<(), String> {
    let id = export_id(ext).ok_or("format d'export inconnu")?;
    let cp = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let cf = std::ffi::CString::new(id).unwrap();
    let mut e = vec![0 as std::os::raw::c_char; 512];
    let rc = unsafe { ffi::as_export(cp.as_ptr(), cf.as_ptr(), verts.as_ptr(), verts.len() / 18, cols.map_or(std::ptr::null(), |c| c.as_ptr()), e.as_mut_ptr(), 512) };
    if rc != 0 { return Err(unsafe { std::ffi::CStr::from_ptr(e.as_ptr()) }.to_string_lossy().into_owned()); }
    Ok(())
}
#[cfg(not(has_assimp))]
pub fn import(_: &str) -> Result<(Vec<f32>, Option<Vec<f32>>), String> { Err(crate::trc("fmt_indispo").into()) }
#[cfg(not(has_assimp))]
pub fn export(_: &std::path::Path, _: &str, _: &[f32], _: Option<&[f32]>) -> Result<(), String> { Err(crate::trc("fmt_indispo").into()) }
