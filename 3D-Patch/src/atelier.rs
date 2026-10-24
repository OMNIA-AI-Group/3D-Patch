//! ATELIER TEXTURES de 3D-Patch : panneau du mode Texture, pinceau des zones lisses, bibliothèque
//! d'images, aperçu en direct. Moteur : crate::matiere. Priorités de l'écran : choisir la texture,
//! peindre les zones à laisser lisses, appliquer ; tous les autres réglages sont repliés.

use crate::matiere::{self, motifs, projection, source};
use crate::{tr, Lang, JAUNE};
use glam::Vec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Outil du pinceau.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outil { Peindre, Surface, Gommer }

/// Demandes du panneau à l'application.
#[derive(Default)]
pub struct Actions { pub quitter: bool, pub appliquer: bool }

#[derive(Clone)]
struct Calque {
    relief: Option<source::Relief>, // None = pas encore de texture choisie
    proj: projection::Params,
    profondeur: f32,
    grave: bool,
    melange: matiere::Melange,
    visible: bool,
    proportion: bool,
}

impl Calque {
    fn nouveau(taille: f32, profondeur: f32, centre: [f32; 3]) -> Self {
        Calque {
            relief: None,
            proj: projection::Params { taille_u: taille, taille_v: taille, centre, ..Default::default() },
            profondeur, grave: false, melange: matiere::Melange::Ajouter, visible: true, proportion: true,
        }
    }
    fn nom(&self, lang: Lang) -> String {
        match self.relief.as_ref().map(|r| &r.source) {
            Some(source::Source::Motif(p)) => tr(lang, p.motif.cle()).to_string(),
            Some(source::Source::Image(c)) => c.nom.clone(),
            None => tr(lang, "at_vide").to_string(),
        }
    }
    fn cle(&self) -> String {
        let src = match self.relief.as_ref() {
            Some(r) => format!("{} {} {}", match &r.source { source::Source::Motif(p) => format!("{p:?}"), source::Source::Image(c) => format!("img:{}:{}", c.nom, c.g.len()) }, r.inverser, r.contraste),
            None => "-".into(),
        };
        format!("[{src} {:?} {} {} {:?} {}]", self.proj, self.profondeur, self.grave, self.melange, self.visible)
    }
    fn moteur(&self) -> Option<matiere::Calque> {
        let relief = self.relief.clone()?;
        Some(matiere::Calque {
            relief, proj: self.proj, profondeur: self.profondeur,
            sens: if self.grave { matiere::Sens::Rentrant } else { matiere::Sens::Sortant },
            melange: self.melange, opacite: 1.0, visible: self.visible,
        })
    }
}

/// Travail d'application (calculé dans un fil à part).
pub struct Travail { calques: Vec<matiere::Calque>, faces: Vec<u32>, reg: matiere::Reglages }

impl Travail {
    pub fn executer(&self, verts: &[f32]) -> Result<(Vec<f32>, usize), String> {
        let r = matiere::calculer(verts, &self.calques, &self.faces, &self.reg, matiere::Mode::Final)?;
        Ok((r.verts, r.avant))
    }
}

type Calcul = std::thread::JoinHandle<Result<(Vec<f32>, Vec<u32>), String>>;

pub struct Atelier {
    pub actif: bool,
    pub outil: Option<Outil>,
    pub peinture: bool,
    pub rayon: f32,
    pub msg: Option<String>,
    pub reveil: Option<Duration>,
    calques: Vec<Calque>,
    courant: usize,
    reg: matiere::Reglages,
    finesse_auto: bool,
    dessus_lisse: bool,
    dessous_lisse: bool,
    angle_lisse: f32,
    // objet visé : (index, nombre de triangles), ses centres de triangles, faces peintes
    cible: Option<(usize, usize)>,
    diag: f32,
    centre: [f32; 3],
    centres: Vec<Vec3>,
    normales: Vec<Vec3>,
    voisinage: Option<(usize, Voisinage)>,
    marche: f32, // outil Surface : dénivelé franchi (mm)
    famille: Option<motifs::Famille>, // onglet de la galerie (None = toutes)
    historique: Vec<Vec<bool>>, // zones lisses avant chaque action (clic droit = retour en arrière, dans l'ordre)
    exclu: Vec<bool>,
    gen_exclu: u64,
    images: Vec<Arc<source::Carte>>,
    choix_image: Option<std::thread::JoinHandle<Option<PathBuf>>>,
    vignettes: HashMap<String, egui::TextureHandle>,
    calcul: Option<(String, Calcul)>,
    cle_affichee: String,
    modifie: Instant,
    dernier: Option<(usize, Vec<u32>)>, // aperçu affiché : (objet, face d'origine de chaque triangle)
    gen_couleurs: u64,
}

const JAUNE_OBJ: [f32; 3] = [0.93, 0.80, 0.33];
const ROUGE_LISSE: [f32; 3] = [0.86, 0.24, 0.20];

impl Atelier {
    pub fn new() -> Self {
        let mut a = Atelier {
            actif: false, outil: None, peinture: false, rayon: 3.0, msg: None, reveil: None,
            calques: Vec::new(), courant: 0, reg: matiere::Reglages::default(), finesse_auto: true,
            dessus_lisse: false, dessous_lisse: true, angle_lisse: 20.0, // dessous lisse : la pièce reste à plat sur le plateau
            cible: None, diag: 100.0, centre: [0.0; 3], centres: Vec::new(), normales: Vec::new(), voisinage: None, marche: 0.5, famille: None, historique: Vec::new(), exclu: Vec::new(), gen_exclu: 0,
            images: Vec::new(), choix_image: None, vignettes: HashMap::new(), calcul: None, cle_affichee: String::new(),
            modifie: Instant::now(), dernier: None, gen_couleurs: u64::MAX,
        };
        a.charger_bibliotheque();
        a
    }

    pub fn ouvrir(&mut self) { self.actif = true; self.msg = None; }

    /// Après APPLIQUER : la texture est dans l'objet ; on vide le choix pour montrer le résultat tel quel
    /// (sinon l'aperçu reposerait la texture sur l'objet déjà texturé). Réglages et tailles sont gardés.
    pub fn apres_application(&mut self, msg: String) {
        for c in &mut self.calques { c.relief = None; }
        self.outil = None;
        self.msg = Some(msg);
    }
    pub fn fermer(&mut self) { self.actif = false; self.outil = None; self.peinture = false; }
    pub fn armer_pinceau(&mut self) { self.outil = Some(Outil::Peindre); }

    /// Rayon du pinceau (mm) quand un pinceau rond est armé (pas l'outil Surface).
    pub fn rayon_pinceau(&self) -> Option<f32> { self.outil.filter(|o| *o != Outil::Surface).map(|_| self.rayon) }

