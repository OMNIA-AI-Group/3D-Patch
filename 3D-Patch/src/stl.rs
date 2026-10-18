//! Lecture d'un STL (ASCII ou binaire) → sommets + normales par facette (ombrage plat, net, façon CAO).

use glam::Vec3;

pub struct Mesh {
    /// Triplets (position, normale) entrelacés, 6 f32 par sommet, 3 sommets par triangle.
    pub verts: Vec<f32>,
    pub min: Vec3,
    pub max: Vec3,
}

impl Mesh {
    pub fn center(&self) -> Vec3 { (self.min + self.max) * 0.5 }
    pub fn radius(&self) -> f32 { ((self.max - self.min) * 0.5).length().max(1e-3) }
}

pub fn load(path: &str) -> std::io::Result<Mesh> {
    let bytes = std::fs::read(path)?;
    let is_ascii = bytes.len() >= 5 && &bytes[..5] == b"solid"
        && bytes.iter().take(512).all(|&b| b == b'\n' || b == b'\r' || b == b'\t' || (0x20..=0x7e).contains(&b));
    let tris = if is_ascii { parse_ascii(&bytes) } else { parse_binary(&bytes) };

    let mut verts = Vec::with_capacity(tris.len() * 18);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (n, p) in &tris {
        for v in p {
            verts.extend_from_slice(&[v.x, v.y, v.z, n.x, n.y, n.z]);
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    Ok(Mesh { verts, min, max })
}

fn parse_ascii(bytes: &[u8]) -> Vec<(Vec3, [Vec3; 3])> {
    let text = String::from_utf8_lossy(bytes);
    let mut tris = Vec::new();
    let mut normal = Vec3::ZERO;
    let mut pts: Vec<Vec3> = Vec::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.as_slice() {
            ["facet", "normal", a, b, c] => {
                normal = Vec3::new(a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0), c.parse().unwrap_or(0.0));
                pts.clear();
            }
            ["vertex", a, b, c] => {
                pts.push(Vec3::new(a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0), c.parse().unwrap_or(0.0)));
            }
            ["endfacet"] if pts.len() == 3 => {
                let n = fix_normal(normal, &pts);
                tris.push((n, [pts[0], pts[1], pts[2]]));
            }
            _ => {}
        }
    }
    tris
}

fn parse_binary(bytes: &[u8]) -> Vec<(Vec3, [Vec3; 3])> {
    let mut tris = Vec::new();
    if bytes.len() < 84 { return tris; }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let mut off = 84;
    let rd = |b: &[u8], o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    for k in 0..count {
        if off + 50 > bytes.len() { break; }
        if k % 65536 == 0 { crate::progress::set(k as f32 / count.max(1) as f32); }
        let n = Vec3::new(rd(bytes, off), rd(bytes, off + 4), rd(bytes, off + 8));
        let mut p = [Vec3::ZERO; 3];
        for (i, pt) in p.iter_mut().enumerate() {
            let b = off + 12 + i * 12;
            *pt = Vec3::new(rd(bytes, b), rd(bytes, b + 4), rd(bytes, b + 8));
        }
        tris.push((fix_normal(n, &p), p));
        off += 50;
    }
    tris
}

