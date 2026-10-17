//! Réparation « A.I » : enveloppe fermée (watertight) de l'objet par voxelisation
//! + extraction de surface (naive surface nets). Tous les trous sont bouchés par
//! construction : la surface extraite est un manifold fermé qui entoure l'objet.
//!
//! Étapes : coque voxel (échantillonnage des triangles) -> dilatation (scelle les
//! trous) -> flood-fill extérieur -> solide = non-extérieur -> érosion (recolle la
//! surface sur l'objet) -> champ lissé -> surface nets orienté par le gradient.

use glam::Vec3;
use crate::fasthash::HashMap;

/// Lissage de Taubin (λ|μ) : enlève l'escalier de voxelisation SANS rétrécir le volume
/// (contrairement au simple Laplacien). Ne change pas la topologie -> reste watertight.
fn smooth_taubin(verts: &[f32], passes: usize) -> Vec<f32> {
    let ntri = verts.len() / 18;
    if ntri == 0 { return verts.to_vec(); }
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for t in 0..ntri { for k in 0..3 { let o = t*18 + k*6; for d in 0..3 { lo[d] = lo[d].min(verts[o+d]); hi[d] = hi[d].max(verts[o+d]); } } }
    let ext = (hi[0]-lo[0]).max(hi[1]-lo[1]).max(hi[2]-lo[2]).max(1e-6);
    let cell = ext * 1e-5;
    let q = |v: f32| (v / cell).round() as i64;
    let mut map: HashMap<(i64,i64,i64), u32> = HashMap::default();
    let mut pos: Vec<Vec3> = Vec::new();
    let mut idx: Vec<u32> = Vec::with_capacity(ntri*3);
    for t in 0..ntri { for k in 0..3 {
        let o = t*18 + k*6;
        let key = (q(verts[o]), q(verts[o+1]), q(verts[o+2]));
        let id = *map.entry(key).or_insert_with(|| { pos.push(Vec3::new(verts[o], verts[o+1], verts[o+2])); (pos.len()-1) as u32 });
        idx.push(id);
    }}
    // voisinage (sommets reliés par une arête)
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); pos.len()];
    for t in 0..ntri {
        let tr = [idx[t*3], idx[t*3+1], idx[t*3+2]];
        for e in 0..3 { let (a, b) = (tr[e], tr[(e+1)%3]); adj[a as usize].push(b); adj[b as usize].push(a); }
    }
    for a in &mut adj { a.sort_unstable(); a.dedup(); }
    let (lambda, mu) = (0.5f32, -0.53f32);
    let nth = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    for _ in 0..passes {
        for f in [lambda, mu] {
            let src = pos.clone();
            let per = ((pos.len() + nth - 1) / nth).max(1);
            std::thread::scope(|sc| {
                for (ci, part) in pos.chunks_mut(per).enumerate() {
                    let (src, adj) = (&src, &adj);
                    sc.spawn(move || {
                        for (k, pv) in part.iter_mut().enumerate() {
                            let v = ci * per + k;
                            if adj[v].is_empty() { continue; }
                            let mut avg = Vec3::ZERO; for &n in &adj[v] { avg += src[n as usize]; }
                            avg /= adj[v].len() as f32;
                            *pv = src[v] + (avg - src[v]) * f;
                        }
                    });
                }
            });
        }
    }
    // Normales LISSES (moyenne pondérée par l'aire des faces adjacentes) : supprime
    // l'aspect granuleux dû aux normales à plat sur un maillage dense terracé.
    let mut vn = vec![Vec3::ZERO; pos.len()];
    for t in 0..ntri {
        let (ia, ib, ic) = (idx[t*3] as usize, idx[t*3+1] as usize, idx[t*3+2] as usize);
        let fnrm = (pos[ib]-pos[ia]).cross(pos[ic]-pos[ia]); // non normalisé = pondéré par l'aire
        vn[ia] += fnrm; vn[ib] += fnrm; vn[ic] += fnrm;
    }
    for n in &mut vn { *n = n.normalize_or_zero(); }
    let mut out = Vec::with_capacity(verts.len());
    for t in 0..ntri {
        for k in 0..3 {
            let vi = idx[t*3+k] as usize; let (p, n) = (pos[vi], vn[vi]);
            out.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
        }
    }
    out
}