    /// Mémorise l'état des zones lisses avant une action (au plus 40 étapes).
    fn memoriser(&mut self) {
        self.historique.push(self.exclu.clone());
        if self.historique.len() > 40 { self.historique.remove(0); }
    }

    /// Clic droit : annule la dernière action sur les zones lisses (pinceau, surface, gomme, tout effacer),
    /// de la plus récente à la plus ancienne. Vrai si quelque chose a été annulé.
    pub fn annuler_exclusion(&mut self) -> bool {
        let Some(e) = self.historique.pop() else { return false };
        if e.len() == self.exclu.len() || self.exclu.is_empty() { self.exclu = e; }
        self.gen_exclu += 1;
        self.modifie = Instant::now();
        true
    }

    /// Clic droit sur une zone rouge : toute la zone laissée lisse qui touche la face `f` (faces voisines
    /// peintes, d'un seul tenant) est retirée.
    pub fn retirer_zone(&mut self, verts: &[f32], f: usize) {
        let n = verts.len() / 18;
        if f >= n || !self.exclu.get(f).copied().unwrap_or(false) { return; }
        if self.voisinage.as_ref().map_or(true, |v| v.0 != n) { self.voisinage = Some((n, Voisinage::new(verts))); }
        let vs = &self.voisinage.as_ref().unwrap().1;
        let mut pile = vec![f];
        self.exclu[f] = false;
        while let Some(g) = pile.pop() {
            for &h in &vs.voisins[g] { let h = h as usize; if self.exclu[h] { self.exclu[h] = false; pile.push(h); } }
        }
        self.gen_exclu += 1;
        self.modifie = Instant::now();
    }

    /// Outil SURFACE : clic sur la face `f` → toute la surface jusqu'aux arêtes franches, marches d'au plus
    /// `self.marche` mm franchies (escaliers). Maj (ou Gommer) retire la surface au lieu de l'ajouter.
    pub fn surface(&mut self, verts: &[f32], f: usize, inverse: bool) {
        let n = verts.len() / 18;
        if self.exclu.len() != n { self.exclu = vec![false; n]; }
        self.memoriser();
        if self.voisinage.as_ref().map_or(true, |v| v.0 != n) { self.voisinage = Some((n, Voisinage::new(verts))); }
        let reg = region_surface(verts, &self.voisinage.as_ref().unwrap().1, f, self.marche);
        for g in reg { self.exclu[g] = !inverse; }
        self.gen_exclu += 1;
        self.modifie = Instant::now();
    }

    /// Triangle touché par le rayon (o, d) et point d'impact.
    pub fn viser(o: Vec3, d: Vec3, tris: &[[Vec3; 3]]) -> Option<(usize, Vec3)> {
        let mut best: Option<(usize, f32)> = None;
        for (k, t) in tris.iter().enumerate() { if let Some(x) = crate::ray_tri(o, d, t) { if best.map_or(true, |b| x < b.1) { best = Some((k, x)); } } }
        best.map(|(k, x)| (k, o + d * x))
    }

