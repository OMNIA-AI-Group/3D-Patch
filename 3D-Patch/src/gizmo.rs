//! Outil de rotation façon PrusaSlicer : trois anneaux (X rouge, Y vert, Z bleu) autour de l'objet,
//! chacun avec sa poignée (cube + deux cônes). Clic droit sur l'objet = afficher l'outil ;
//! glisser un anneau ou sa poignée au clic gauche = tourner l'objet autour de cet axe.
//! Pendant la rotation : seul l'anneau actif reste, avec un cercle blanc gradué et l'angle.
//! Tout le dessin passe par egui (projection des points 3D), la géométrie n'est modifiée
//! qu'au relâchement (aperçu par matrice de modèle pendant le geste).

use glam::{Mat3, Mat4, Vec3};
use std::f32::consts::{PI, TAU};

pub struct RotDrag {
    pub axis: usize,
    /// angle cumulé (radians) depuis le début du geste
    pub angle: f32,
    /// dernier angle mesuré du curseur dans le plan de l'anneau
    prev: f32,
}

pub struct Gizmo {
    pub obj: usize,
    pub center: Vec3,
    pub radius: f32,
    pub hover: Option<usize>,
    pub drag: Option<RotDrag>,
}

const NOMS: [&str; 3] = ["X", "Y", "Z"];

/// (normale = axe de rotation, u, v) : u = repos de la poignée, v = tangente.
fn basis(axis: usize) -> (Vec3, Vec3, Vec3) {
    match axis { 0 => (Vec3::X, Vec3::Y, Vec3::Z), 1 => (Vec3::Y, Vec3::Z, Vec3::X), _ => (Vec3::Z, Vec3::X, Vec3::Y) }
}
fn col(axis: usize, hot: bool) -> egui::Color32 {
    match (axis, hot) {
        (0, false) => egui::Color32::from_rgb(205, 20, 20),
        (1, false) => egui::Color32::from_rgb(10, 175, 10),
        (_, false) => egui::Color32::from_rgb(20, 40, 215),
        (0, true) => egui::Color32::from_rgb(60, 225, 230),   // survol X : cyan
        (1, true) => egui::Color32::from_rgb(230, 70, 235),   // survol Y : magenta
        (_, true) => egui::Color32::from_rgb(240, 230, 60),   // survol Z : jaune
    }
}

/// Projection écran (points egui) + profondeur (w du clip).
fn proj(mvp: &Mat4, p: Vec3, rect: (f32, f32, f32, f32), ppp: f32) -> Option<(egui::Pos2, f32)> {
    let c = *mvp * p.extend(1.0);
    if c.w <= 1e-5 { return None; }
    let n = c.truncate() / c.w;
    let (rx, ry, rw, rh) = rect;
    Some((egui::pos2((rx + (n.x * 0.5 + 0.5) * rw) / ppp, (ry + (0.5 - n.y * 0.5) * rh) / ppp), c.w))
}

/// Magnétisme : à moins de 5° d'un multiple de 45°, l'angle s'y colle.
pub const SNAP_PAS: f32 = 45.0;
const SNAP_TOL: f32 = 5.0;
fn snap(a: f32) -> f32 {
    let d = a.to_degrees();
    let m = (d / SNAP_PAS).round() * SNAP_PAS;
    if (d - m).abs() <= SNAP_TOL { m.to_radians() } else { a }
}

fn wrap(a: f32) -> f32 { let mut a = a % TAU; if a > PI { a -= TAU; } if a < -PI { a += TAU; } a }

impl Gizmo {
    pub fn new(obj: usize, min: Vec3, max: Vec3) -> Gizmo {
        let mut g = Gizmo { obj, center: Vec3::ZERO, radius: 1.0, hover: None, drag: None };
        g.fit(min, max); g
    }
    /// Recale centre et rayon sur la boîte de l'objet (sphère englobante, un peu agrandie).
    pub fn fit(&mut self, min: Vec3, max: Vec3) {
        self.center = (min + max) * 0.5;
        self.radius = ((max - min) * 0.5).length().max(1e-3) * 1.08;
    }
    fn handle_angle(&self, axis: usize) -> f32 {
        self.drag.as_ref().filter(|d| d.axis == axis).map_or(0.0, |d| snap(d.angle))
    }
    /// Angle appliqué (avec magnétisme), en radians.
    pub fn angle(&self) -> f32 { self.drag.as_ref().map_or(0.0, |d| snap(d.angle)) }
    fn ring_point(&self, axis: usize, a: f32, k: f32) -> Vec3 {
        let (_, u, v) = basis(axis);
        self.center + (u * a.cos() + v * a.sin()) * self.radius * k
    }