/// Fondations pour l'impression : pose le maillage sur le lit (min Z -> 0) et rabat au sol
/// (Z = 0) tous les sommets situés à moins de `found` mm du bas, pour que les fils proches
/// du plancher le touchent (bonne adhérence première couche).
pub fn floor_foundation(verts: &mut [f32], found: f32) {
    let mut minz = f32::INFINITY;
    for c in verts.chunks_exact(6) { minz = minz.min(c[2]); }
    if !minz.is_finite() { return; }
    for c in verts.chunks_exact_mut(6) {
        c[2] -= minz;                         // posé sur le lit
        if c[2] <= found.max(0.0) { c[2] = 0.0; } // collé au sol si proche du plancher
    }
}

/// Décimation par regroupement de sommets (vertex clustering) : ramène un maillage dense
/// à une grille de `res` cellules sur le plus grand côté. Rapide et robuste ; sert à rendre
/// le fil de fer possible sur des objets trop denses, et à régler sa finesse.
pub fn decimate(verts: &[f32], res: usize) -> Vec<f32> {
    let ntri = verts.len() / 18;
    if ntri == 0 { return Vec::new(); }
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for t in 0..ntri { for k in 0..3 { let o = t*18+k*6; for d in 0..3 { lo[d]=lo[d].min(verts[o+d]); hi[d]=hi[d].max(verts[o+d]); } } }
    let maxd = (hi[0]-lo[0]).max(hi[1]-lo[1]).max(hi[2]-lo[2]).max(1e-6);
    let res = res.clamp(4, 500);
    let h = maxd / res as f32;
    let key = |o: usize| (
        ((verts[o]   - lo[0]) / h).floor() as i64,
        ((verts[o+1] - lo[1]) / h).floor() as i64,
        ((verts[o+2] - lo[2]) / h).floor() as i64);
    // centroïde par cellule
    let mut acc: HashMap<(i64,i64,i64), (Vec3, u32)> = HashMap::default();
    for t in 0..ntri { for k in 0..3 { let o = t*18+k*6; let e = acc.entry(key(o)).or_insert((Vec3::ZERO, 0)); e.0 += Vec3::new(verts[o],verts[o+1],verts[o+2]); e.1 += 1; } }
    let mut idmap: HashMap<(i64,i64,i64), u32> = HashMap::default();
    let mut reps: Vec<Vec3> = Vec::new();
    for (k, (s, n)) in &acc { idmap.insert(*k, reps.len() as u32); reps.push(*s / *n as f32); }
    let mut out: Vec<f32> = Vec::new();
    let mut seen: crate::fasthash::HashSet<[u32;3]> = crate::fasthash::HashSet::default();
    for t in 0..ntri {
        let (a, b, c) = (idmap[&key(t*18)], idmap[&key(t*18+6)], idmap[&key(t*18+12)]);
        if a == b || b == c || a == c { continue; }           // triangle écrasé
        let mut s = [a, b, c]; s.sort_unstable();
        if !seen.insert(s) { continue; }                      // doublon
        let (pa, pb, pc) = (reps[a as usize], reps[b as usize], reps[c as usize]);
        let n = (pb-pa).cross(pc-pa).normalize_or_zero();
        for p in [pa, pb, pc] { out.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]); }
    }
    out
}

/// Petite sphère low-poly (UV) ajoutée au maillage de sortie (jointures du treillis).
fn add_ball(out: &mut Vec<f32>, c: Vec3, r: f32, seg: usize) {
    let (slon, slat) = (seg.max(4), (seg/2).max(3));
    let p = |i: usize, j: usize| -> Vec3 {
        let lon = i as f32 / slon as f32 * std::f32::consts::TAU;
        let lat = j as f32 / slat as f32 * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
        c + Vec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()) * r
    };
    let mut tri = |a: Vec3, b: Vec3, d: Vec3| {
        let n = (b - a).cross(d - a).normalize_or_zero();
        for q in [a, b, d] { out.extend_from_slice(&[q.x, q.y, q.z, n.x, n.y, n.z]); }
    };
    for j in 0..slat { for i in 0..slon {
        let (a, b, cc, dd) = (p(i, j), p(i+1, j), p(i+1, j+1), p(i, j+1));
        tri(a, b, cc); tri(a, cc, dd);
    }}
}