    /// Coup de pinceau autour du point `p`, regardé dans la direction `vue` : faces dont le centre est dans le
    /// rayon ET tournées vers l'observateur (une pièce mince n'est pas peinte au travers). `inverse` (Maj) gomme.
    /// La gomme suit les aplats comme l'outil Surface : une face touchée libère tout son aplat (marches comprises).
    pub fn peindre(&mut self, tris: &[[Vec3; 3]], verts: &[f32], p: Vec3, vue: Vec3, inverse: bool) {
        if !self.peinture { self.memoriser(); } // début d'un coup de pinceau : une étape annulable
        if self.centres.len() != tris.len() {
            self.centres = tris.iter().map(|t| (t[0] + t[1] + t[2]) / 3.0).collect();
            self.normales = tris.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()).collect();
        }
        if self.exclu.len() != tris.len() { self.exclu = vec![false; tris.len()]; }
        let gommer = matches!(self.outil, Some(Outil::Gommer)) ^ inverse;
        let r2 = self.rayon * self.rayon;
        let mut change = false;
        let touchees: Vec<usize> = (0..self.centres.len())
            .filter(|&f| self.centres[f].distance_squared(p) <= r2 && self.exclu[f] == gommer && self.normales[f].dot(vue) < 0.0).collect();
        if gommer && !touchees.is_empty() {
            let n = tris.len();
            if self.voisinage.as_ref().map_or(true, |v| v.0 != n) { self.voisinage = Some((n, Voisinage::new(verts))); }
            for f in touchees {
                if !self.exclu[f] { continue; } // déjà libérée par l'aplat d'une face voisine
                for g in region_surface(verts, &self.voisinage.as_ref().unwrap().1, f, self.marche) {
                    if self.exclu[g] { self.exclu[g] = false; change = true; }
                }
            }
        } else {
            for f in touchees { self.exclu[f] = true; change = true; }
        }
        if change { self.gen_exclu += 1; self.modifie = Instant::now(); }
    }

    fn calque(&mut self) -> &mut Calque {
        if self.calques.is_empty() {
            let (t, p) = self.defauts();
            self.calques.push(Calque::nouveau(t, p, self.centre));
            self.courant = 0;
        }
        let i = self.courant.min(self.calques.len() - 1);
        &mut self.calques[i]
    }

    /// Taille de motif et profondeur proposées pour l'objet visé.
    fn defauts(&self) -> (f32, f32) {
        let t = (self.diag / 8.0).max(1.0);
        let t = if t > 10.0 { t.round() } else { (t * 10.0).round() / 10.0 };
        (t, (self.diag / 200.0).clamp(0.2, 2.0))
    }

    fn regler_finesse(&mut self) {
        if !self.finesse_auto { return; }
        let taille = self.calques.iter().filter(|c| c.visible && c.relief.is_some()).map(|c| c.proj.taille_u.min(c.proj.taille_v)).fold(f32::MAX, f32::min);
        let taille = if taille == f32::MAX { self.diag / 8.0 } else { taille };
        let prof = self.calques.iter().filter(|c| c.visible && c.relief.is_some()).map(|c| c.profondeur).fold(0.0f32, f32::max).max(0.1);
        self.reg.resolution = matiere::resolution_conseillee(self.diag, taille);
        self.reg.tolerance = (prof / 40.0).max(0.005);
    }

    /// Faces que le moteur doit texturer (bits des calques), 0 = laissée lisse.
    fn faces(&self, verts: &[f32]) -> Vec<u32> {
        let n = verts.len() / 18;
        let bits: u32 = self.calques.iter().enumerate().filter(|(_, c)| c.visible && c.relief.is_some()).fold(0, |b, (i, _)| b | (1 << i.min(31)));
        let angle = if self.dessus_lisse || self.dessous_lisse {
            Some(matiere::masque_angle(verts, if self.dessus_lisse { self.angle_lisse } else { 0.0 }, if self.dessous_lisse { self.angle_lisse } else { 0.0 }))
        } else { None };
        (0..n).map(|f| {
            let lisse = self.exclu.get(f).copied().unwrap_or(false) || angle.as_ref().map_or(false, |a| a[f]);
            if lisse { 0 } else { bits }
        }).collect()
    }

    /// Dessous lisse : le relief ne descend pas sous le point le plus bas de l'objet (le plateau).
    fn plancher(&self, verts: &[f32]) -> Option<f32> {
        self.dessous_lisse.then(|| verts.chunks_exact(6).map(|q| q[2]).fold(f32::MAX, f32::min))
    }

    fn calques_moteur(&self) -> Vec<matiere::Calque> { self.calques.iter().filter_map(|c| c.moteur()).collect() }

    fn a_texture(&self) -> bool { self.calques.iter().any(|c| c.visible && c.relief.is_some()) }

    /// Travail d'application pour l'objet visé (None sans texture).
    pub fn preparer(&self, verts: &[f32]) -> Option<Travail> {
        if !self.a_texture() { return None; }
        let mut reg = self.reg;
        reg.max_triangles = reg.max_triangles.max(verts.len() / 18);
        reg.plancher = self.plancher(verts);
        Some(Travail { calques: self.calques_moteur(), faces: self.faces(verts), reg })
    }

    /// Couleurs (3 par sommet) d'un maillage dont `src` donne la face d'origine de chaque triangle.
    fn couleurs(&self, src: &[u32], faces_lisses: &dyn Fn(u32) -> bool) -> Vec<f32> {
        let mut c = Vec::with_capacity(src.len() * 9);
        for &s in src { let col = if faces_lisses(s) { ROUGE_LISSE } else { JAUNE_OBJ }; for _ in 0..3 { c.extend_from_slice(&col); } }
        c
    }

    /// À chaque image : suit l'objet visé, lance / récupère l'aperçu. Renvoie l'aperçu à afficher
    /// (objet, sommets, couleurs ; sommets vides = nouvelles couleurs seulement) et s'il faut retirer l'aperçu.
    pub fn tick(&mut self, ctx: &egui::Context, cible: Option<(usize, &[f32])>) -> (Option<(usize, Vec<f32>, Vec<f32>)>, bool) {
        self.reveil = None;
        // image choisie dans le sélecteur
        if self.choix_image.as_ref().map_or(false, |h| h.is_finished()) {
            if let Some(p) = self.choix_image.take().unwrap().join().ok().flatten() { self.ajouter_image(&p); }
        }
        if self.choix_image.is_some() { self.reveil = Some(Duration::from_millis(150)); }
        if !self.actif || cible.is_none() {
            let retirer = self.dernier.take().is_some();
            self.cle_affichee.clear();
            if !self.actif { self.cible = None; }
            return (None, retirer);
        }
        let (i, verts) = cible.unwrap();
        let n = verts.len() / 18;
        if self.cible != Some((i, n)) {
            // nouvel objet : pinceau effacé, tailles proposées selon l'objet
            self.cible = Some((i, n));
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for q in verts.chunks_exact(6) { for d in 0..3 { lo[d] = lo[d].min(q[d]); hi[d] = hi[d].max(q[d]); } }
            self.diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1.0);
            self.centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
            self.exclu = vec![false; n];
            self.centres.clear();
            self.voisinage = None;
            self.historique.clear();
            self.gen_exclu += 1;
            self.rayon = (self.diag / 30.0).max(0.5);
            let (t, p) = self.defauts();
            for c in &mut self.calques { c.proj.centre = self.centre; c.proj.taille_u = t; c.proj.taille_v = t; c.profondeur = p; }
            self.dernier = None;
            self.cle_affichee.clear();
        }
        self.regler_finesse();
        // pendant un coup de pinceau : seulement les couleurs de l'aperçu affiché
        if self.peinture {
            if let Some((j, src)) = self.dernier.as_ref().filter(|d| d.0 == i) {
                if self.gen_couleurs != self.gen_exclu {
                    self.gen_couleurs = self.gen_exclu;
                    let ex = &self.exclu;
                    let cols = self.couleurs(src, &|s| ex.get(s as usize).copied().unwrap_or(false));
                    return (Some((*j, Vec::new(), cols)), false);
                }
            }
            return (None, false);
        }
        // aperçu à jour ?
        let cle = format!("{i} {n} {} {:?} {} {} {} {}", self.calques.iter().map(|c| c.cle()).collect::<String>(), self.reg, self.dessus_lisse, self.dessous_lisse, self.angle_lisse, self.gen_exclu);
        if let Some((k, h)) = self.calcul.as_ref() {
            if h.is_finished() {
                let (k, h) = self.calcul.take().unwrap();
                match h.join() {
                    Ok(Ok((v, src))) => {
                        let ex = self.exclu.clone();
                        let cols = self.couleurs(&src, &|s| ex.get(s as usize).copied().unwrap_or(false));
                        self.dernier = Some((i, src));
                        self.gen_couleurs = self.gen_exclu;
                        self.cle_affichee = k;
                        ctx.request_repaint();
                        return (Some((i, v, cols)), false);
                    }
                    Ok(Err(e)) => { self.msg = Some(e); self.cle_affichee = k; }
                    Err(_) => { self.cle_affichee = k; }
                }
            } else {
                let _ = k;
                self.reveil = Some(Duration::from_millis(60));
                return (None, false);
            }
        }
        if cle != self.cle_affichee {
            // petite attente après un changement (curseur qui glisse) avant de relancer le calcul
            let attente = Duration::from_millis(120).saturating_sub(self.modifie.elapsed());
            if !attente.is_zero() { self.reveil = Some(attente + Duration::from_millis(5)); return (None, false); }
            let v = verts.to_vec();
            if !self.a_texture() {
                // pas encore de texture : l'objet tel quel, zones peintes en rouge
                let src: Vec<u32> = (0..n as u32).collect();
                let ex = &self.exclu;
                let cols = self.couleurs(&src, &|s| ex.get(s as usize).copied().unwrap_or(false));
                self.dernier = Some((i, src));
                self.gen_couleurs = self.gen_exclu;
                self.cle_affichee = cle;
                return (Some((i, v, cols)), false);
            }
            let (calques, faces, mut reg) = (self.calques_moteur(), self.faces(verts), self.reg);
            reg.plancher = self.plancher(verts);
            self.calcul = Some((cle, std::thread::spawn(move || {
                let r = matiere::calculer(&v, &calques, &faces, &reg, matiere::Mode::Apercu)?;
                let src = if r.src.is_empty() { vec![0; r.verts.len() / 18] } else { r.src };
                Ok((r.verts, src))
            })));
            self.reveil = Some(Duration::from_millis(60));
        }
        (None, false)
    }

    pub fn calcul_en_cours(&self) -> bool { self.calcul.is_some() }

    // ───────────────────────────── bibliothèque d'images ─────────────────────────────

    fn dossier_images() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("3d-patch").join("images-relief"))
    }

    fn charger_bibliotheque(&mut self) {
        let Some(d) = Self::dossier_images() else { return };
        let Ok(rd) = std::fs::read_dir(&d) else { return };
        let mut ch: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        ch.sort();
        for p in ch { if let Ok(c) = source::Carte::charger(&p) { self.images.push(Arc::new(c)); } }
    }

    fn ajouter_image(&mut self, p: &std::path::Path) {
        match source::Carte::charger(p) {
            Ok(c) => {
                // copie dans la bibliothèque (retrouvée aux prochains lancements)
                if let Some(d) = Self::dossier_images() {
                    let _ = std::fs::create_dir_all(&d);
                    if let Some(nom) = p.file_name() { let _ = std::fs::copy(p, d.join(nom)); }
                }
                let c = Arc::new(c);
                self.images.retain(|x| x.nom != c.nom);
                self.images.push(c.clone());
                self.calque().relief = Some(source::Relief { source: source::Source::Image(c), inverser: false, contraste: 1.0 });
                self.modifie = Instant::now();
            }
            Err(e) => self.msg = Some(e),
        }
    }

    fn retirer_image(&mut self, nom: &str) {
        if let Some(d) = Self::dossier_images() {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() { if e.path().file_stem().map_or(false, |s| s.to_string_lossy() == nom) { let _ = std::fs::remove_file(e.path()); } }
            }
        }
        self.images.retain(|x| x.nom != nom);
    }

    // ───────────────────────────── vignettes ─────────────────────────────

    fn vignette(&mut self, ctx: &egui::Context, cle: String, h: impl Fn(f32, f32) -> f32) -> egui::TextureId {
        if let Some(t) = self.vignettes.get(&cle) { return t.id(); }
        let n = 72usize;
        let mut px = Vec::with_capacity(n * n);
        for y in 0..n { for x in 0..n {
            // relief éclairé de biais : lecture plus parlante qu'un simple gris
            let (u, v) = ((x as f32 + 0.5) / n as f32, 1.0 - (y as f32 + 0.5) / n as f32);
            let e = 1.0 / n as f32;
            let (hx, hy) = (h(u + e, v) - h(u - e, v), h(u, v + e) - h(u, v - e));
            let lum = (0.62 + 0.55 * h(u, v) * 0.5 - 1.8 * (hx - hy)).clamp(0.05, 1.0);
            let g = (lum * 235.0) as u8;
            px.push(egui::Color32::from_rgb(g, (g as f32 * 0.93) as u8, (g as f32 * 0.78) as u8));
        } }
        let t = ctx.load_texture(cle.clone(), egui::ColorImage { size: [n, n], pixels: px }, egui::TextureOptions::LINEAR);
        let id = t.id();
        self.vignettes.insert(cle, t);
        id
    }

    // ───────────────────────────── panneau ─────────────────────────────

    pub fn panneau(&mut self, ui: &mut egui::Ui, lang: Lang, cible: Option<(usize, &[f32])>, busy: bool) -> Actions {
        let mut act = Actions::default();
        let avant = self.cle_reglages();
        let retour = egui::Button::new(egui::RichText::new(format!("◀  {}", tr(lang, "at_quitter").to_uppercase())).size(12.5).strong().color(egui::Color32::WHITE))
            .fill(egui::Color32::from_gray(60)).rounding(egui::Rounding::same(6.0)).stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110)));
        if ui.add_sized(egui::vec2(ui.available_width(), 26.0), retour).clicked() { act.quitter = true; }
        ui.add_space(6.0);
        if cible.is_none() {
            ui.add(egui::Label::new(egui::RichText::new(tr(lang, "at_cible")).size(12.5).color(egui::Color32::from_rgb(235, 150, 90))).wrap(true));
            ui.add_space(4.0);
        }

        // ===== 1 · TEXTURE =====
        crate::title_box(ui, &tr(lang, "at_s_texture").to_uppercase(), 13.5, ui.available_width());
        ui.add_space(4.0);
        self.galerie(ui, lang);
        ui.add_space(6.0);
        if self.a_texture() || !self.calques.is_empty() {
            let diag = self.diag;
            egui::CollapsingHeader::new(egui::RichText::new(tr(lang, "at_s_reglages_tex").to_uppercase()).strong()).id_source("at_reglages_tex").default_open(false).show(ui, |ui| {
            let c = self.calque();
            let mut t = c.proj.taille_u;
            ui.horizontal(|ui| {
                ui.label(tr(lang, "at_taille"));
                if ui.add(egui::Slider::new(&mut t, (diag / 200.0).max(0.2)..=(diag * 1.5).max(5.0)).logarithmic(true).max_decimals(1).suffix(" mm")).on_hover_text(tr(lang, "at_taille_tip")).changed() {
                    let k = t / c.proj.taille_u.max(1e-6);
                    c.proj.taille_u = t;
                    c.proj.taille_v = if c.proportion { t } else { c.proj.taille_v * k };
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr(lang, "at_prof"));
                ui.add(egui::Slider::new(&mut c.profondeur, 0.05..=5.0).logarithmic(true).max_decimals(2).suffix(" mm")).on_hover_text(tr(lang, "at_prof_tip"));
            });
            ui.horizontal(|ui| {
                if crate::radio_jaune(ui, !c.grave, tr(lang, "at_relief")).clicked() { c.grave = false; }
                if crate::radio_jaune(ui, c.grave, tr(lang, "at_grave")).clicked() { c.grave = true; }
            });
            });
        }

        // ===== 2 · PINCEAU : ZONES LISSES =====
        ui.add_space(10.0);
        crate::title_box(ui, &tr(lang, "at_s_pinceau").to_uppercase(), 13.5, ui.available_width());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0;
            for (o, k, tip) in [(Outil::Peindre, "at_peindre", "at_peindre_tip"), (Outil::Surface, "at_surface", "at_surface_tip"), (Outil::Gommer, "at_gommer", "at_gommer_tip")] {
                let on = self.outil == Some(o);
                let b = egui::Button::new(egui::RichText::new(tr(lang, k).to_uppercase()).size(14.0).strong().color(if on { egui::Color32::BLACK } else { egui::Color32::WHITE }))
                    .fill(if on { if o == Outil::Gommer { JAUNE } else { egui::Color32::from_rgb(225, 80, 60) } } else { egui::Color32::from_gray(55) })
                    .rounding(egui::Rounding::same(6.0));
                if ui.add_sized(egui::vec2(w, 40.0), b).on_hover_text(tr(lang, tip)).clicked() { self.outil = if on { None } else { Some(o) }; }
            }
        });
        egui::CollapsingHeader::new(egui::RichText::new(tr(lang, "at_s_reglages_pinceau").to_uppercase()).strong()).id_source("at_reglages_pinceau").default_open(false).show(ui, |ui| {
        let rmax = (self.diag / 4.0).max(2.0);
        ui.horizontal(|ui| {
            ui.label(tr(lang, "at_rayon"));
            ui.add(egui::Slider::new(&mut self.rayon, (self.diag / 400.0).max(0.1)..=rmax).logarithmic(true).max_decimals(1).suffix(" mm"));
        });
        ui.horizontal(|ui| {
            ui.label(tr(lang, "at_marche"));
            ui.add(egui::Slider::new(&mut self.marche, 0.0..=0.5).max_decimals(2).suffix(" mm")).on_hover_text(tr(lang, "at_marche_tip"));
        });
        });
        // options d'exclusion toujours visibles
        ui.horizontal(|ui| {
            crate::case_jaune(ui, &mut self.dessus_lisse, &tr(lang, "at_dessus")).on_hover_text(tr(lang, "at_lisse_tip"));
            crate::case_jaune(ui, &mut self.dessous_lisse, &tr(lang, "at_dessous")).on_hover_text(tr(lang, "at_lisse_tip"));
        });
        let peints = self.exclu.iter().filter(|&&x| x).count();
        if ui.add_enabled(peints > 0, egui::Button::new(format!("{}  ({peints})", tr(lang, "at_effacer"))).min_size(egui::vec2(ui.available_width(), 24.0))).on_hover_text(tr(lang, "at_effacer_tip")).clicked() {
            self.memoriser();
            self.exclu.iter_mut().for_each(|x| *x = false);
            self.gen_exclu += 1;
        }
        ui.add(egui::Label::new(egui::RichText::new(tr(lang, "at_aide_exclure")).size(11.5).color(egui::Color32::from_gray(185))).wrap(true));

        // ===== APPLIQUER =====
        ui.add_space(10.0);
        let pret = cible.is_some() && self.a_texture() && !busy;
        if ui.add_enabled_ui(pret, |ui| ui.add_sized(egui::vec2(ui.available_width(), 38.0), crate::yellow_btn(&tr(lang, "at_appliquer").to_uppercase(), 14.5))).inner
            .on_hover_text(tr(lang, "at_appliquer_tip")).clicked() { act.appliquer = true; }
        if self.calcul.is_some() {
            ui.label(egui::RichText::new(tr(lang, "at_calcul")).size(11.5).color(egui::Color32::from_gray(180)));
        }
        if let Some(m) = &self.msg { ui.add(egui::Label::new(egui::RichText::new(m).size(12.0).color(egui::Color32::from_rgb(130, 210, 140))).wrap(true)); }

        // ===== réglages repliés =====
        ui.add_space(10.0);
        self.sections_repliees(ui, lang);

        if self.cle_reglages() != avant { self.modifie = Instant::now(); }
        act
    }

    fn cle_reglages(&self) -> String {
        format!("{} {:?} {} {} {} {} {}", self.calques.iter().map(|c| c.cle()).collect::<String>(), self.reg, self.finesse_auto, self.dessus_lisse, self.dessous_lisse, self.angle_lisse, self.courant)
    }

    /// Galerie : motifs calculés, puis les images de l'utilisateur et le bouton pour en ajouter.
    fn galerie(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let cols = 4usize;
        let esp = 4.0;
        let w = ((ui.available_width() - esp * (cols as f32 - 1.0)) / cols as f32).floor();
        let choisi: Option<String> = self.calques.get(self.courant).and_then(|c| c.relief.as_ref()).map(|r| match &r.source {
            source::Source::Motif(p) => format!("m:{:?}", p.motif), source::Source::Image(c) => format!("i:{}", c.nom),
        });
        let ctx = ui.ctx().clone();
        let mut clic: Option<source::Source> = None;
        let mut retirer: Option<String> = None;
        // onglets de familles
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            if ui.selectable_label(self.famille.is_none(), tr(lang, "f_toutes")).clicked() { self.famille = None; }
            for f in motifs::FAMILLES { if ui.selectable_label(self.famille == Some(f), tr(lang, f.cle())).clicked() { self.famille = Some(f); } }
        });
        ui.add_space(4.0);
        // sections affichées : « Toutes » = GRIP d'abord (titre à part), séparation, puis les autres textures
        let sections: Vec<(Option<&'static str>, Vec<motifs::Motif>)> = match self.famille {
            None => vec![
                (Some("at_grip_titre"), motifs::GRIP.to_vec()),
                (Some("at_autres"), motifs::TOUS.iter().copied().filter(|m| m.famille() != motifs::Famille::Grip).collect()),
            ],
            Some(f) => vec![(if f == motifs::Famille::Grip { Some("at_grip_titre") } else { None }, motifs::TOUS.iter().copied().filter(|m| m.famille() == f).collect())],
        };
        // ascenseur : 6 rangées de vignettes visibles, les boutons et menus du dessous restent à l'écran
        let haut = 6.0 * (w + esp);
        let w = w - 4.0; // place de l'ascenseur
        // ascenseur jaune, épais, toujours visible (bien repérable à droite des textures)
        ui.scope(|ui| {
        {
            let st = ui.style_mut();
            let mut sc = egui::style::ScrollStyle::solid();
            sc.bar_width = 16.0; sc.handle_min_length = 40.0; sc.bar_inner_margin = 3.0;
            st.spacing.scroll = sc;
            st.visuals.widgets.inactive.bg_fill = JAUNE;
            st.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(250, 222, 110);
            st.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(255, 235, 150);
            st.visuals.extreme_bg_color = egui::Color32::from_gray(35);
        }
        let w = w - 12.0; // place de l'ascenseur épais
        egui::ScrollArea::vertical().id_source("at_galerie").max_height(haut).auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(ui, |ui| {
        for (k, (titre, liste)) in sections.iter().enumerate() {
        if let Some(t) = titre {
            if k > 0 { ui.add_space(4.0); ui.separator(); }
            let grip = *t == "at_grip_titre";
            ui.label(egui::RichText::new(tr(lang, t)).size(12.0).strong().color(if grip { JAUNE } else { egui::Color32::from_gray(200) }));
            ui.add_space(2.0);
        }
        for r in liste.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = esp;
                for &m in r {
                    let p = motifs::Params { motif: m, ..Default::default() };
                    let id = self.vignette(&ctx, format!("m:{m:?}"), |u, v| motifs::hauteur(&p, u, v));
                    let on = choisi.as_deref() == Some(&format!("m:{m:?}"));
                    if case_vignette(ui, id, w, on).on_hover_text(tr(lang, m.cle())).clicked() {
                        // garde les réglages fins du motif d'avant si c'en était un
                        let ancien = self.calques.get(self.courant).and_then(|c| c.relief.as_ref()).and_then(|r| match &r.source { source::Source::Motif(q) => Some(*q), _ => None });
                        let q = ancien.map(|q| motifs::Params { motif: m, ..q }).unwrap_or(p);
                        clic = Some(source::Source::Motif(q));
                    }
                }
            });
            ui.add_space(esp);
        }
        }
        });
        });
        // mes images
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tr(lang, "at_mes_images")).size(12.5).strong());
        let imgs: Vec<Arc<source::Carte>> = self.images.clone();
        for r in imgs.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = esp;
                for c in r {
                    let c2 = c.clone();
                    let id = self.vignette(&ctx, format!("i:{}:{}", c.nom, c.g.len()), move |u, v| c2.lire(u, v));
                    let on = choisi.as_deref() == Some(&format!("i:{}", c.nom));
                    let resp = case_vignette(ui, id, w, on).on_hover_text(&c.nom);
                    if resp.clicked() { clic = Some(source::Source::Image(c.clone())); }
                    resp.context_menu(|ui| { if ui.button(tr(lang, "at_retirer_image")).clicked() { retirer = Some(c.nom.clone()); ui.close_menu(); } });
                }
            });
            ui.add_space(esp);
        }
        if ui.add_enabled(self.choix_image.is_none(), egui::Button::new(tr(lang, "at_ajouter_image")).min_size(egui::vec2(ui.available_width(), 26.0)))
            .on_hover_text(tr(lang, "at_ajouter_image_tip")).clicked() {
            self.choix_image = Some(std::thread::spawn(|| rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp"]).pick_file()));
        }
        // NÉGATIF : creux et bosses inversés sur l'objet
        ui.add_space(4.0);
        let neg = self.calques.get(self.courant).and_then(|c| c.relief.as_ref()).map_or(false, |r| r.inverser);
        let b = egui::Button::new(egui::RichText::new(tr(lang, "at_negatif").to_uppercase()).size(13.0).strong().color(if neg { egui::Color32::BLACK } else { egui::Color32::WHITE }))
            .fill(if neg { JAUNE } else { egui::Color32::from_gray(55) }).rounding(egui::Rounding::same(6.0));
        // NÉGATIF + taille du motif sur la même ligne, juste sous les textures
        let diag = self.diag;
        let mut inverser = false;
        let mut taille = self.calques.get(self.courant).map(|c| c.proj.taille_u);
        ui.horizontal(|ui| {
            if ui.add_sized(egui::vec2(96.0, 30.0), b).on_hover_text(tr(lang, "at_negatif_tip")).clicked() { inverser = true; }
            if let Some(t) = taille.as_mut() {
                ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(60.0);
                // minimum 1,5 mm : en dessous, le motif devient plus fin que le détail imprimable (simple grain)
                ui.add(egui::Slider::new(t, 1.5f32.min(diag / 10.0)..=(diag * 1.5).max(5.0)).logarithmic(true).max_decimals(1).suffix(" mm"))
                    .on_hover_text(tr(lang, "at_taille_tip"));
            }
        });
        if inverser { if let Some(r) = self.calque().relief.as_mut() { r.inverser = !r.inverser; } }
        if let Some(t) = taille {
            let c = self.calque();
            if (t - c.proj.taille_u).abs() > 1e-6 {
                let k = t / c.proj.taille_u.max(1e-6);
                c.proj.taille_u = t;
                c.proj.taille_v = if c.proportion { t } else { c.proj.taille_v * k };
            }
        }
        if let Some(n) = retirer { self.retirer_image(&n); }
        if let Some(s) = clic {
            // motif de prise (grip) : petites pointes serrées par défaut, pas la taille d'une texture décorative
            let prise = matches!(&s, source::Source::Motif(p) if p.motif.famille() == motifs::Famille::Grip);
            let deja_prise = self.calques.get(self.courant).and_then(|c| c.relief.as_ref())
                .map_or(false, |r| matches!(&r.source, source::Source::Motif(p) if p.motif.famille() == motifs::Famille::Grip));
            if prise && !deja_prise {
                let c = self.calque();
                c.proj.taille_u = 6.0; c.proj.taille_v = 6.0;
                c.profondeur = c.profondeur.min(0.4);
            }
            let c = self.calque();
            let (inv, ct) = c.relief.as_ref().map_or((false, 1.0), |r| (r.inverser, r.contraste));
            c.relief = Some(source::Relief { source: s, inverser: inv, contraste: ct });
            c.visible = true;
        }
    }

    fn sections_repliees(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let diag = self.diag;
        // --- réglages du motif ---
        if !self.calques.is_empty() {
            egui::CollapsingHeader::new(egui::RichText::new(tr(lang, "at_s_motif").to_uppercase()).strong()).id_source("at_motif").default_open(false).show(ui, |ui| {
                let c = self.calque();
                if let Some(r) = c.relief.as_mut() {
                    if let source::Source::Motif(p) = &mut r.source {
                        if p.motif.a_joint() { ui.add(egui::Slider::new(&mut p.joint, 0.02..=0.45).text(tr(lang, "at_joint"))); }
                        ui.add(egui::Slider::new(&mut p.arrondi, 0.0..=1.0).text(tr(lang, "at_arrondi")));
                        if p.motif.a_densite() { ui.add(egui::Slider::new(&mut p.densite, 1..=16).text(tr(lang, "at_densite"))); }
                        ui.add(egui::Slider::new(&mut p.graine, 1..=99).text(tr(lang, "at_variante")));
                    }
                    ui.checkbox(&mut r.inverser, tr(lang, "at_inverser"));
                    ui.add(egui::Slider::new(&mut r.contraste, 0.3..=3.0).logarithmic(true).text(tr(lang, "at_contraste")));
                }
            });
            // --- placement ---
            egui::CollapsingHeader::new(egui::RichText::new(tr(lang, "at_s_placement").to_uppercase()).strong()).id_source("at_placement").default_open(false).show(ui, |ui| {
                let c = self.calque();
                ui.label(tr(lang, "at_projection"));
                for m in projection::MODES { if ui.radio(c.proj.mode == m, tr(lang, m.cle())).clicked() { c.proj.mode = m; } }
                if matches!(c.proj.mode, projection::Mode::Plan | projection::Mode::Cylindre | projection::Mode::Sphere) {
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "at_axe"));
                        for (n, a) in [("X", [1.0, 0.0, 0.0]), ("Y", [0.0, 1.0, 0.0]), ("Z", [0.0, 0.0, 1.0])] {
                            if ui.selectable_label(c.proj.axe == a, n).clicked() { c.proj.axe = a; }
                        }
                    });
                }
                if c.proj.mode == projection::Mode::Triplanaire { ui.add(egui::Slider::new(&mut c.proj.fondu, 0.0..=1.0).text(tr(lang, "at_fondu"))); }
                ui.add(egui::Slider::new(&mut c.proj.rotation, -180.0..=180.0).suffix(" °").text(tr(lang, "at_rotation")));
                ui.add(egui::Slider::new(&mut c.proj.decalage[0], 0.0..=1.0).text(tr(lang, "at_decalage_u")));
                ui.add(egui::Slider::new(&mut c.proj.decalage[1], 0.0..=1.0).text(tr(lang, "at_decalage_v")));
                ui.checkbox(&mut c.proportion, tr(lang, "at_proportion"));
                if !c.proportion {
                    ui.add(egui::Slider::new(&mut c.proj.taille_v, (diag / 200.0).max(0.2)..=(diag * 1.5).max(5.0)).logarithmic(true).suffix(" mm").text(tr(lang, "at_taille_v")));
                } else { c.proj.taille_v = c.proj.taille_u; }
            });
        }
        // --- finesse ---
        egui::CollapsingHeader::new(egui::RichText::new(tr(lang, "at_s_finesse").to_uppercase()).strong()).id_source("at_finesse").default_open(false).show(ui, |ui| {
            ui.checkbox(&mut self.finesse_auto, tr(lang, "at_auto"));
            ui.add_enabled_ui(!self.finesse_auto, |ui| {
                ui.add(egui::Slider::new(&mut self.reg.resolution, 0.02..=5.0).logarithmic(true).suffix(" mm").text(tr(lang, "at_resolution"))).on_hover_text(tr(lang, "at_resolution_tip"));
                ui.add(egui::Slider::new(&mut self.reg.tolerance, 0.002..=0.5).logarithmic(true).suffix(" mm").text(tr(lang, "at_tolerance"))).on_hover_text(tr(lang, "at_tolerance_tip"));
            });
            let mut m = (self.reg.max_triangles / 1000) as u32;
            if ui.add(egui::Slider::new(&mut m, 100..=8000).logarithmic(true).suffix(" k").text(tr(lang, "at_max"))).changed() { self.reg.max_triangles = m as usize * 1000; }
            ui.add(egui::Slider::new(&mut self.reg.adoucir, 0.0..=(diag / 20.0).max(1.0)).suffix(" mm").text(tr(lang, "at_adoucir")));
            ui.add(egui::Slider::new(&mut self.angle_lisse, 1.0..=60.0).suffix(" °").text(tr(lang, "at_angle")));
        });
        // --- calques ---
        egui::CollapsingHeader::new(egui::RichText::new(tr(lang, "at_s_calques").to_uppercase()).strong()).id_source("at_calques").default_open(false).show(ui, |ui| {
            let mut suppr = None;
            for k in 0..self.calques.len() {
                ui.horizontal(|ui| {
                    let nom = format!("{} · {}", k + 1, self.calques[k].nom(lang));
                    if ui.selectable_label(self.courant == k, nom).clicked() { self.courant = k; }
                    let c = &mut self.calques[k];
                    ui.checkbox(&mut c.visible, "");
                    let r = if c.melange == matiere::Melange::Ajouter { tr(lang, "at_ajoute") } else { tr(lang, "at_remplace") };
                    if ui.small_button(r).on_hover_text(tr(lang, "at_melange_tip")).clicked() {
                        c.melange = if c.melange == matiere::Melange::Ajouter { matiere::Melange::Remplacer } else { matiere::Melange::Ajouter };
                    }
                    if ui.small_button("✕").clicked() { suppr = Some(k); }
                });
            }
            if let Some(k) = suppr { self.calques.remove(k); self.courant = self.courant.min(self.calques.len().saturating_sub(1)); }
            if self.calques.len() < 8 && ui.button(tr(lang, "at_ajouter_calque")).clicked() {
                let (t, p) = self.defauts();
                self.calques.push(Calque::nouveau(t, p, self.centre));
                self.courant = self.calques.len() - 1;
            }
        });
    }
}

