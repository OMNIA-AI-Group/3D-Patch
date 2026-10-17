//! Calculs voxels sur GPU (shaders de calcul OpenGL 4.3+) : mise en voxels de la surface,
//! épaississement et remplissage de l'extérieur — la partie lourde du Lissage interne et de
//! l'Enveloppe unique. Les voxels sont des BITS (mémoire ÷32).
//!
//! Le contexte GL appartient au fil principal : un calcul en arrière-plan envoie une demande
//! (`exterior`) et attend la réponse ; le fil principal l'exécute à l'image suivante (`serve`).
//! Toute erreur => None => l'appelant refait le calcul sur le processeur.

use glow::HasContext;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

/// Demande : bits « extérieur » d'une grille nx×ny×nz (indice (z*ny+y)*nx+x).
pub struct Req {
    pub tris: Vec<f32>,      // 9 flottants par triangle
    pub org: [f32; 3],       // origine de la grille
    pub h: f32,              // taille d'un voxel
    pub off: i32,            // décalage entier ajouté à floor((p-org)/h)
    pub dims: [i32; 3],
    pub cube: bool,          // marquage dilaté d'1 voxel en cube (Lissage interne)
    pub grow6: u32,          // nombre de dilatations 6-voisins après marquage (Enveloppe)
}
type Job = (Req, Sender<Option<Vec<u32>>>);

static TX: Mutex<Option<Sender<Job>>> = Mutex::new(None);

/// Appelé depuis un calcul en arrière-plan. None si GPU désactivé/indisponible ou en échec.
pub fn exterior(req: Req) -> Option<Vec<u32>> {
    if !crate::perf::use_gpu() { return None; } // case cochée ET GPU plus rapide que le processeur
    let tx = TX.lock().ok()?.clone()?;
    let (rtx, rrx) = channel();
    tx.send((req, rtx)).ok()?;
    rrx.recv().ok()?
}

/// Lecture d'un bit de la grille renvoyée.
#[inline] pub fn bit(bits: &[u32], i: usize) -> bool { bits[i >> 5] >> (i & 31) & 1 == 1 }

const MARK: &str = r#"#version 430
layout(local_size_x = 64) in;
layout(std430, binding = 0) readonly buffer T { float tri[]; };
layout(std430, binding = 1) buffer W { uint wall[]; };
uniform vec3 org; uniform float h; uniform int off; uniform ivec3 dims; uniform uint ntri; uniform int cube;
void setb(ivec3 c) {
    if (any(lessThan(c, ivec3(0))) || any(greaterThanEqual(c, dims))) return;
    uint i = uint((c.z * dims.y + c.y) * dims.x + c.x);
    atomicOr(wall[i >> 5], 1u << (i & 31u));
}
void main() {
    uint t = gl_GlobalInvocationID.x; if (t >= ntri) return;
    vec3 a = vec3(tri[t*9u], tri[t*9u+1u], tri[t*9u+2u]);
    vec3 b = vec3(tri[t*9u+3u], tri[t*9u+4u], tri[t*9u+5u]);
    vec3 c = vec3(tri[t*9u+6u], tri[t*9u+7u], tri[t*9u+8u]);
    float m = max(length(b - a), max(length(c - b), length(a - c)));
    int n = clamp(int(ceil(m / (h * 0.5))), 1, 64);   // triangles pré-découpés côté CPU : ≤ 32 voxels
    for (int i = 0; i <= n; i++) for (int j = 0; j <= n - i; j++) {
        float u = float(i) / float(n), v = float(j) / float(n);
        vec3 p = a * (1.0 - u - v) + b * u + c * v;
        ivec3 cc = ivec3(floor((p - org) / h)) + ivec3(off);
        if (cube == 1) { for (int dz = -1; dz <= 1; dz++) for (int dy = -1; dy <= 1; dy++) for (int dx = -1; dx <= 1; dx++) setb(cc + ivec3(dx, dy, dz)); }
        else setb(cc);
    }
}"#;

const DILATE: &str = r#"#version 430
layout(local_size_x = 8, local_size_y = 8, local_size_z = 4) in;
layout(std430, binding = 1) readonly buffer S { uint src[]; };
layout(std430, binding = 2) buffer D { uint dst[]; };
uniform ivec3 dims;
bool g(ivec3 c) {
    if (any(lessThan(c, ivec3(0))) || any(greaterThanEqual(c, dims))) return false;
    uint i = uint((c.z * dims.y + c.y) * dims.x + c.x);
    return ((src[i >> 5] >> (i & 31u)) & 1u) == 1u;
}
void main() {
    ivec3 c = ivec3(gl_GlobalInvocationID); if (any(greaterThanEqual(c, dims))) return;
    if (g(c) || g(c + ivec3(1,0,0)) || g(c - ivec3(1,0,0)) || g(c + ivec3(0,1,0)) || g(c - ivec3(0,1,0)) || g(c + ivec3(0,0,1)) || g(c - ivec3(0,0,1))) {
        uint i = uint((c.z * dims.y + c.y) * dims.x + c.x);
        atomicOr(dst[i >> 5], 1u << (i & 31u));
    }
}"#;