/// Convertit les ARÊTES du maillage en fils imprimables : chaque arête devient un FIL
/// (tube à `sides` côtés) de rayon `strut_r` = demi-largeur de buse. PAS de bille aux
/// sommets (pas de sur-épaisseur) : les fils se croisent simplement pour former la forme.
/// Résultat prêt pour le slicer (STL). Abandonne si trop d'arêtes.
pub fn wireframe_solid(verts: &[f32], strut_r: f32, sides: usize) -> Result<Vec<f32>, String> {
    let ntri = verts.len() / 18;
    if ntri == 0 { return Err(crate::trc("obj_vide").into()); }
    // soudure des sommets
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for t in 0..ntri { for k in 0..3 { let o = t*18+k*6; for d in 0..3 { lo[d]=lo[d].min(verts[o+d]); hi[d]=hi[d].max(verts[o+d]); } } }
    let ext = (hi[0]-lo[0]).max(hi[1]-lo[1]).max(hi[2]-lo[2]).max(1e-6);
    let cell = ext * 1e-5; let q = |v: f32| (v/cell).round() as i64;
    let mut map: HashMap<(i64,i64,i64), u32> = HashMap::default();
    let mut pos: Vec<Vec3> = Vec::new(); let mut idx: Vec<u32> = Vec::with_capacity(ntri*3);
    for t in 0..ntri { for k in 0..3 { let o=t*18+k*6;
        let key=(q(verts[o]),q(verts[o+1]),q(verts[o+2]));
        let id=*map.entry(key).or_insert_with(|| { pos.push(Vec3::new(verts[o],verts[o+1],verts[o+2])); (pos.len()-1) as u32 });
        idx.push(id);
    }}
    // arêtes uniques
    let mut eset: crate::fasthash::HashSet<(u32,u32)> = crate::fasthash::HashSet::default();
    for t in 0..ntri { let tr=[idx[t*3],idx[t*3+1],idx[t*3+2]];
        for e in 0..3 { let (a,b)=(tr[e],tr[(e+1)%3]); if a!=b { eset.insert(if a<b {(a,b)} else {(b,a)}); } } }
    if eset.len() > 250_000 { return Err(format!("{} ({} {})", crate::trc("trop_aretes"), eset.len(), crate::trc("aretes"))); }
    let sides = sides.clamp(3, 16);
    let mut out: Vec<f32> = Vec::new();
    let mut tri = |out: &mut Vec<f32>, a: Vec3, b: Vec3, c: Vec3| {
        let n = (b-a).cross(c-a).normalize_or_zero();
        for p in [a,b,c] { out.extend_from_slice(&[p.x,p.y,p.z,n.x,n.y,n.z]); }
    };
    let ne = eset.len().max(1);
    for (k, &(ia, ib)) in eset.iter().enumerate() {
        if k % 8192 == 0 { crate::progress::set(0.3 + 0.7 * k as f32 / ne as f32); }
        let (pa, pb) = (pos[ia as usize], pos[ib as usize]);
        let axis = pb - pa; let len = axis.length(); if len < 1e-5 { continue; }
        let az = axis / len;
        let refv = if az.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let u = (refv - az * refv.dot(az)).normalize_or_zero();
        let v = az.cross(u);
        let ring = |ctr: Vec3| -> Vec<Vec3> { (0..sides).map(|i| { let th = i as f32/sides as f32*std::f32::consts::TAU; ctr + (u*th.cos()+v*th.sin())*strut_r }).collect() };
        let (r0, r1) = (ring(pa), ring(pb));
        for i in 0..sides { let j = (i+1)%sides;
            tri(&mut out, r0[i], r0[j], r1[j]); tri(&mut out, r0[i], r1[j], r1[i]); // paroi
            tri(&mut out, pa, r0[j], r0[i]); tri(&mut out, pb, r1[i], r1[j]);        // bouchons
        }
    }
    // Pas de bille aux jointures : les fils se croisent, sans sur-épaisseur.
    Ok(out)
}

fn tris_of(verts: &[f32]) -> Vec<[Vec3; 3]> {
    verts.chunks_exact(18).map(|c| [
        Vec3::new(c[0], c[1], c[2]),
        Vec3::new(c[6], c[7], c[8]),
        Vec3::new(c[12], c[13], c[14]),
    ]).collect()
}

/// Grille de voxels (origine, pas, dimensions).
#[derive(Clone, Copy)]
pub struct Grille { pub lo: Vec3, pub h: f32, pub nx: usize, pub ny: usize, pub nz: usize }

