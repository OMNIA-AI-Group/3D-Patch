//! Profil de la machine, mesuré au lancement : l'appli s'ajuste à la configuration sur laquelle
//! elle tourne (cœurs, vitesse du processeur, mémoire vive, GPU éventuel), sans réglage manuel.
//!
//!  - moteur des calculs voxels : GPU seulement s'il est coché ET réellement plus rapide ;
//!  - finesse des voxels (Enveloppe, Lissage interne) proportionnelle à la racine cubique de la
//!    puissance du processeur (le volume de voxels suit la puissance => durée comparable), avec
//!    un bonus si le GPU fait la mise en voxels ; plafonnée par la mémoire disponible.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Mutex;

/// Débit PROCESSEUR de référence (milliards de voxels/s, mesure ci-dessous) qui correspond à la
/// finesse de base 300 / 384 (un portable 6 cœurs / 12 fils récent). Simple repère d'échelle :
/// chaque machine se place par rapport à lui, plus fine si plus puissante, plus grossière sinon.
const REF_CPU_GVOX: f64 = 0.36;
/// Bonus de finesse quand le GPU fait la mise en voxels : il retire environ un tiers de la durée
/// de l'Enveloppe (mesuré), soit ~+33 % de voxels pour la même durée => racine cubique ≈ 1,1.
const GPU_BONUS: f64 = 1.1;
const BASE_ENV: f64 = 300.0;
const BASE_LISS: f64 = 384.0;

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub threads: usize,
    pub cpu_gvox: f64,              // débit voxels du processeur (tous cœurs)
    pub ram_mb: Option<u64>,        // mémoire vive disponible
    pub gpu_name: Option<String>,
    pub gpu_gvox: Option<f64>,      // débit voxels du GPU
    pub vram_mb: Option<i32>,       // mémoire vidéo libre (si le pilote la donne)
}

static PROFILE: Mutex<Option<Profile>> = Mutex::new(None);
/// Case « Activer GPU » cochée par l'utilisateur.
pub static GPU_CHECKED: AtomicBool = AtomicBool::new(false);

pub fn profile() -> Profile { PROFILE.lock().ok().and_then(|p| p.clone()).unwrap_or_default() }

/// Mesure du processeur (≈0,1 s) : dilatation 6-voisins d'une grille 160³ sur tous les cœurs.
pub fn measure_cpu() {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let n = 160usize;
    let src: Vec<u8> = (0..n * n * n).map(|i| ((i * 2654435761) >> 28 & 1) as u8).collect();
    let mut dst = vec![0u8; n * n * n];
    let run = |src: &[u8], dst: &mut [u8]| {
        let slab = n * n;
        let per = ((n + threads - 1) / threads).max(1);
        std::thread::scope(|sc| {
            for (ci, part) in dst.chunks_mut(slab * per).enumerate() {
                sc.spawn(move || {
                    for (k, d) in part.iter_mut().enumerate() {
                        let id = ci * slab * per + k;
                        let (z, y, x) = (id / slab, (id % slab) / n, id % n);
                        let g = |xx: usize, yy: usize, zz: usize| src[(zz * n + yy) * n + xx];
                        let mut v = src[id];
                        if x > 0 { v |= g(x - 1, y, z); } if x + 1 < n { v |= g(x + 1, y, z); }
                        if y > 0 { v |= g(x, y - 1, z); } if y + 1 < n { v |= g(x, y + 1, z); }
                        if z > 0 { v |= g(x, y, z - 1); } if z + 1 < n { v |= g(x, y, z + 1); }
                        *d = v;
                    }
                });
            }
        });
    };
    run(&src, &mut dst); // chauffe
    let mut best = 0.0f64;
    for _ in 0..3 {
        let t0 = std::time::Instant::now();
        run(&src, &mut dst);
        best = best.max((n * n * n) as f64 / t0.elapsed().as_secs_f64().max(1e-6) / 1e9);
    }
    std::hint::black_box(&dst);
    let mut p = PROFILE.lock().unwrap();
    let mut pr = p.take().unwrap_or_default();
    pr.threads = threads; pr.cpu_gvox = best; pr.ram_mb = ram_available_mb();
    *p = Some(pr);
}

/// Enregistre la mesure du GPU (faite par gpu::Gpu::calibrate).
pub fn set_gpu(name: String, gvox: f64, vram_mb: Option<i32>) {
    let mut p = PROFILE.lock().unwrap();
    let mut pr = p.take().unwrap_or_default();
    pr.gpu_name = Some(name); pr.gpu_gvox = Some(gvox); pr.vram_mb = vram_mb;
    *p = Some(pr);
}

/// Le GPU est-il plus rapide que le processeur sur CETTE machine ? (marge de 20 %)
pub fn gpu_faster() -> bool { let p = profile(); p.gpu_gvox.map_or(false, |g| g > p.cpu_gvox * 1.2) }
/// Les calculs voxels passent-ils effectivement par le GPU ?
pub fn use_gpu() -> bool { GPU_CHECKED.load(Relaxed) && gpu_faster() }

/// Finesse (voxels sur le plus grand côté) selon le moteur effectif et la mémoire.
fn res(base: f64, max: f64, bytes_per_voxel: f64) -> usize {
    let p = profile();
    // la finesse suit surtout le PROCESSEUR (extraction, lissage, simplification restent dessus) ;
    // le GPU, s'il est utilisé, n'accélère que la mise en voxels => petit bonus
    let mut f = (p.cpu_gvox.max(1e-3) / REF_CPU_GVOX).cbrt();
    if use_gpu() { f *= GPU_BONUS; }
    let f = f.clamp(0.6, 1.7);
    let mut r = (base * f).clamp(base * 0.6, max);
    // mémoire : la grille ne doit pas dépasser ~20 % de la mémoire vive disponible
    if let Some(mb) = p.ram_mb {
        let cap = (mb as f64 * 1e6 * 0.20 / bytes_per_voxel).cbrt();
        r = r.min(cap.max(base * 0.6));
    }
    r as usize
}
/// Enveloppe : champ flottant + grilles booléennes ≈ 7 octets/voxel.
pub fn env_res() -> usize { res(BASE_ENV, 512.0, 7.0) }
/// Lissage interne : grille d'octets + marges ≈ 1,5 octet/voxel.
pub fn liss_res() -> usize { res(BASE_LISS, 640.0, 1.5) }

#[cfg(target_os = "linux")]
fn ram_available_mb() -> Option<u64> {
    let t = std::fs::read_to_string("/proc/meminfo").ok()?;
    let l = t.lines().find(|l| l.starts_with("MemAvailable:"))?;
    Some(l.split_whitespace().nth(1)?.parse::<u64>().ok()? / 1024)
}
#[cfg(windows)]
fn ram_available_mb() -> Option<u64> {
    #[repr(C)]
    struct MemStatus { len: u32, load: u32, total: u64, avail: u64, tpage: u64, apage: u64, tvirt: u64, avirt: u64, aext: u64 }
    #[link(name = "kernel32")]
    extern "system" { fn GlobalMemoryStatusEx(m: *mut MemStatus) -> i32; }
    let mut m = MemStatus { len: std::mem::size_of::<MemStatus>() as u32, load: 0, total: 0, avail: 0, tpage: 0, apage: 0, tvirt: 0, avirt: 0, aext: 0 };
    if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 { Some(m.avail / (1024 * 1024)) } else { None }
}
#[cfg(not(any(target_os = "linux", windows)))]
fn ram_available_mb() -> Option<u64> { None }
