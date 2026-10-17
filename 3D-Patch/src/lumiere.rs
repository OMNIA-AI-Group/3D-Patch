//! PROJECTEUR : position de la lumière de la scène, choisie à la souris sur un dôme autour de l'objet.
//! La lumière est directionnelle (comme un soleil) : seule sa direction compte ; à chaque ouverture
//! elle repart de l'angle de référence (azimut 328°, élévation −12°).

use glam::{Mat4, Vec3};
use std::f32::consts::TAU;

/// Direction à l'ouverture de l'appli (choisie par l'utilisateur) : azimut 328°, élévation −12°.
pub fn defaut() -> Vec3 {
    let (a, e) = (328f32.to_radians(), (-12f32).to_radians());
    Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())
}

/// Point du dôme visé par le rayon : intersection avant avec la sphère, sinon point de la sphère
/// le plus proche du rayon (bord de la silhouette).
pub fn point_sphere(orig: Vec3, dir: Vec3, c: Vec3, r: f32) -> Vec3 {
    let oc = orig - c;
    let b = oc.dot(dir);
    let disc = b * b - (oc.length_squared() - r * r);
    if disc >= 0.0 {
        let t = -b - disc.sqrt();
        let t = if t > 0.0 { t } else { -b + disc.sqrt() };
        return orig + dir * t;
    }
    let proche = orig + dir * (-b).max(0.0);
    c + (proche - c).normalize_or_zero() * r
}

fn proj(mvp: &Mat4, p: Vec3, rect: (f32, f32, f32, f32), ppp: f32) -> Option<egui::Pos2> {
    let c = *mvp * p.extend(1.0);
    if c.w <= 1e-5 { return None; }
    let n = c.truncate() / c.w;
    let (rx, ry, rw, rh) = rect;
    Some(egui::pos2((rx + (n.x * 0.5 + 0.5) * rw) / ppp, (ry + (0.5 - n.y * 0.5) * rh) / ppp))
}

/// Position du projecteur à l'écran (points egui).
pub fn position_ecran(mvp: &Mat4, c: Vec3, r: f32, d: Vec3, rect: (f32, f32, f32, f32), ppp: f32) -> Option<egui::Pos2> {
    proj(mvp, c + d * r, rect, ppp)
}

/// Dôme (équateur + deux méridiens passant par le projecteur), faisceau, projecteur, angles.
pub fn dessiner(p: &egui::Painter, mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32, c: Vec3, r: f32, d: Vec3, actif: bool) {
    let gris = egui::Color32::from_rgba_unmultiplied(255, 230, 140, 90);
    let cercle = |u: Vec3, v: Vec3, col: egui::Color32, w: f32| {
        let pts: Vec<egui::Pos2> = (0..=96).filter_map(|i| { let a = i as f32 / 96.0 * TAU; proj(mvp, c + (u * a.cos() + v * a.sin()) * r, rect, ppp) }).collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(w, col)));
    };
    cercle(Vec3::X, Vec3::Y, gris, 1.2);                       // équateur
    let az = Vec3::new(d.x, d.y, 0.0).normalize_or(Vec3::X);
    cercle(az, Vec3::Z, gris, 1.0);                             // méridien du projecteur
    cercle(az.cross(Vec3::Z), Vec3::Z, egui::Color32::from_rgba_unmultiplied(255, 230, 140, 45), 1.0);
    let jaune = egui::Color32::from_rgb(255, 215, 60);
    let (Some(pl), Some(pc)) = (proj(mvp, c + d * r, rect, ppp), proj(mvp, c, rect, ppp)) else { return };
    // ombre portée du projecteur sur l'équateur (repère d'azimut) + verticale
    if let Some(pe) = proj(mvp, c + az * r, rect, ppp) {
        p.line_segment([pe, pl], egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 230, 140, 120)));
        p.circle_filled(pe, 2.5, jaune);
    }
    // faisceau (pointillés) vers le centre
    let n = 14;
    for i in 0..n {
        if i % 2 == 1 { continue; }
        let a = pl + (pc - pl) * (i as f32 / n as f32);
        let b = pl + (pc - pl) * ((i + 1) as f32 / n as f32);
        p.line_segment([a, b], egui::Stroke::new(1.6, jaune));
    }
    // projecteur : orienté vers le centre
    let dir2 = (pc - pl).normalized();
    let dir2 = if dir2.x.is_finite() { dir2 } else { egui::vec2(0.0, 1.0) };
    p.circle_filled(pl, if actif { 22.0 } else { 19.0 }, egui::Color32::from_rgba_unmultiplied(255, 215, 60, 45)); // halo
    projecteur(p, pl, dir2, if actif { 20.0 } else { 17.0 }, jaune);
    // angles
    let elev = d.z.clamp(-1.0, 1.0).asin().to_degrees();
    let azim = (d.y.atan2(d.x).to_degrees() + 360.0) % 360.0;
    let txt = format!("Azimut {azim:.0}°  ·  Élévation {elev:.0}°");
    let font = egui::FontId::proportional(13.0);
    let gal = p.layout_no_wrap(txt, font, egui::Color32::from_gray(240));
    let vue = egui::Rect::from_min_size(egui::pos2(rect.0 / ppp, rect.1 / ppp), egui::vec2(rect.2 / ppp, rect.3 / ppp)).shrink(8.0);
    let mut pos = pl + egui::vec2(18.0, -28.0);
    pos.x = pos.x.clamp(vue.left(), (vue.right() - gal.size().x).max(vue.left()));
    pos.y = pos.y.clamp(vue.top(), (vue.bottom() - gal.size().y).max(vue.top()));
    p.rect_filled(egui::Rect::from_min_size(pos, gal.size()).expand2(egui::vec2(6.0, 3.0)), 3.0, egui::Color32::from_rgba_unmultiplied(28, 28, 32, 225));
    p.galley(pos, gal, egui::Color32::from_gray(240));
}

/// Symbole projecteur (corps + lentille + rayons) centré en `c`, orienté selon `dir`, taille `s`.
pub fn projecteur(p: &egui::Painter, c: egui::Pos2, dir: egui::Vec2, s: f32, col: egui::Color32) {
    let perp = egui::vec2(-dir.y, dir.x);
    let corps = |t: f32, w: f32| c + dir * (t * s) + perp * (w * s);
    let noir = egui::Color32::from_rgb(25, 25, 30);
    // corps (trapèze) : arrière étroit, avant large
    p.add(egui::Shape::convex_polygon(vec![corps(-0.9, -0.38), corps(-0.9, 0.38), corps(0.25, 0.6), corps(0.25, -0.6)], egui::Color32::from_rgb(200, 205, 215), egui::Stroke::new(1.4, noir)));
    // lentille lumineuse
    let l0 = corps(0.25, -0.62); let l1 = corps(0.25, 0.62);
    p.line_segment([l0, l1], egui::Stroke::new(s * 0.22, col));
    // rayons
    for k in [-0.55f32, 0.0, 0.55] {
        let a = corps(0.45, k * 0.9);
        let b = corps(1.15, k * 1.6);
        p.line_segment([a, b], egui::Stroke::new(1.6, col));
    }
    // pied (étrier)
    p.line_segment([corps(-0.3, -0.75), corps(-0.3, 0.75)], egui::Stroke::new(1.4, egui::Color32::from_gray(150)));
}
