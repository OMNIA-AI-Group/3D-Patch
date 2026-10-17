//! Pont OCCT -> Rust : crée une primitive via le noyau OCCT et récupère sa tessellation
//! (triangles + normales à plat) directement en mémoire, sans fichier intermédiaire.

use glam::Vec3;
use std::os::raw::c_int;

use crate::stl::Mesh;

extern "C" {
    fn occt_make_primitive(kind: c_int, a: f64, b: f64, c: f64, defl: f64,
                           out: *mut *mut f32, out_nfloats: *mut c_int) -> c_int;
    fn occt_import_cad(path: *const std::os::raw::c_char, is_iges: c_int, defl: f64,
                       out: *mut *mut f32, out_nfloats: *mut c_int) -> c_int;
    fn occt_import_brep(path: *const std::os::raw::c_char, defl: f64,
                        out: *mut *mut f32, out_nfloats: *mut c_int) -> c_int;
    fn occt_make_demo(dx: f64, dy: f64, dz: f64, r: f64, defl: f64,
                      out: *mut *mut f32, out_nfloats: *mut c_int) -> c_int;
    fn occt_free(p: *mut f32);
}

/// Objet de démonstration : boîte à congés (vitrine du noyau OCCT).
pub fn demo(dx: f64, dy: f64, dz: f64, r: f64, deflection: f64) -> Result<Mesh, i32> {
    let mut ptr: *mut f32 = std::ptr::null_mut();
    let mut n: c_int = 0;
    let rc = unsafe { occt_make_demo(dx, dy, dz, r, deflection, &mut ptr, &mut n) };
    if rc != 0 || ptr.is_null() { return Err(rc); }
    verts_to_mesh(unsafe {
        let slice = std::slice::from_raw_parts(ptr, n as usize);
        let v = slice.to_vec();
        occt_free(ptr);
        v
    })
}

/// Importe un fichier BREP OpenCASCADE (p.ex. extrait d'un .FCStd FreeCAD).
pub fn import_brep(path: &str, deflection: f64) -> Result<Mesh, i32> {
    let c = std::ffi::CString::new(path).map_err(|_| -1)?;
    let mut ptr: *mut f32 = std::ptr::null_mut();
    let mut n: c_int = 0;
    let rc = unsafe { occt_import_brep(c.as_ptr(), deflection, &mut ptr, &mut n) };
    if rc != 0 || ptr.is_null() { return Err(rc); }
    verts_to_mesh(unsafe {
        let slice = std::slice::from_raw_parts(ptr, n as usize);
        let v = slice.to_vec();
        occt_free(ptr);
        v
    })
}

/// Importe un fichier STEP ou IGES via OCCT et renvoie le maillage tessellé.
pub fn import_cad(path: &str, is_iges: bool, deflection: f64) -> Result<Mesh, i32> {
    let c = std::ffi::CString::new(path).map_err(|_| -1)?;
    let mut ptr: *mut f32 = std::ptr::null_mut();
    let mut n: c_int = 0;
    let rc = unsafe { occt_import_cad(c.as_ptr(), is_iges as c_int, deflection, &mut ptr, &mut n) };
    if rc != 0 || ptr.is_null() { return Err(rc); }
    verts_to_mesh(unsafe {
        let slice = std::slice::from_raw_parts(ptr, n as usize);
        let v = slice.to_vec();
        occt_free(ptr);
        v
    })
}

fn verts_to_mesh(verts: Vec<f32>) -> Result<Mesh, i32> {
    if verts.len() < 18 { return Err(44); }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for chunk in verts.chunks_exact(6) {
        let p = Vec3::new(chunk[0], chunk[1], chunk[2]);
        min = min.min(p); max = max.max(p);
    }
    Ok(Mesh { verts, min, max })
}

/// Flèche de tessellation (mm) : écart maximal entre la surface exacte et ses triangles.
/// 0,02 mm = sous la résolution d'une imprimante 3D (complétée par 6° de flèche angulaire).
pub const DEFLEXION: f64 = 0.02;

#[derive(Clone, Copy)]
pub enum Prim {
    Box { dx: f64, dy: f64, dz: f64 },
    Sphere { r: f64 },
    Cylinder { r: f64, h: f64 },
    Cone { r1: f64, r2: f64, h: f64 },
    Torus { big: f64, small: f64 },
}

impl Prim {
    fn args(self) -> (c_int, f64, f64, f64) {
        match self {
            Prim::Box { dx, dy, dz } => (0, dx, dy, dz),
            Prim::Sphere { r } => (1, r, 0.0, 0.0),
            Prim::Cylinder { r, h } => (2, r, h, 0.0),
            Prim::Cone { r1, r2, h } => (3, r1, r2, h),
            Prim::Torus { big, small } => (4, big, small, 0.0),
        }
    }
}

/// Construit la primitive dans OCCT et renvoie son maillage prêt pour le rendu.
pub fn primitive(p: Prim, deflection: f64) -> Result<Mesh, i32> {
    let (kind, a, b, c) = p.args();
    let mut ptr: *mut f32 = std::ptr::null_mut();
    let mut n: c_int = 0;
    let rc = unsafe { occt_make_primitive(kind, a, b, c, deflection, &mut ptr, &mut n) };
    if rc != 0 || ptr.is_null() {
        return Err(rc);
    }
    let verts = unsafe {
        let slice = std::slice::from_raw_parts(ptr, n as usize);
        let v = slice.to_vec();
        occt_free(ptr);
        v
    };
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for chunk in verts.chunks_exact(6) {
        let p = Vec3::new(chunk[0], chunk[1], chunk[2]);
        min = min.min(p);
        max = max.max(p);
    }
    Ok(Mesh { verts, min, max })
}