/// Remplissage de l'extérieur par balayages de lignes (6-connexité) : chaque invocation parcourt
/// une ligne de la grille le long de l'axe `ax`, dans les deux sens. Répété jusqu'à stabilité.
const SWEEP: &str = r#"#version 430
layout(local_size_x = 8, local_size_y = 8) in;
layout(std430, binding = 1) readonly buffer W { uint wall[]; };
layout(std430, binding = 3) buffer E { uint ext[]; };
layout(std430, binding = 4) buffer C { uint changed; };
uniform ivec3 dims; uniform int ax; uniform int init;
uint idx(ivec3 c) { return uint((c.z * dims.y + c.y) * dims.x + c.x); }
bool w(uint i) { return ((wall[i >> 5] >> (i & 31u)) & 1u) == 1u; }
bool e(uint i) { return ((ext[i >> 5] >> (i & 31u)) & 1u) == 1u; }
void setE(uint i) { uint old = atomicOr(ext[i >> 5], 1u << (i & 31u)); if ((old & (1u << (i & 31u))) == 0u) atomicOr(changed, 1u); }
void main() {
    ivec3 d = dims; int L = ax == 0 ? d.x : (ax == 1 ? d.y : d.z);
    ivec2 q = ivec2(gl_GlobalInvocationID.xy);
    ivec2 lim = ax == 0 ? d.yz : (ax == 1 ? d.xz : d.xy);
    if (any(greaterThanEqual(q, lim))) return;
    for (int pass = 0; pass < 2; pass++) {
        bool carry = false;
        for (int k = 0; k < L; k++) {
            int s = pass == 0 ? k : L - 1 - k;
            ivec3 c = ax == 0 ? ivec3(s, q.x, q.y) : (ax == 1 ? ivec3(q.x, s, q.y) : ivec3(q.x, q.y, s));
            uint i = idx(c);
            if (w(i)) { carry = false; continue; }
            // départ : les cases libres du bord de la grille sont à l'extérieur
            bool border = init == 1 && (any(equal(c, ivec3(0))) || any(equal(c, d - ivec3(1))));
            if (e(i) || border) { if (!e(i)) setE(i); carry = true; }
            else if (carry) setE(i);
        }
    }
}"#;

/// Découpe (en 4, par les milieux) tout triangle dont une arête dépasse `max_len`.
fn split_large(t: &[f32], max_len: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(t.len());
    let mut pile: Vec<[[f32; 3]; 3]> = t.chunks_exact(9).map(|c| [[c[0], c[1], c[2]], [c[3], c[4], c[5]], [c[6], c[7], c[8]]]).collect();
    let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    let m = |a: [f32; 3], b: [f32; 3]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5];
    while let Some([a, b, c]) = pile.pop() {
        if d(a, b).max(d(b, c)).max(d(c, a)) <= max_len || out.len() > 400_000_000 {
            for p in [a, b, c] { out.extend_from_slice(&p); }
        } else {
            let (ab, bc, ca) = (m(a, b), m(b, c), m(c, a));
            pile.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
    }
    out
}

pub struct Gpu { mark: glow::NativeProgram, dilate: glow::NativeProgram, sweep: glow::NativeProgram, pub name: String,
                 /// débit voxels mesuré, mémoire vidéo libre (Mo, si connue)
                 pub power: f64, pub mem_mb: Option<i32> } // power = débit voxels mesuré (Gvox/s)

unsafe fn compile(gl: &glow::Context, src: &str) -> Option<glow::NativeProgram> {
    let s = gl.create_shader(glow::COMPUTE_SHADER).ok()?;
    gl.shader_source(s, src); gl.compile_shader(s);
    if !gl.get_shader_compile_status(s) { eprintln!("GPU : shader refusé : {}", gl.get_shader_info_log(s)); return None; }
    let p = gl.create_program().ok()?;
    gl.attach_shader(p, s); gl.link_program(p);
    if !gl.get_program_link_status(p) { eprintln!("GPU : programme refusé : {}", gl.get_program_info_log(p)); return None; }
    Some(p)
}