/// Volume plein de l'objet en voxels (surface voxelisée, trous scellés, extérieur rempli depuis
/// le bord, puis érosion pour recoller à la surface). `res` = voxels visés sur le plus grand côté.
fn solide(tris_: &[[Vec3; 3]], res: usize) -> Option<(Grille, Vec<bool>)> {
    let tris = tris_.to_vec();
    let tt = std::time::Instant::now();
    let dbg = |m: &str, t: &std::time::Instant| if std::env::var_os("REMESH_PROF").is_some() { eprintln!("  [env] {m} : {:.0} ms", t.elapsed().as_secs_f32() * 1000.0); };
    // --- boîte englobante + taille de voxel ---
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for t in &tris { for p in t { lo = lo.min(*p); hi = hi.max(*p); } }
    let maxd = (hi - lo).max_element().max(1e-6);
    let res = res.clamp(24, 512);
    let mut h = maxd / res as f32;
    let pad = 3.0 * h;
    lo -= Vec3::splat(pad);
    hi += Vec3::splat(pad);
    let ext = hi - lo;
    let dim = |e: f32, h: f32| ((e / h).ceil() as usize + 1).max(2);
    let (mut nx, mut ny, mut nz) = (dim(ext.x, h), dim(ext.y, h), dim(ext.z, h));
    // budget mémoire/temps : 30 M voxels à la finesse 300, proportionnel au cube au-delà (GPU puissant)
    let budget = (30_000_000.0 * (res as f64 / 300.0).powi(3).max(1.0)).min(140_000_000.0) as usize;
    while nx * ny * nz > budget {
        h *= 1.26;
        nx = dim(ext.x, h); ny = dim(ext.y, h); nz = dim(ext.z, h);
    }
    let idx = |x: usize, y: usize, z: usize| (z * ny + y) * nx + x;
    let n = nx * ny * nz;
    if std::env::var("REMESH_DBG").is_ok() { eprintln!("  [remesh] maxd={maxd:.2} h={h:.3} grid={nx}x{ny}x{nz}={n}"); }

    let grow = 2i64;
    // GPU (option « Activer GPU ») : coque + épaississement + extérieur sur la carte
    let gpu_ext = crate::gpu::exterior(crate::gpu::Req {
        tris: tris.iter().flat_map(|t| [t[0].x, t[0].y, t[0].z, t[1].x, t[1].y, t[1].z, t[2].x, t[2].y, t[2].z]).collect(),
        org: [lo.x, lo.y, lo.z], h, off: 0, dims: [nx as i32, ny as i32, nz as i32], cube: false, grow6: grow as u32 });
    let outside: Vec<bool> = if let Some(bits) = gpu_ext { (0..n).map(|i| crate::gpu::bit(&bits, i)).collect() } else {
        // --- 1) coque : voxels traversés par la surface ---
        let mut shell = vec![false; n];
        for t in &tris {
            let (a, ab, ac) = (t[0], t[1] - t[0], t[2] - t[0]);
            let steps = ((ab.length().max(ac.length()) / (h * 0.5)).ceil() as usize + 1).max(2);
            for i in 0..=steps {
                for j in 0..=(steps - i) {
                    let p = a + ab * (i as f32 / steps as f32) + ac * (j as f32 / steps as f32);
                    let c = (p - lo) / h;
                    let (x, y, z) = (c.x as i64, c.y as i64, c.z as i64);
                    if x >= 0 && y >= 0 && z >= 0 && (x as usize) < nx && (y as usize) < ny && (z as usize) < nz {
                        shell[idx(x as usize, y as usize, z as usize)] = true;
                    }
                }
            }
        }

        // --- 2) dilatation de la coque : scelle les trous jusqu'à ~2*grow voxels ---
        let dilate6 = |src: &[bool], nx: usize, ny: usize, nz: usize| -> Vec<bool> {
            let idx = |x: usize, y: usize, z: usize| (z * ny + y) * nx + x;
            let mut out = src.to_vec();
            for z in 0..nz { for y in 0..ny { for x in 0..nx {
                if src[idx(x, y, z)] { continue; }
                let mut near = false;
                for (dx, dy, dz) in [(-1i64,0,0),(1,0,0),(0,-1,0),(0,1,0),(0,0,-1),(0,0,1)] {
                    let (xx, yy, zz) = (x as i64 + dx, y as i64 + dy, z as i64 + dz);
                    if xx >= 0 && yy >= 0 && zz >= 0 && (xx as usize) < nx && (yy as usize) < ny && (zz as usize) < nz
                        && src[idx(xx as usize, yy as usize, zz as usize)] { near = true; break; }
                }
                if near { out[idx(x, y, z)] = true; }
            }}}
            out
        };
        let mut wall = shell;
        for _ in 0..grow { wall = dilate6(&wall, nx, ny, nz); }

        // --- 3) flood-fill extérieur (voxels vides atteignables depuis le bord) ---
        let mut outside = vec![false; n];
        let mut stack: Vec<(usize, usize, usize)> = Vec::new();
        for z in 0..nz { for y in 0..ny { for x in 0..nx {
            if x == 0 || y == 0 || z == 0 || x == nx-1 || y == ny-1 || z == nz-1 {
                let id = idx(x, y, z);
                if !wall[id] && !outside[id] { outside[id] = true; stack.push((x, y, z)); }
            }
        }}}
        while let Some((x, y, z)) = stack.pop() {
            for (dx, dy, dz) in [(-1i64,0,0),(1,0,0),(0,-1,0),(0,1,0),(0,0,-1),(0,0,1)] {
                let (xx, yy, zz) = (x as i64 + dx, y as i64 + dy, z as i64 + dz);
                if xx >= 0 && yy >= 0 && zz >= 0 && (xx as usize) < nx && (yy as usize) < ny && (zz as usize) < nz {
                    let id = idx(xx as usize, yy as usize, zz as usize);
                    if !wall[id] && !outside[id] { outside[id] = true; stack.push((xx as usize, yy as usize, zz as usize)); }
                }
            }
        }

        outside
    };

    dbg("voxels + extérieur", &tt); let tt = std::time::Instant::now();
    // --- 4) solide = non-extérieur ; érosion pour recoller la surface sur l'objet ---
    let mut solid: Vec<bool> = outside.iter().map(|&o| !o).collect();
    for _ in 0..grow {
        let src = solid.clone();
        for z in 0..nz { for y in 0..ny { for x in 0..nx {
            if !src[idx(x, y, z)] { continue; }
            let mut edge = false;
            for (dx, dy, dz) in [(-1i64,0,0),(1,0,0),(0,-1,0),(0,1,0),(0,0,-1),(0,0,1)] {
                let (xx, yy, zz) = (x as i64 + dx, y as i64 + dy, z as i64 + dz);
                if xx < 0 || yy < 0 || zz < 0 || (xx as usize) >= nx || (yy as usize) >= ny || (zz as usize) >= nz
                    || !src[idx(xx as usize, yy as usize, zz as usize)] { edge = true; break; }
            }
            if edge { solid[idx(x, y, z)] = false; }
        }}}
    }

    dbg("érosion", &tt);
    Some((Grille { lo, h, nx, ny, nz }, solid))
}