    /// Angle du curseur dans le plan de l'anneau (rayon ∩ plan ; anneau vu par la tranche :
    /// angle mesuré à l'écran autour du centre projeté).
    pub fn cursor_angle(&self, axis: usize, orig: Vec3, dir: Vec3, mvp: &Mat4, cursor_pts: egui::Pos2, rect: (f32, f32, f32, f32), ppp: f32) -> f32 {
        let (n, u, v) = basis(axis);
        let dn = dir.dot(n);
        if dn.abs() > 0.08 {
            let t = (self.center - orig).dot(n) / dn;
            let p = orig + dir * t - self.center;
            return p.dot(v).atan2(p.dot(u));
        }
        // repli écran
        let (Some((c, _)), Some((pu, _)), Some((pv, _))) = (proj(mvp, self.center, rect, ppp), proj(mvp, self.center + u * self.radius, rect, ppp), proj(mvp, self.center + v * self.radius, rect, ppp)) else { return 0.0 };
        let (eu, ev, d) = (pu - c, pv - c, cursor_pts - c);
        // coordonnées du curseur dans la base (eu, ev) projetée (moindres carrés 2×2)
        let det = eu.x * ev.y - eu.y * ev.x;
        if det.abs() < 1e-6 { return d.y.atan2(d.x); }
        let a = (d.x * ev.y - d.y * ev.x) / det;
        let b = (eu.x * d.y - eu.y * d.x) / det;
        b.atan2(a)
    }

    /// Anneau (ou poignée) sous le curseur, en points egui.
    pub fn hit(&self, mvp: &Mat4, cursor_pts: egui::Pos2, rect: (f32, f32, f32, f32), ppp: f32) -> Option<usize> {
        // poignées d'abord (prioritaires : la poignée Y est posée sur le trajet de l'anneau X…)
        let mut best: Option<(usize, f32)> = None;
        for axis in 0..3 {
            if let Some((h, _)) = proj(mvp, self.ring_point(axis, self.handle_angle(axis), 1.0), rect, ppp) {
                let rpx = (self.handle_px(mvp, axis, rect, ppp) * 2.0).max(9.0);
                let d = (h - cursor_pts).length();
                if d < rpx && best.map_or(true, |b| d < b.1) { best = Some((axis, d)); }
            }
        }
        if best.is_some() { return best.map(|b| b.0); }
        for axis in 0..3 {
            // anneau
            let n = 96;
            let pts: Vec<egui::Pos2> = (0..=n).filter_map(|i| proj(mvp, self.ring_point(axis, i as f32 / n as f32 * TAU, 1.0), rect, ppp).map(|p| p.0)).collect();
            for w in pts.windows(2) {
                let d = seg_dist(cursor_pts, w[0], w[1]);
                if d < 7.0 && best.map_or(true, |b| d < b.1) { best = Some((axis, d)); }
            }
        }
        best.map(|b| b.0)
    }

    /// Taille écran approximative de la poignée (points).
    fn handle_px(&self, mvp: &Mat4, axis: usize, rect: (f32, f32, f32, f32), ppp: f32) -> f32 {
        let p = self.ring_point(axis, self.handle_angle(axis), 1.0);
        let s = self.radius * 0.035;
        match (proj(mvp, p, rect, ppp), proj(mvp, p + Vec3::Z * s, rect, ppp), proj(mvp, p + Vec3::X * s, rect, ppp)) {
            (Some((a, _)), Some((b, _)), Some((c, _))) => (b - a).length().max((c - a).length()),
            _ => 8.0,
        }
    }