impl Gpu {
    /// GPU utilisable ? (pilote matériel, OpenGL ≥ 4.3, shaders compilés) + branchement du canal.
    pub unsafe fn detect(gl: &glow::Context) -> Option<(Gpu, Receiver<Job>)> {
        let name = gl.get_parameter_string(glow::RENDERER);
        let low = name.to_ascii_lowercase();
        if ["llvmpipe", "softpipe", "swrast", "software"].iter().any(|k| low.contains(k)) { return None; }
        let v = gl.version();
        if (v.major, v.minor) < (4, 3) || v.is_embedded { return None; }
        let mut g = Gpu { mark: compile(gl, MARK)?, dilate: compile(gl, DILATE)?, sweep: compile(gl, SWEEP)?, name, power: 1.0, mem_mb: None };
        g.calibrate(gl);
        let (tx, rx) = channel();
        *TX.lock().ok()? = Some(tx);
        Some((g, rx))
    }

    /// Mesure la puissance (dilatations sur une grille 256³) et la mémoire vidéo, puis fixe la
    /// finesse des voxels : plus la carte est puissante, plus les voxels sont fins (même durée).
    unsafe fn calibrate(&mut self, gl: &glow::Context) {
        let d = [256i32, 256, 256];
        let words = (256usize * 256 * 256) / 32;
        let mk = || -> Option<glow::NativeBuffer> {
            let b = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(b));
            gl.buffer_data_size(glow::SHADER_STORAGE_BUFFER, (words * 4) as i32, glow::DYNAMIC_COPY);
            Some(b)
        };
        let (Some(b1), Some(b2)) = (mk(), mk()) else { return };
        gl.use_program(Some(self.dilate));
        let l = gl.get_uniform_location(self.dilate, "dims"); gl.uniform_3_i32(l.as_ref(), d[0], d[1], d[2]);
        let run = |n: u32| { for _ in 0..n {
            gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 1, Some(b1));
            gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 2, Some(b2));
            gl.dispatch_compute(32, 32, 64);
            gl.memory_barrier(glow::SHADER_STORAGE_BARRIER_BIT);
        } gl.finish(); };
        run(6); // chauffe (montée en fréquence de la carte)
        let mut score = 0.0f64; // milliards de voxels / s : meilleur de 3 essais
        for _ in 0..3 {
            let t0 = std::time::Instant::now();
            run(8);
            let ms = (t0.elapsed().as_secs_f64() * 1000.0).max(0.01);
            score = score.max(8.0 * 16.777216 / ms);
        }
        for b in [b1, b2] { gl.delete_buffer(b); }
        gl.use_program(None);
        // mémoire vidéo libre : AMD (ATI_meminfo) ou NVIDIA (NVX_gpu_memory_info), en Ko
        let ext = gl.supported_extensions();
        let mut v = [0i32; 4];
        self.mem_mb = if ext.contains("GL_ATI_meminfo") { gl.get_parameter_i32_slice(0x87FC, &mut v); Some(v[0] / 1024) }
                      else if ext.contains("GL_NVX_gpu_memory_info") { Some(gl.get_parameter_i32(0x9049) / 1024) } else { None };
        while gl.get_error() != glow::NO_ERROR {}
        self.power = score;
        crate::perf::set_gpu(self.name.clone(), score, self.mem_mb);
        eprintln!("GPU : {:.2} Gvox/s, mémoire libre {:?} Mo", score, self.mem_mb);
    }

    /// Exécute les demandes en attente (fil principal, contexte GL courant).
    pub unsafe fn serve(&self, gl: &glow::Context, rx: &Receiver<Job>) {
        while let Ok((req, reply)) = rx.try_recv() {
            let r = self.run(gl, &req);
            let _ = reply.send(r);
        }
    }

    unsafe fn run(&self, gl: &glow::Context, r: &Req) -> Option<Vec<u32>> {
        let t0 = std::time::Instant::now();
        let n = r.dims[0] as usize * r.dims[1] as usize * r.dims[2] as usize;
        let words = (n + 31) / 32;
        // grands triangles découpés : chaque invocation GPU reste courte (sinon le pilote coupe le
        // calcul trop long -> « context lost » -> l'application est tuée)
        let tris = split_large(&r.tris, r.h * 16.0);
        let ntri = (tris.len() / 9) as u32;
        if ntri == 0 || n == 0 { return None; }
        let buf = |data: Option<&[u8]>, size: usize| -> Option<glow::NativeBuffer> {
            let b = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(b));
            match data { Some(d) => gl.buffer_data_u8_slice(glow::SHADER_STORAGE_BUFFER, d, glow::STATIC_DRAW),
                         None => gl.buffer_data_u8_slice(glow::SHADER_STORAGE_BUFFER, &vec![0u8; size], glow::DYNAMIC_COPY) }
            Some(b)
        };
        let tri_bytes = core::slice::from_raw_parts(tris.as_ptr() as *const u8, tris.len() * 4);
        let bt = buf(Some(tri_bytes), 0)?;
        let mut bw = buf(None, words * 4)?;
        let be = buf(None, words * 4)?;
        let bc = buf(None, 4)?;
        let set3 = |p: glow::NativeProgram, name: &str, v: [i32; 3]| { let l = gl.get_uniform_location(p, name); gl.uniform_3_i32(l.as_ref(), v[0], v[1], v[2]); };
        // 1) marquage de la surface
        gl.use_program(Some(self.mark));
        gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 0, Some(bt));
        gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 1, Some(bw));
        let l = gl.get_uniform_location(self.mark, "org"); gl.uniform_3_f32(l.as_ref(), r.org[0], r.org[1], r.org[2]);
        let l = gl.get_uniform_location(self.mark, "h"); gl.uniform_1_f32(l.as_ref(), r.h);
        let l = gl.get_uniform_location(self.mark, "off"); gl.uniform_1_i32(l.as_ref(), r.off);
        let l = gl.get_uniform_location(self.mark, "ntri"); gl.uniform_1_u32(l.as_ref(), ntri);
        let l = gl.get_uniform_location(self.mark, "cube"); gl.uniform_1_i32(l.as_ref(), r.cube as i32);
        set3(self.mark, "dims", r.dims);
        gl.dispatch_compute((ntri + 63) / 64, 1, 1);
        gl.memory_barrier(glow::SHADER_STORAGE_BARRIER_BIT);
        // 2) épaississement 6-voisins
        let g3 = |d: [i32; 3], l: [i32; 3]| [((d[0] + l[0] - 1) / l[0]) as u32, ((d[1] + l[1] - 1) / l[1]) as u32, ((d[2] + l[2] - 1) / l[2]) as u32];
        for _ in 0..r.grow6 {
            let bd = buf(None, words * 4)?;
            gl.use_program(Some(self.dilate));
            gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 1, Some(bw));
            gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 2, Some(bd));
            set3(self.dilate, "dims", r.dims);
            let g = g3(r.dims, [8, 8, 4]);
            gl.dispatch_compute(g[0], g[1], g[2]);
            gl.memory_barrier(glow::SHADER_STORAGE_BARRIER_BIT);
            gl.delete_buffer(bw);
            bw = bd;
        }
        // 3) extérieur : balayages X, Y, Z répétés jusqu'à ce que plus rien ne change
        gl.use_program(Some(self.sweep));
        gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 1, Some(bw));
        gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 3, Some(be));
        gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 4, Some(bc));
        set3(self.sweep, "dims", r.dims);
        let lax = gl.get_uniform_location(self.sweep, "ax");
        let linit = gl.get_uniform_location(self.sweep, "init");
        let d = r.dims;
        let mut rounds = 0;
        loop {
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(bc));
            gl.buffer_sub_data_u8_slice(glow::SHADER_STORAGE_BUFFER, 0, &[0u8; 4]);
            for ax in 0..3 {
                gl.uniform_1_i32(lax.as_ref(), ax);
                gl.uniform_1_i32(linit.as_ref(), (rounds == 0) as i32);
                let lim = match ax { 0 => [d[1], d[2]], 1 => [d[0], d[2]], _ => [d[0], d[1]] };
                gl.dispatch_compute(((lim[0] + 7) / 8) as u32, ((lim[1] + 7) / 8) as u32, 1);
                gl.memory_barrier(glow::SHADER_STORAGE_BARRIER_BIT | glow::BUFFER_UPDATE_BARRIER_BIT);
            }
            let mut c = [0u8; 4];
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(bc));
            gl.get_buffer_sub_data(glow::SHADER_STORAGE_BUFFER, 0, &mut c);
            rounds += 1;
            if u32::from_ne_bytes(c) == 0 || rounds > 500 { break; }
        }
        let mut out = vec![0u32; words];
        gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(be));
        let ob = core::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, words * 4);
        gl.get_buffer_sub_data(glow::SHADER_STORAGE_BUFFER, 0, ob);
        for b in [bt, bw, be, bc] { gl.delete_buffer(b); }
        gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, None);
        gl.use_program(None);
        if gl.get_error() != glow::NO_ERROR { eprintln!("GPU : erreur GL, retour au processeur"); return None; }
        eprintln!("GPU : grille {}×{}×{} — {} balayage(s) — {:.0} ms", d[0], d[1], d[2], rounds, t0.elapsed().as_secs_f32() * 1000.0);
        Some(out)
    }
}