/// Surface iso `iso` du champ `fld` (dedans = valeur > iso) par marching tetrahedra :
/// 2-manifold et étanche par construction ; normales vers l'extérieur.
fn extraire(g: &Grille, fld: &[f32], iso: f32) -> Vec<f32> {
    let Grille { lo, h, nx, ny, nz } = *g;
    let idx = |x: usize, y: usize, z: usize| (z * ny + y) * nx + x;
    let nth = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    // --- 6) extraction par MARCHING TETRAHEDRA ---
    // Chaque cube est découpé en 6 tétraèdres (décomposition de Freudenthal, même
    // diagonale 0-7 partout → faces cohérentes entre cubes voisins). Le marching
    // tetrahedra est 2-manifold et watertight par construction (pas de table, pas
    // de cas ambigu comme en surface nets / marching cubes).
    let (cnx, cny, cnz) = (nx - 1, ny - 1, nz - 1);
    if cnx == 0 || cny == 0 || cnz == 0 { return Vec::new(); }
    // positions des 8 coins d'un cube (indice = bits x,y,z)
    let coff: [(usize, usize, usize); 8] = [(0,0,0),(1,0,0),(0,1,0),(1,1,0),(0,0,1),(1,0,1),(0,1,1),(1,1,1)];
    // 6 tétraèdres partageant la diagonale 0-7
    let tets: [[usize; 4]; 6] = [[0,1,3,7],[0,1,5,7],[0,2,3,7],[0,2,6,7],[0,4,5,7],[0,4,6,7]];

    let emit = |out: &mut Vec<f32>, p: [Vec3; 3], inside_ref: Vec3| {
        let mut n = (p[1] - p[0]).cross(p[2] - p[0]);
        if n.length_squared() < 1e-20 { return; }
        n = n.normalize();
        let ctr = (p[0] + p[1] + p[2]) / 3.0;
        // la normale doit pointer vers l'extérieur (à l'opposé des sommets « dedans »)
        let (a, b, c) = if n.dot(inside_ref - ctr) > 0.0 { (p[0], p[2], p[1]) } else { (p[0], p[1], p[2]) };
        let nn = (b - a).cross(c - a).normalize_or_zero();
        for q in [a, b, c] { out.extend_from_slice(&[q.x, q.y, q.z, nn.x, nn.y, nn.z]); }
    };

    let per = ((cnz + nth - 1) / nth).max(1);
    let parts: Vec<Vec<f32>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..cnz).step_by(per).map(|z0| { let fld = &fld; sc.spawn(move || {
    let mut out: Vec<f32> = Vec::new();
    for z in z0..(z0 + per).min(cnz) { for y in 0..cny { for x in 0..cnx {
        // valeurs et positions des 8 coins
        let mut cv = [0f32; 8];
        let mut cp = [Vec3::ZERO; 8];
        for i in 0..8 {
            let (dx, dy, dz) = coff[i];
            cv[i] = fld[idx(x + dx, y + dy, z + dz)];
            cp[i] = lo + Vec3::new((x + dx) as f32, (y + dy) as f32, (z + dz) as f32) * h;
        }
        for tet in tets {
            // indices dedans / dehors
            let mut ins = [0usize; 4]; let mut ni = 0;
            let mut outs = [0usize; 4]; let mut no = 0;
            for &c in &tet { if cv[c] > iso { ins[ni] = c; ni += 1; } else { outs[no] = c; no += 1; } }
            if ni == 0 || ni == 4 { continue; }
            // point d'intersection sur l'arête (a dedans, b dehors)
            let cross = |a: usize, b: usize| -> Vec3 {
                let t = ((iso - cv[a]) / (cv[b] - cv[a])).clamp(0.0, 1.0);
                cp[a] + (cp[b] - cp[a]) * t
            };
            let iref = {
                let mut s = Vec3::ZERO; for k in 0..ni { s += cp[ins[k]]; } s / ni as f32
            };
            match ni {
                1 => { let a = ins[0];
                    emit(&mut out, [cross(a, outs[0]), cross(a, outs[1]), cross(a, outs[2])], iref); }
                3 => { let d = outs[0];
                    emit(&mut out, [cross(ins[0], d), cross(ins[1], d), cross(ins[2], d)], iref); }
                _ => { // 2 dedans, 2 dehors : quad (deux triangles)
                    let (a, b, c, d) = (ins[0], ins[1], outs[0], outs[1]);
                    let (ac, ad, bc, bd) = (cross(a, c), cross(a, d), cross(b, c), cross(b, d));
                    emit(&mut out, [ac, ad, bd], iref);
                    emit(&mut out, [ac, bd, bc], iref);
                }
            }
        }
    }}}
    out
        }) }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    parts.concat()
}