    pub fn start(&mut self, axis: usize, a: f32) { self.drag = Some(RotDrag { axis, angle: 0.0, prev: a }); }
    pub fn update(&mut self, a: f32) {
        if let Some(d) = self.drag.as_mut() { d.angle += wrap(a - d.prev); d.prev = a; }
    }
    /// Rotation en cours (autour du centre), à appliquer en aperçu puis à cuire.
    pub fn matrix(&self) -> Option<Mat4> {
        let d = self.drag.as_ref()?;
        let (n, _, _) = basis(d.axis);
        Some(Mat4::from_translation(self.center) * Mat4::from_axis_angle(n, snap(d.angle)) * Mat4::from_translation(-self.center))
    }
    pub fn rot3(&self) -> Option<Mat3> {
        let d = self.drag.as_ref()?;
        Some(Mat3::from_axis_angle(basis(d.axis).0, snap(d.angle)))
    }

    pub fn draw(&self, p: &egui::Painter, mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32, eye: Vec3) {
        let actif = self.drag.as_ref().map(|d| d.axis);
        for axis in 0..3 {
            if actif.is_some() && actif != Some(axis) { continue; }
            let hot = self.hover == Some(axis) || actif == Some(axis);
            let c = col(axis, hot);
            // anneau
            let n = 128;
            let pts: Vec<egui::Pos2> = (0..=n).filter_map(|i| proj(mvp, self.ring_point(axis, i as f32 / n as f32 * TAU, 1.0), rect, ppp).map(|q| q.0)).collect();
            p.add(egui::Shape::line(pts, egui::Stroke::new(if hot { 2.6 } else { 1.8 }, c)));
            if actif == Some(axis) { self.draw_dial(p, mvp, rect, ppp, axis); }
            self.draw_handle(p, mvp, rect, ppp, eye, axis, c);
        }
        // traits du centre vers les poignées (repos), comme PrusaSlicer
        if actif.is_none() {
            if let Some((c0, _)) = proj(mvp, self.center, rect, ppp) {
                for axis in 0..3 {
                    if let Some((h, _)) = proj(mvp, self.ring_point(axis, 0.0, 1.0), rect, ppp) {
                        p.line_segment([c0, h], egui::Stroke::new(1.0, col(axis, self.hover == Some(axis))));
                    }
                }
            }
        }
    }

    /// Cercle blanc gradué + rayons + angle affiché (pendant la rotation).
    fn draw_dial(&self, p: &egui::Painter, mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32, axis: usize) {
        let w = egui::Color32::WHITE;
        let st = egui::Stroke::new(1.5, w);
        let pr = |a: f32, k: f32| proj(mvp, self.ring_point(axis, a, k), rect, ppp).map(|q| q.0);
        let n = 128;
        let cercle: Vec<egui::Pos2> = (0..=n).filter_map(|i| pr(i as f32 / n as f32 * TAU, 0.80)).collect();
        p.add(egui::Shape::line(cercle, st));
        for i in 0..72 { // graduation tous les 5°, plus longue tous les 45°
            let a = i as f32 * TAU / 72.0;
            let k1 = if i % 9 == 0 { 0.90 } else { 0.85 };
            if let (Some(a0), Some(a1)) = (pr(a, 0.81), pr(a, k1)) { p.line_segment([a0, a1], egui::Stroke::new(1.2, w)); }
        }
        for i in 0..8 { // rayons intérieurs
            let a = i as f32 * TAU / 8.0;
            if let (Some(a0), Some(a1)) = (pr(a, 0.30), pr(a, 0.55)) { p.line_segment([a0, a1], st); }
        }
        let ang = self.handle_angle(axis);
        if let (Some(c), Some(h)) = (pr(0.0, 0.0), pr(ang, 1.0)) {
            p.line_segment([c, h], st);
            let deg = (ang.to_degrees() % 360.0 + 360.0) % 360.0;
            let txt = format!("{} : {}°", NOMS[axis], format!("{deg:.2}").replace('.', ","));
            let font = egui::FontId::proportional(14.0);
            let gal = p.layout_no_wrap(txt, font, egui::Color32::from_gray(235));
            // vers l'extérieur de l'anneau, au-delà de la poignée, sans sortir de la vue
            let dir = (h - c).normalized();
            let dir = if dir.x.is_finite() { dir } else { egui::vec2(1.0, 0.0) };
            let sz = gal.size();
            let centre = h + dir * (self.handle_px(mvp, axis, rect, ppp) * 2.6 + 12.0 + sz.x * 0.5 * dir.x.abs() + sz.y * 0.5 * dir.y.abs());
            let vue = egui::Rect::from_min_size(egui::pos2(rect.0 / ppp, rect.1 / ppp), egui::vec2(rect.2 / ppp, rect.3 / ppp)).shrink(10.0);
            let mut pos = centre - sz * 0.5;
            pos.x = pos.x.clamp(vue.left(), (vue.right() - sz.x).max(vue.left()));
            pos.y = pos.y.clamp(vue.top(), (vue.bottom() - sz.y).max(vue.top()));
            let r = egui::Rect::from_min_size(pos, sz).expand2(egui::vec2(7.0, 4.0));
            p.rect_filled(r, egui::Rounding::same(3.0), egui::Color32::from_rgba_unmultiplied(28, 28, 32, 230));
            p.galley(pos, gal, egui::Color32::from_gray(235));
        }
    }

