//! Placement du motif sur l'objet : à chaque point (position, normale) on associe une ou plusieurs
//! coordonnées de carreau (u, v) pondérées. Toutes les longueurs sont en mm.

/// Façon d'envelopper l'objet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Vu d'un seul axe (comme un tampon) : toutes les faces reçoivent la même image.
    Plan,
    /// Chaque face prend l'axe le plus proche de sa normale (raccords nets aux arêtes).
    Cube,
    /// Comme Cube, mais les trois axes se fondent sur les zones arrondies (pas de raccord visible).
    Triplanaire,
    /// Enroulé autour d'un axe (cylindres, vases, poignées) ; les fonds reçoivent une projection à plat.
    Cylindre,
    /// Enroulé sur une sphère (longitude / latitude).
    Sphere,
}

pub const MODES: [Mode; 5] = [Mode::Triplanaire, Mode::Cube, Mode::Plan, Mode::Cylindre, Mode::Sphere];

impl Mode {
    pub fn cle(self) -> &'static str {
        match self { Mode::Plan => "p_plan", Mode::Cube => "p_cube", Mode::Triplanaire => "p_triplanaire", Mode::Cylindre => "p_cylindre", Mode::Sphere => "p_sphere" }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    pub mode: Mode,
    pub taille_u: f32,  // mm couverts par un carreau
    pub taille_v: f32,
    pub rotation: f32,  // degrés, dans le plan du motif
    pub decalage: [f32; 2], // en carreaux
    pub axe: [f32; 3],  // axe du Plan / du Cylindre (unitaire)
    pub centre: [f32; 3], // centre de l'enroulement (Cylindre, Sphère) et origine des carreaux
    pub fondu: f32,     // Triplanaire : largeur du fondu entre axes (0 = net, 1 = très doux)
}

impl Default for Params {
    fn default() -> Self {
        Params { mode: Mode::Triplanaire, taille_u: 10.0, taille_v: 10.0, rotation: 0.0, decalage: [0.0; 2], axe: [0.0, 0.0, 1.0], centre: [0.0; 3], fondu: 0.35 }
    }
}

type V = [f32; 3];
#[inline] fn dot(a: V, b: V) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
#[inline] fn sub(a: V, b: V) -> V { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
#[inline] fn cross(a: V, b: V) -> V { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
#[inline] fn norm(a: V) -> V { let l = dot(a, a).sqrt(); if l > 1e-12 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 0.0, 1.0] } }

/// Repère (e1, e2) d'un plan de normale `n`, stable : e1 suit l'horizontale quand c'est possible.
pub fn repere(n: V) -> (V, V) {
    let haut = if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    let e1 = norm(cross(haut, n));
    (e1, cross(n, e1))
}

/// Projection préparée (repères calculés une fois).
pub struct Projection { p: Params, cos: f32, sin: f32, plan: (V, V), axe: V, perp: (V, V) }

impl Projection {
    pub fn new(p: Params) -> Self {
        let (s, c) = p.rotation.to_radians().sin_cos();
        let axe = norm(p.axe);
        Projection { p, cos: c, sin: s, plan: repere(axe), axe, perp: repere(axe) }
    }

    /// (x, y) en mm dans le plan → carreau (u, v), avec rotation, échelle et décalage.
    #[inline]
    fn carreau(&self, x: f32, y: f32) -> (f32, f32) {
        let (xr, yr) = (self.cos * x - self.sin * y, self.sin * x + self.cos * y);
        (xr / self.p.taille_u.max(1e-3) + self.p.decalage[0], yr / self.p.taille_v.max(1e-3) + self.p.decalage[1])
    }

    /// Projection à plat vue de l'axe principal `k` (0 = X, 1 = Y, 2 = Z), orientée par le signe de la normale
    /// (le motif n'est pas vu en miroir sur la face opposée).
    #[inline]
    fn axe_principal(&self, q: V, k: usize, signe: f32) -> (f32, f32) {
        let (x, y) = match k { 0 => (q[1] * signe, q[2]), 1 => (-q[0] * signe, q[2]), _ => (q[0] * signe, q[1]) };
        self.carreau(x, y)
    }

    /// Lit la hauteur au point `p` de normale `n` (unitaire) avec la fonction `h(u, v)`.
    pub fn hauteur(&self, p: V, n: V, h: &dyn Fn(f32, f32) -> f32) -> f32 {
        let q = sub(p, self.p.centre);
        match self.p.mode {
            Mode::Plan => { let (u, v) = self.carreau(dot(q, self.plan.0), dot(q, self.plan.1)); h(u, v) }
            Mode::Cube => {
                let a = [n[0].abs(), n[1].abs(), n[2].abs()];
                let k = if a[0] >= a[1] && a[0] >= a[2] { 0 } else if a[1] >= a[2] { 1 } else { 2 };
                let (u, v) = self.axe_principal(q, k, n[k].signum());
                h(u, v)
            }
            Mode::Triplanaire => {
                // poids |n|^k : k élevé = raccord net ; seuls les axes de poids notable sont lus
                let k = 2.0 + (1.0 - self.p.fondu.clamp(0.0, 1.0)) * 14.0;
                let w = [n[0].abs().powf(k), n[1].abs().powf(k), n[2].abs().powf(k)];
                let tot = (w[0] + w[1] + w[2]).max(1e-12);
                let mut s = 0.0;
                for i in 0..3 {
                    let wi = w[i] / tot;
                    if wi > 0.01 { let (u, v) = self.axe_principal(q, i, if n[i] >= 0.0 { 1.0 } else { -1.0 }); s += wi * h(u, v); }
                }
                s
            }
            Mode::Cylindre => {
                let a = self.axe;
                let haut = dot(q, a);
                let radial = sub(q, [a[0] * haut, a[1] * haut, a[2] * haut]);
                let r = dot(radial, radial).sqrt();
                let cote = 1.0 - dot(n, a).abs(); // 1 sur la paroi, 0 sur les fonds
                let paroi = || {
                    let ang = dot(radial, self.perp.1).atan2(dot(radial, self.perp.0));
                    // longueur d'arc au rayon moyen : le motif garde ses proportions sur la paroi
                    let (u, v) = self.carreau(ang * r.max(1e-3), haut);
                    h(u, v)
                };
                let fond = || { let (u, v) = self.carreau(dot(q, self.perp.0), dot(q, self.perp.1)); h(u, v) };
                if cote > 0.35 { paroi() } else if cote < 0.15 { fond() } else { let t = (cote - 0.15) / 0.2; fond() * (1.0 - t) + paroi() * t }
            }
            Mode::Sphere => {
                let d = norm(q);
                let lon = dot(d, self.perp.1).atan2(dot(d, self.perp.0));
                let lat = dot(d, self.axe).clamp(-1.0, 1.0).asin();
                let r = dot(q, q).sqrt();
                let (u, v) = self.carreau(lon * r * lat.cos().max(0.05), lat * r);
                h(u, v)
            }
        }
    }
}

/// Taille de carreau la plus proche de `t` qui fait un nombre entier de tours sur un cylindre de rayon `r`
/// (raccord invisible à la jonction de la paroi).
pub fn taille_sans_couture(t: f32, r: f32) -> f32 {
    let tour = std::f32::consts::TAU * r;
    let n = (tour / t.max(1e-3)).round().max(1.0);
    tour / n
}