/// Enveloppe fermée de l'objet. `res` = nombre visé de voxels sur le plus grand côté.
/// Renvoie des sommets entrelacés [pos nrm] * 3 par triangle (format du viewer).
pub fn envelope(verts: &[f32], res: usize) -> Vec<f32> {
    let tris = tris_of(verts);
    if tris.is_empty() { return Vec::new(); }
    let Some((g, solid)) = solide(&tris, res) else { return Vec::new() };
    let Grille { nx, ny, nz, .. } = g;
    let tt = std::time::Instant::now();
    let dbg = |m: &str, t: &std::time::Instant| if std::env::var_os("REMESH_PROF").is_some() { eprintln!("  [env] {m} : {:.0} ms", t.elapsed().as_secs_f32() * 1000.0); };
    // --- 5) champ scalaire lissé (1 dedans / 0 dehors puis flou boîte) ---
    // Un lissage généreux arrondit les cellules « selle » ambiguës qui, sinon,
    // produisent des arêtes non-manifold en surface nets naïf.
    let mut fld: Vec<f32> = solid.iter().map(|&s| if s { 1.0 } else { 0.0 }).collect();
    let nth = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    for _ in 0..2 {
        let src = fld.clone();
        let slab = nx * ny;
        let per = ((nz + nth - 1) / nth).max(1);
        std::thread::scope(|sc| {
            for (ci, part) in fld.chunks_mut(slab * per).enumerate() {
                let src = &src;
                sc.spawn(move || {
                    for (k, val) in part.iter_mut().enumerate() {
                        let id = ci * slab * per + k;
                        let (z, y, x) = (id / slab, (id % slab) / nx, id % nx);
                        let mut sum = src[id]; let mut cnt = 1.0f32;
                        for (dx, dy, dz) in [(-1i64,0,0),(1,0,0),(0,-1,0),(0,1,0),(0,0,-1),(0,0,1)] {
                            let (xx, yy, zz) = (x as i64 + dx, y as i64 + dy, z as i64 + dz);
                            if xx >= 0 && yy >= 0 && zz >= 0 && (xx as usize) < nx && (yy as usize) < ny && (zz as usize) < nz {
                                sum += src[((zz as usize) * ny + yy as usize) * nx + xx as usize]; cnt += 1.0;
                            }
                        }
                        *val = sum / cnt;
                    }
                });
            }
        });
    }

    dbg("flou", &tt); let tt = std::time::Instant::now();
    let out = extraire(&g, &fld, 0.5);
    // lissage : efface l'escalier de voxelisation tout en restant collé à l'objet
    dbg("marching tetrahedra", &tt); let tt = std::time::Instant::now();
    let r = smooth_taubin(&out, 18);
    dbg("lissage Taubin", &tt);
    r
}