    /// Poignée 3D : cube + deux cônes dans la direction de la tangente, triangles projetés et
    /// triés par profondeur (peintre), ombrés selon la direction de la vue.
    fn draw_handle(&self, p: &egui::Painter, mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32, eye: Vec3, axis: usize, c: egui::Color32) {
        let (n, u, v) = basis(axis);
        let a = self.handle_angle(axis);
        let radial = u * a.cos() + v * a.sin();
        let tang = n.cross(radial);
        let pos = self.center + radial * self.radius;
        let s = self.radius * 0.0275; // demi-côté du cube
        let mut tris: Vec<[Vec3; 3]> = Vec::new();
        // cube orienté (radial, tangente, axe)
        let (ex, ey, ez) = (radial * s, tang * s, n * s);
        let corner = |i: u32| pos + if i & 1 != 0 { ex } else { -ex } + if i & 2 != 0 { ey } else { -ey } + if i & 4 != 0 { ez } else { -ez };
        for (a0, b0, c0, d0) in [(0, 2, 6, 4), (1, 5, 7, 3), (0, 4, 5, 1), (2, 3, 7, 6), (0, 1, 3, 2), (4, 6, 7, 5)] {
            tris.push([corner(a0), corner(b0), corner(c0)]);
            tris.push([corner(a0), corner(c0), corner(d0)]);
        }
        // deux cônes le long de la tangente
        for sg in [1.0f32, -1.0] {
            let base = pos + tang * (s * 1.25 * sg);
            let apex = pos + tang * (s * 4.2 * sg);
            let rb = s * 0.95;
            let m = 20;
            for i in 0..m {
                let (t0, t1) = (i as f32 / m as f32 * TAU, (i + 1) as f32 / m as f32 * TAU);
                let q0 = base + (radial * t0.cos() + n * t0.sin()) * rb;
                let q1 = base + (radial * t1.cos() + n * t1.sin()) * rb;
                tris.push([q0, q1, apex]);
                tris.push([q1, q0, base]);
            }
        }
        let l = (eye - pos).normalize_or_zero();
        let lum = (l + Vec3::Z * 0.6).normalize_or_zero();
        let mut out: Vec<(f32, [egui::Pos2; 3], f32)> = Vec::new();
        for t in &tris {
            let (Some(a), Some(b), Some(cc)) = (proj(mvp, t[0], rect, ppp), proj(mvp, t[1], rect, ppp), proj(mvp, t[2], rect, ppp)) else { continue };
            // triangle vu par la tranche (vue de face sur l'anneau) : aplati à l'écran ; egui
            // l'étirerait en un immense triangle qui recouvre toute la vue -> ignoré (invisible)
            let aire = ((b.0 - a.0).x * (cc.0 - a.0).y - (b.0 - a.0).y * (cc.0 - a.0).x).abs() * 0.5;
            if aire < 0.05 || !aire.is_finite() { continue; }
            let nn = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            let sh = 0.35 + 0.65 * nn.dot(lum).abs();
            out.push(((a.1 + b.1 + cc.1) / 3.0, [a.0, b.0, cc.0], sh));
        }
        out.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
        for (_, q, sh) in out {
            let f = |x: u8| (x as f32 * sh).min(255.0) as u8;
            let cc = egui::Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()));
            p.add(egui::Shape::convex_polygon(q.to_vec(), cc, egui::Stroke::NONE));
        }
    }
}

