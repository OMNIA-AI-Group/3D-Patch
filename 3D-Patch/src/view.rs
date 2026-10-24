//! Vues multiples façon MoI : 3D (perspective orbitale) + Dessus / Face / Droite (orthographiques).

use glam::{Mat4, Vec2, Vec3};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewKind { Top, ThreeD, Front, Right }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode { Quad, Single(ViewKind) }

/// Caméra orbitale (vue 3D). Z vers le haut.
#[derive(Clone, Copy)]
pub struct Orbit { pub yaw: f32, pub pitch: f32, pub dist: f32, pub target: Vec3 }
impl Orbit {
    pub fn eye(&self) -> Vec3 {
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        self.target + Vec3::new(self.dist * cp * sy, self.dist * cp * cy, self.dist * sp)
    }
    pub fn mvp(&self, aspect: f32) -> (Mat4, Vec3) {
        // near/far resserrés : le ratio far/near doit rester modéré pour la précision du depth buffer.
        let near = (self.dist * 0.1).max(0.05);
        let far = self.dist * 12.0 + 600.0;
        let proj = Mat4::perspective_rh_gl(45f32.to_radians(), aspect, near, far);
        let eye = self.eye();
        (proj * Mat4::look_at_rh(eye, self.target, Vec3::Z), eye)
    }
}

/// État d'une vue orthographique : décalage (pan) dans le plan + demi-hauteur (zoom).
#[derive(Clone, Copy)]
pub struct Ortho { pub pan: Vec2, pub scale: f32 }

/// Direction œil→cible (edir) et « haut » de chaque vue ortho.
pub fn ortho_basis(k: ViewKind) -> (Vec3, Vec3) {
    match k {
        ViewKind::Top => (Vec3::Z, Vec3::Y),               // au-dessus, regarde vers -Z
        ViewKind::Front => (Vec3::new(0.0, -1.0, 0.0), Vec3::Z), // devant (-Y), regarde +Y
        ViewKind::Right => (Vec3::X, Vec3::Z),             // à droite (+X), regarde -X
        ViewKind::ThreeD => (Vec3::Z, Vec3::Y),
    }
}

impl Ortho {
    pub fn right_up(k: ViewKind) -> (Vec3, Vec3) {
        let (edir, up) = ortho_basis(k);
        let right = (-edir).cross(up).normalize();
        (right, up)
    }
    pub fn mvp(&self, k: ViewKind, base: Vec3, aspect: f32) -> (Mat4, Vec3) {
        let (edir, up) = ortho_basis(k);
        let (right, _) = Ortho::right_up(k);
        let focus = base + right * self.pan.x + up * self.pan.y;
        let eye = focus + edir * (self.scale * 50.0 + 1000.0);
        let (hw, hh) = (self.scale * aspect, self.scale);
        let proj = Mat4::orthographic_rh_gl(-hw, hw, -hh, hh, -1.0e5, 1.0e5);
        (proj * Mat4::look_at_rh(eye, focus, up), eye)
    }
}

pub fn oidx(k: ViewKind) -> usize {
    match k { ViewKind::Top => 0, ViewKind::Front => 1, ViewKind::Right => 2, ViewKind::ThreeD => 0 }
}

pub fn kind_name(k: ViewKind) -> &'static str {
    match k { ViewKind::Top => "Dessus", ViewKind::ThreeD => "3D", ViewKind::Front => "Face", ViewKind::Right => "Droite" }
}

/// Rectangle écran (origine en haut-gauche) d'une vue : (x, y, w, h) en pixels.
/// `bar` = hauteur réservée en bas, `right` = largeur réservée à droite (panneau d'outils).
/// Hauteur réservée en haut (bandeau du mode Texture), en pixels ; 0 sinon.
static HAUT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub fn regler_haut(px: f32) { HAUT.store(px.to_bits(), std::sync::atomic::Ordering::Relaxed); }
fn haut() -> f32 { f32::from_bits(HAUT.load(std::sync::atomic::Ordering::Relaxed)) }

pub fn screen_rect(mode: ViewMode, k: ViewKind, w: f32, h: f32, bar: f32, right: f32) -> Option<(f32, f32, f32, f32)> {
    let top = haut();
    let ch = (h - bar - top).max(1.0);
    let w = (w - right).max(1.0);
    match mode {
        ViewMode::Single(sk) => if sk == k { Some((0.0, top, w, ch)) } else { None },
        ViewMode::Quad => {
            let (hw, hh) = (w / 2.0, ch / 2.0);
            Some(match k {
                ViewKind::Top => (0.0, top, hw, hh),
                ViewKind::ThreeD => (hw, top, hw, hh),
                ViewKind::Front => (0.0, top + hh, hw, hh),
                ViewKind::Right => (hw, top + hh, hw, hh),
            })
        }
    }
}

/// Rectangle OpenGL (origine en bas-gauche) correspondant.
pub fn gl_rect(sr: (f32, f32, f32, f32), h: f32) -> (i32, i32, i32, i32) {
    let (x, y, rw, rh) = sr;
    (x as i32, (h - (y + rh)) as i32, rw as i32, rh as i32)
}

pub const ALL_VIEWS: [ViewKind; 4] = [ViewKind::Top, ViewKind::ThreeD, ViewKind::Front, ViewKind::Right];

/// Quelle vue se trouve sous le curseur (coordonnées écran, origine haut-gauche).
pub fn view_at(mode: ViewMode, w: f32, h: f32, bar: f32, right: f32, cx: f32, cy: f32) -> Option<ViewKind> {
    for k in ALL_VIEWS {
        if let Some((x, y, rw, rh)) = screen_rect(mode, k, w, h, bar, right) {
            if cx >= x && cx < x + rw && cy >= y && cy < y + rh { return Some(k); }
        }
    }
    None
}