/// Distance euclidienne exacte au carré (Felzenszwalb), une ligne : `f` = coût (0 sur les voxels
/// hors de l'objet, LOIN dedans) -> `d`.
const LOIN: f32 = 1.0e12;
fn edt_ligne(f: &[f32], d: &mut [f32], v: &mut Vec<usize>, z: &mut Vec<f32>) {
    let n = f.len();
    v.clear(); v.resize(n, 0);
    z.clear(); z.resize(n + 1, 0.0);
    let mut k = 0usize;
    z[0] = f32::NEG_INFINITY; z[1] = f32::INFINITY;
    let inter = |q: usize, p: usize| ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
    for q in 1..n {
        let mut s = inter(q, v[k]);
        while s <= z[k] {
            if k == 0 { break; }
            k -= 1;
            s = inter(q, v[k]);
        }
        if s <= z[k] { v[0] = q; z[0] = f32::NEG_INFINITY; z[1] = f32::INFINITY; continue; } // k == 0
        k += 1; v[k] = q; z[k] = s; z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f32 { k += 1; }
        let dq = q as f32 - v[k] as f32;
        d[q] = (dq * dq + f[v[k]]).min(LOIN);
    }
}

struct Ptr(*mut f32);
unsafe impl Send for Ptr {}
unsafe impl Sync for Ptr {}

/// Transformée de distance 3D (au carré, en voxels) sur toute la grille, axe par axe, en parallèle.
fn edt3d(g: &Grille, cout: &mut [f32]) {
    let Grille { nx, ny, nz, .. } = *g;
    let nth = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    // (longueur de ligne, pas, départs des lignes)
    let passes: [(usize, usize, Vec<usize>); 3] = [
        (nx, 1, (0..ny * nz).map(|l| l * nx).collect()),
        (ny, nx, (0..nz).flat_map(|z| (0..nx).map(move |x| z * nx * ny + x)).collect()),
        (nz, nx * ny, (0..nx * ny).collect()),
    ];
    for (len, pas, departs) in passes.iter() {
        let ptr = Ptr(cout.as_mut_ptr());
        let par = (departs.len() + nth - 1) / nth;
        std::thread::scope(|sc| {
            for bloc in departs.chunks(par.max(1)) {
                let ptr = &ptr;
                sc.spawn(move || {
                    let (mut f, mut d, mut v, mut z) = (vec![0f32; *len], vec![0f32; *len], Vec::new(), Vec::new());
                    for &dep in bloc {
                        // lignes disjointes : chaque fil écrit seulement les siennes
                        unsafe { for i in 0..*len { f[i] = *ptr.0.add(dep + i * pas); } }
                        edt_ligne(&f, &mut d, &mut v, &mut z);
                        unsafe { for i in 0..*len { *ptr.0.add(dep + i * pas) = d[i]; } }
                    }
                });
            }
        });
    }
}