fn seg_dist(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    let t = if l2 > 0.0 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t - p).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    // vue 3D en perspective, caméra en biais au-dessus
    fn cam() -> (Mat4, (f32, f32, f32, f32)) {
        let v = Mat4::look_at_rh(Vec3::new(60.0, -80.0, 50.0), Vec3::ZERO, Vec3::Z);
        let p = Mat4::perspective_rh_gl(0.8, 1.5, 0.1, 1000.0);
        (p * v, (0.0, 0.0, 1200.0, 800.0))
    }
    fn ray(mvp: &Mat4, rect: (f32, f32, f32, f32), s: egui::Pos2) -> (Vec3, Vec3) {
        let nx = s.x / rect.2 * 2.0 - 1.0; let ny = 1.0 - s.y / rect.3 * 2.0;
        let inv = mvp.inverse();
        let a = inv * glam::Vec4::new(nx, ny, -1.0, 1.0); let b = inv * glam::Vec4::new(nx, ny, 1.0, 1.0);
        let (a, b) = (a.truncate() / a.w, b.truncate() / b.w);
        (a, (b - a).normalize())
    }
    #[test]
    fn poignees_et_anneaux_detectes() {
        let (mvp, rect) = cam();
        let g = Gizmo::new(0, Vec3::splat(-10.0), Vec3::splat(10.0));
        for axis in 0..3 {
            let (h, _) = proj(&mvp, g.ring_point(axis, 0.0, 1.0), rect, 1.0).unwrap();
            assert_eq!(g.hit(&mvp, h, rect, 1.0), Some(axis), "poignée {axis}");
            let (q, _) = proj(&mvp, g.ring_point(axis, 2.2, 1.0), rect, 1.0).unwrap();
            assert_eq!(g.hit(&mvp, q, rect, 1.0), Some(axis), "anneau {axis}");
        }
        let (c, _) = proj(&mvp, g.center, rect, 1.0).unwrap();
        assert_eq!(g.hit(&mvp, c, rect, 1.0), None, "centre : rien");
    }
    #[test]
    fn angle_du_curseur_et_rotation() {
        let (mvp, rect) = cam();
        let mut g = Gizmo::new(0, Vec3::splat(-10.0), Vec3::splat(10.0));
        for axis in 0..3 {
            for deg in [0.0f32, 40.0, 135.0, -100.0] {
                let (s, _) = proj(&mvp, g.ring_point(axis, deg.to_radians(), 1.0), rect, 1.0).unwrap();
                let (o, d) = ray(&mvp, rect, s);
                let a = g.cursor_angle(axis, o, d, &mvp, s, rect, 1.0);
                assert!(wrap(a - deg.to_radians()).abs() < 0.02, "axe {axis} {deg}° -> {}", a.to_degrees());
            }
        }
        // geste : de 10° à 350° en passant par 180° = +340° cumulés (pas de saut)
        g.start(2, 10f32.to_radians());
        for d in (20..=350).step_by(10) { g.update((d as f32).to_radians()); }
        assert!((g.drag.as_ref().unwrap().angle.to_degrees() - 340.0).abs() < 0.01);
        let m = g.rot3().unwrap();
        assert!((m * Vec3::X - Vec3::new(340f32.to_radians().cos(), 340f32.to_radians().sin(), 0.0)).length() < 1e-4);
        // magnétisme : 87° -> 90°, 93° -> 90°, 84° reste 84°, 318° -> 315°
        for (brut, attendu) in [(87.0f32, 90.0f32), (93.0, 90.0), (84.0, 84.0), (318.0, 315.0), (-2.0, 0.0)] {
            g.start(2, 0.0); g.drag.as_mut().unwrap().angle = brut.to_radians();
            assert!((g.angle().to_degrees() - attendu).abs() < 1e-3, "{brut} -> {}", g.angle().to_degrees());
        }
    }
}