/// Si la normale fournie est nulle/invalide, on la recalcule depuis le triangle.
fn fix_normal(n: Vec3, p: &[Vec3]) -> Vec3 {
    if n.length_squared() > 1e-8 {
        n.normalize()
    } else {
        (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero()
    }
}

/// Construit un Mesh (normales à plat + bornes) depuis une liste de triangles.
pub fn from_tris(tris: &[[Vec3; 3]]) -> Mesh {
    let mut verts = Vec::with_capacity(tris.len() * 18);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for p in tris {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        for v in p {
            verts.extend_from_slice(&[v.x, v.y, v.z, n.x, n.y, n.z]);
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    if !tris.is_empty() { Mesh { verts, min, max } } else { Mesh { verts, min: Vec3::ZERO, max: Vec3::ZERO } }
}

/// Charge n'importe quel maillage reconnu d'après l'extension (STL / OBJ / PLY).
/// STEP et IGES passent par OCCT (voir occt::import_cad), traités en amont.
pub fn load_any(path: &str) -> std::io::Result<Mesh> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "obj" => load_obj(path),
        "ply" => load_ply(path),
        _ => load(path),
    }
}

// ---------- OBJ ----------

pub fn load_obj(path: &str) -> std::io::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    let mut pos: Vec<Vec3> = Vec::new();
    let mut tris: Vec<[Vec3; 3]> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f32> = it.take(3).map(|s| s.parse().unwrap_or(0.0)).collect();
                if c.len() == 3 { pos.push(Vec3::new(c[0], c[1], c[2])); }
            }
            Some("f") => {
                // indices (éventuellement v/vt/vn), 1-based, négatifs = relatifs
                let idx: Vec<i64> = it.filter_map(|tok| {
                    let v = tok.split('/').next().unwrap_or("");
                    v.parse::<i64>().ok()
                }).collect();
                let resolve = |i: i64| -> Option<usize> {
                    if i > 0 { Some((i - 1) as usize) }
                    else if i < 0 { let n = pos.len() as i64; let r = n + i; if r >= 0 { Some(r as usize) } else { None } }
                    else { None }
                };
                // triangulation en éventail
                for k in 1..idx.len().saturating_sub(1) {
                    if let (Some(a), Some(b), Some(c)) = (resolve(idx[0]), resolve(idx[k]), resolve(idx[k + 1])) {
                        if let (Some(&pa), Some(&pb), Some(&pc)) = (pos.get(a), pos.get(b), pos.get(c)) {
                            tris.push([pa, pb, pc]);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(from_tris(&tris))
}

pub fn write_obj(path: &std::path::Path, mesh: &Mesh) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(f, "# exporté par 3D-Projects")?;
    let ntri = mesh.verts.len() / 18;
    for c in mesh.verts.chunks_exact(6) { writeln!(f, "v {} {} {}", c[0], c[1], c[2])?; }
    for t in 0..ntri { let b = (t * 3 + 1) as usize; writeln!(f, "f {} {} {}", b, b + 1, b + 2)?; }
    Ok(())
}

// ---------- PLY (ASCII + binaire petit-boutiste) ----------

pub fn load_ply(path: &str) -> std::io::Result<Mesh> {
    let bytes = std::fs::read(path)?;
    // En-tête : jusqu'à "end_header\n"
    let hdr_end = bytes.windows(10).position(|w| w == b"end_header")
        .map(|p| p + 10).unwrap_or(0);
    let mut i = hdr_end;
    while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
    let body_start = (i + 1).min(bytes.len());
    let header = String::from_utf8_lossy(&bytes[..hdr_end]);

    let mut ascii = true;
    let mut little = true;
    let mut nverts = 0usize;
    let mut nfaces = 0usize;
    let mut vprops: Vec<String> = Vec::new(); // noms des propriétés du sommet
    let mut cur = ""; // élément courant
    for line in header.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.as_slice() {
            ["format", f, _] => { ascii = f.starts_with("ascii"); little = !f.contains("big"); }
            ["element", "vertex", n] => { nverts = n.parse().unwrap_or(0); cur = "vertex"; }
            ["element", "face", n] => { nfaces = n.parse().unwrap_or(0); cur = "face"; }
            ["element", _, _] => { cur = "other"; }
            ["property", _ty, name] if cur == "vertex" => { vprops.push(name.to_string()); }
            _ => {}
        }
    }
    let xi = vprops.iter().position(|p| p == "x").unwrap_or(0);
    let yi = vprops.iter().position(|p| p == "y").unwrap_or(1);
    let zi = vprops.iter().position(|p| p == "z").unwrap_or(2);

    let mut pos: Vec<Vec3> = Vec::with_capacity(nverts);
    let mut tris: Vec<[Vec3; 3]> = Vec::new();

    if ascii {
        let text = String::from_utf8_lossy(&bytes[body_start..]);
        let mut lines = text.lines();
        for _ in 0..nverts {
            if let Some(l) = lines.next() {
                let vals: Vec<f32> = l.split_whitespace().map(|s| s.parse().unwrap_or(0.0)).collect();
                pos.push(Vec3::new(*vals.get(xi).unwrap_or(&0.0), *vals.get(yi).unwrap_or(&0.0), *vals.get(zi).unwrap_or(&0.0)));
            }
        }
        for _ in 0..nfaces {
            if let Some(l) = lines.next() {
                let vals: Vec<i64> = l.split_whitespace().map(|s| s.parse().unwrap_or(0)).collect();
                if vals.is_empty() { continue; }
                let n = vals[0] as usize;
                let f: Vec<usize> = vals[1..].iter().take(n).map(|&v| v as usize).collect();
                for k in 1..f.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (pos.get(f[0]), pos.get(f[k]), pos.get(f[k + 1])) {
                        tris.push([a, b, c]);
                    }
                }
            }
        }
    } else if little {
        // binaire petit-boutiste : on suppose des propriétés de sommet en f32 (cas courant).
        let np = vprops.len().max(3);
        let rf = |b: &[u8], o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let mut o = body_start;
        for _ in 0..nverts {
            if o + np * 4 > bytes.len() { break; }
            let g = |k: usize| rf(&bytes, o + k * 4);
            pos.push(Vec3::new(g(xi), g(yi), g(zi)));
            o += np * 4;
        }
        // faces : u8 count puis count * i32
        for _ in 0..nfaces {
            if o >= bytes.len() { break; }
            let n = bytes[o] as usize; o += 1;
            let mut f = Vec::with_capacity(n);
            for _ in 0..n {
                if o + 4 > bytes.len() { break; }
                f.push(i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize);
                o += 4;
            }
            for k in 1..f.len().saturating_sub(1) {
                if let (Some(&a), Some(&b), Some(&c)) = (pos.get(f[0]), pos.get(f[k]), pos.get(f[k + 1])) {
                    tris.push([a, b, c]);
                }
            }
        }
    } else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, crate::trc("ply_be")));
    }
    Ok(from_tris(&tris))
}

pub fn write_ply(path: &std::path::Path, mesh: &Mesh) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let nverts = mesh.verts.len() / 6;
    let ntri = mesh.verts.len() / 18;
    writeln!(f, "ply")?;
    writeln!(f, "format ascii 1.0")?;
    writeln!(f, "comment exporté par 3D-Projects")?;
    writeln!(f, "element vertex {nverts}")?;
    writeln!(f, "property float x")?;
    writeln!(f, "property float y")?;
    writeln!(f, "property float z")?;
    writeln!(f, "element face {ntri}")?;
    writeln!(f, "property list uchar int vertex_indices")?;
    writeln!(f, "end_header")?;
    for c in mesh.verts.chunks_exact(6) { writeln!(f, "{} {} {}", c[0], c[1], c[2])?; }
    for t in 0..ntri { let b = t * 3; writeln!(f, "3 {} {} {}", b, b + 1, b + 2)?; }
    Ok(())
}