/// CREUSER : surface intérieure de l'objet, à `epaisseur` (mm) sous sa peau (normales vers
/// l'extérieur de la cavité), allégée à environ `cible` triangles. GARANTIE : aucun point de la
/// paroi intérieure n'est à moins de l'épaisseur demandée de la surface de l'objet (contrôle sur
/// la carte des distances exactes après lissage et allègement, points trop proches repoussés vers
/// l'intérieur) : la cavité ne perce jamais la peau. Vide si la paroi remplit tout l'objet.
pub fn interieur(verts: &[f32], epaisseur: f32, res: usize, cible: usize) -> Vec<f32> {
    let tris = tris_of(verts);
    if tris.is_empty() { return Vec::new(); }
    // finesse : au moins ~4 voxels dans l'épaisseur de la paroi (dans la limite du budget mémoire)
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for t in &tris { for p in t { lo = lo.min(*p); hi = hi.max(*p); } }
    let maxd = (hi - lo).max_element().max(1e-6);
    let res = res.max((maxd / (epaisseur / 4.0).max(1e-3)).ceil() as usize).min(512);
    let Some((g, solid)) = solide(&tris, res) else { return Vec::new() };
    crate::progress::set(0.35);
    let mut d2: Vec<f32> = solid.iter().map(|&s| if s { LOIN } else { 0.0 }).collect();
    edt3d(&g, &mut d2);
    crate::progress::set(0.6);
    // distance (mm) au bord de l'objet ; la cavité = plus loin que l'épaisseur (+ 0,3 voxel :
    // le lissage et l'allègement ne ramènent presque aucun point trop près)
    let h = g.h;
    let dist: Vec<f32> = d2.iter().map(|&q| (q.sqrt() - 0.5).max(0.0) * h).collect();
    drop(d2);
    let fld: Vec<f32> = dist.iter().map(|&d| d - epaisseur - 0.3 * h).collect();
    let out = extraire(&g, &fld, 0.0);
    drop(fld);
    crate::progress::set(0.75);
    if out.is_empty() { return out; }
    let out = smooth_taubin(&out, 8);
    let mut out = if out.len() / 18 > cible { crate::simplify::resample(&out, cible) } else { out };
    // contrôle : distance au bord (interpolée) de chaque sommet ; trop près -> poussé vers l'intérieur
    let Grille { lo: g0, nx, ny, nz, .. } = g;
    let at = |x: usize, y: usize, z: usize| dist[(z.min(nz - 1) * ny + y.min(ny - 1)) * nx + x.min(nx - 1)];
    let echant = |p: Vec3| -> f32 {
        let c = ((p - g0) / h).max(Vec3::ZERO);
        let (x, y, z) = (c.x.floor() as usize, c.y.floor() as usize, c.z.floor() as usize);
        let (fx, fy, fz) = (c.x.fract(), c.y.fract(), c.z.fract());
        let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
        l(l(l(at(x, y, z), at(x + 1, y, z), fx), l(at(x, y + 1, z), at(x + 1, y + 1, z), fx), fy),
          l(l(at(x, y, z + 1), at(x + 1, y, z + 1), fx), l(at(x, y + 1, z + 1), at(x + 1, y + 1, z + 1), fx), fy), fz)
    };
    let mut repousses = 0usize;
    for ch in out.chunks_exact_mut(6) {
        let mut p = Vec3::new(ch[0], ch[1], ch[2]);
        let mut bouge = false;
        for _ in 0..30 {
            let d = echant(p);
            if d >= epaisseur { break; }
            let gr = Vec3::new(echant(p + Vec3::X * h) - echant(p - Vec3::X * h), echant(p + Vec3::Y * h) - echant(p - Vec3::Y * h), echant(p + Vec3::Z * h) - echant(p - Vec3::Z * h));
            let gn = gr.normalize_or_zero();
            if gn == Vec3::ZERO { break; }
            p += gn * ((epaisseur - d) * 0.7 + 0.1 * h);
            bouge = true;
        }
        if bouge { repousses += 1; ch[0] = p.x; ch[1] = p.y; ch[2] = p.z; }
    }
    if std::env::var_os("CREUX_DIAG").is_some() {
        let mini = out.chunks_exact(6).map(|c| echant(Vec3::new(c[0], c[1], c[2]))).fold(f32::INFINITY, f32::min);
        eprintln!("  creux : {} sommets, {repousses} repoussés, paroi mini {mini:.3} mm (demandée {epaisseur}), voxel {h:.3}", out.len() / 6);
    }
    out
}