/// Case de galerie : vignette carrée, cadre jaune si choisie.
fn case_vignette(ui: &mut egui::Ui, id: egui::TextureId, w: f32, choisi: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, w), egui::Sense::click());
    ui.painter().image(id, r.shrink(2.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    let bord = if choisi { egui::Stroke::new(3.0, JAUNE) } else if resp.hovered() { egui::Stroke::new(2.0, egui::Color32::from_gray(200)) } else { egui::Stroke::new(1.0, egui::Color32::from_gray(80)) };
    ui.painter().rect_stroke(r.shrink(1.0), 4.0, bord);
    resp
}

/// Voisinage des faces d'un objet (soudé comme le moteur) : faces voisines par arête, normales.
pub struct Voisinage { voisins: Vec<Vec<u32>>, normales: Vec<Vec3> }

impl Voisinage {
    pub fn new(verts: &[f32]) -> Self {
        let n = verts.len() / 18;
        let m = matiere::maillage::souder(verts);
        let ar = matiere::maillage::aretes(&m.t);
        let mut voisins = vec![Vec::new(); n];
        for e in 0..ar.ab.len() {
            let occ = &ar.occ[ar.debut[e] as usize..ar.debut[e + 1] as usize];
            for &a in occ { for &b in occ {
                let (fa, fb) = (m.src[a as usize / 3], m.src[b as usize / 3]);
                if fa != fb { voisins[fa as usize].push(fb); }
            } }
        }
        let normales = verts.chunks_exact(18).map(|t| {
            let (a, b, c) = (Vec3::new(t[0], t[1], t[2]), Vec3::new(t[6], t[7], t[8]), Vec3::new(t[12], t[13], t[14]));
            (b - a).cross(c - a).normalize_or_zero()
        }).collect();
        Voisinage { voisins, normales }
    }
}

/// Surface autour de la face `f0` : faces de même orientation (moins de 30° de la face cliquée, et moins de
/// 30° d'écart avec leur voisine), plus les bandes raides (contremarches) dont TOUTE la hauteur, mesurée le
/// long de la normale de la face cliquée, ne dépasse pas `marche` mm. Un vrai mur, plus haut, arrête la
/// surface même s'il est découpé en petits triangles (c'est la bande entière qui est jugée).
pub fn region_surface(verts: &[f32], v: &Voisinage, f0: usize, marche: f32) -> Vec<usize> {
    let n = verts.len() / 18;
    if f0 >= n { return Vec::new(); }
    let n0 = v.normales[f0];
    let cos_plat = 30f32.to_radians().cos();
    let haut = |f: usize| -> (f32, f32) {
        let o = f * 18;
        let h = [0, 6, 12].map(|k| n0.dot(Vec3::new(verts[o + k], verts[o + k + 1], verts[o + k + 2])));
        (h[0].min(h[1]).min(h[2]), h[0].max(h[1]).max(h[2]))
    };
    let plat = |f: usize| v.normales[f].dot(n0) >= cos_plat;
    // bandes raides : composante (faces raides voisines de moins de 30°) → hauteur totale
    let mut bande = vec![u32::MAX; n];
    let mut bande_ok: Vec<bool> = Vec::new();
    let mut juger = |f: usize, bande: &mut Vec<u32>| -> bool {
        if bande[f] != u32::MAX { return bande_ok[bande[f] as usize]; }
        let id = bande_ok.len() as u32;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut pile = vec![f];
        bande[f] = id;
        while let Some(g) = pile.pop() {
            let (a, b) = haut(g); lo = lo.min(a); hi = hi.max(b);
            for &h in &v.voisins[g] {
                let h = h as usize;
                if bande[h] == u32::MAX && !plat(h) && v.normales[h].dot(n0) > -cos_plat && v.normales[h].dot(v.normales[g]) >= cos_plat {
                    bande[h] = id; pile.push(h);
                }
            }
        }
        let ok = hi - lo <= marche + 1e-5;
        bande_ok.push(ok);
        ok
    };
    let mut pris = vec![false; n];
    let mut pile = vec![f0];
    pris[f0] = true;
    let mut out = Vec::new();
    while let Some(g) = pile.pop() {
        out.push(g);
        for &h in &v.voisins[g] {
            let h = h as usize;
            if pris[h] { continue; }
            let admis = if plat(h) { v.normales[h].dot(v.normales[g]) >= cos_plat || !plat(g) } else { v.normales[h].dot(n0) > -cos_plat && juger(h, &mut bande) };
            if admis { pris[h] = true; pile.push(h); }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escalier (5 marches de 0,4 mm, girons de 5 mm) posé sur un socle de 5 mm, en soupe de triangles.
    fn escalier(marche: f32) -> Vec<f32> {
        let mut v = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            for t in [[a, b, c], [a, c, d]] { for p in t { v.extend_from_slice(&p); v.extend_from_slice(&[0.0; 3]); } }
        };
        let (w, giron, socle) = (10.0f32, 5.0f32, 5.0f32);
        let h = |i: usize| socle + marche * i as f32;
        for i in 0..5 {
            let (x0, x1) = (giron * i as f32, giron * (i + 1) as f32);
            quad([x0, 0.0, h(i)], [x1, 0.0, h(i)], [x1, w, h(i)], [x0, w, h(i)]); // giron (vers +Z)
            if i < 4 { quad([x1, 0.0, h(i)], [x1, 0.0, h(i + 1)], [x1, w, h(i + 1)], [x1, w, h(i)]); } // contremarche (vers -X)
            quad([x0, 0.0, 0.0], [x1, 0.0, 0.0], [x1, 0.0, h(i)], [x0, 0.0, h(i)]); // flanc y = 0 (vers -Y)
            quad([x1, w, 0.0], [x0, w, 0.0], [x0, w, h(i)], [x1, w, h(i)]); // flanc y = w (vers +Y)
            quad([x0, 0.0, 0.0], [x0, w, 0.0], [x1, w, 0.0], [x1, 0.0, 0.0]); // dessous
        }
        quad([0.0, w, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, h(0)], [0.0, w, h(0)]); // bout bas
        quad([25.0, 0.0, 0.0], [25.0, w, 0.0], [25.0, w, h(4)], [25.0, 0.0, h(4)]); // bout haut
        v
    }

    /// Clic droit : les actions sur les zones lisses s'annulent de la plus récente à la plus ancienne.
    #[test]
    fn annulation_chronologique() {
        let v = escalier(2.0);
        let mut a = Atelier::new();
        let n = v.len() / 18;
        a.exclu = vec![false; n];
        a.marche = 0.0;
        a.surface(&v, 0, false);           // 1re action : premier giron
        let apres1 = a.exclu.clone();
        a.surface(&v, 4, false);           // 2e action : une autre face
        let apres2 = a.exclu.clone();
        assert_ne!(apres1, apres2);
        assert!(a.annuler_exclusion());
        assert_eq!(a.exclu, apres1, "1er clic droit : retour après la 1re action");
        assert!(a.annuler_exclusion());
        assert!(a.exclu.iter().all(|&x| !x), "2e clic droit : état de départ");
        assert!(!a.annuler_exclusion(), "plus rien à annuler");
    }

    #[test]
    fn surface_franchit_les_petites_marches() {
        let v = escalier(0.4);
        let vs = Voisinage::new(&v);
        // face 0 = premier giron : avec 0,5 mm, les 5 girons et 4 contremarches (2 triangles chacun)
        let r = region_surface(&v, &vs, 0, 0.5);
        assert_eq!(r.len(), 18, "girons + contremarches attendus, trouvé {:?}", r);
        // tout est tourné vers le haut ou est une contremarche : ni flanc, ni dessous, ni bout
        for &f in &r { let nz = vs.normales[f].z; let nx = vs.normales[f].x; assert!(nz > 0.9 || nx < -0.9, "face {f} hors surface"); }
        // marches de 0,4 mm avec une tolérance de 0,3 mm : seulement le premier giron
        assert_eq!(region_surface(&v, &vs, 0, 0.3).len(), 2);
        // un vrai mur (marches de 2 mm) arrête la surface
        let v2 = escalier(2.0);
        assert_eq!(region_surface(&v2, &Voisinage::new(&v2), 0, 0.5).len(), 2);
    }
}
