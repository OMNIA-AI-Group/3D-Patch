//! 3D-Patch — modeleur 3D façon MoI : OpenGL léger (glow) + egui, cœur OpenCASCADE.
//! 4 vues (Dessus / 3D / Face / Droite) + barre d'outils en bas. Convention CAO : Z vers le haut.
//! Objet = STL importé OU primitive OCCT (arg "prim:<nom>"). Grille X-Y (1 cm), règles graduées.

mod stl;
mod grid;
mod occt;
mod ruler;
mod view;
mod defects;
mod relief;
mod remesh;
mod convert;
mod simplify;
mod progress;
mod paths;
mod gpu;
mod fasthash;
mod perf;
mod assoc;
mod threemf;
mod assimp;
mod rhino;
mod sketchup;
mod gizmo;
mod gravite;
mod lumiere;
mod cache3d;
mod booleen;
mod forme;
mod texte;
mod vectorise;
mod impression;
mod plaque;
mod solidcraft;
mod matiere;
mod atelier;
#[cfg(test)]
mod tests_precision;

use std::ffi::CString;
use std::num::NonZeroU32;
use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3, Vec4, Vec4Swizzles};
use glow::HasContext;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, SwapInterval};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasRawWindowHandle;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowBuilder;

use view::{Ortho, Orbit, ViewKind, ViewMode};

/// Orientation de caméra commune (ouverture de l'appli, ouverture d'un fichier, touche F) :
/// vue côté −Y, un peu à droite, légère plongée => X vers la droite, Y qui s'éloigne, Z vers le haut.
const CAM_YAW: f32 = 2.89;
const CAM_PITCH: f32 = 0.25;
const BAR_H: f32 = 60.0; // hauteur de la barre du bas (une rangée de boutons de 50 px)
const PANEL_W: f32 = 212.0; // largeur du panneau d'outils de droite (points egui)
const PANEL_TEX_W: f32 = 340.0; // mode Texture : colonne élargie (galerie de vignettes 4 par rangée)

const VS: &str = r#"#version 330 core
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aNrm;
layout(location=2) in vec3 aCol;
uniform mat4 uMVP; uniform mat4 uModel;
out vec3 vNrm; out vec3 vPos; out vec3 vCol;
void main(){ vNrm = mat3(uModel)*aNrm; vPos = vec3(uModel*vec4(aPos,1.0)); vCol = aCol; gl_Position = uMVP*uModel*vec4(aPos,1.0); }"#;

const FS: &str = r#"#version 330 core
in vec3 vNrm; in vec3 vPos; in vec3 vCol; uniform vec3 uLight; uniform vec3 uEye; uniform float uReflect; uniform float uDim; uniform float uSel; uniform float uGray; uniform float uAlpha; uniform float uUseCol; uniform vec4 uTint; out vec4 frag;
void main(){
    vec3 N=normalize(vNrm); vec3 L=normalize(uLight); vec3 V=normalize(uEye-vPos);
    if(dot(N,V)<0.0) N=-N;
    float diff=max(dot(N,L),0.0);
    vec3 base=mix(vec3(0.93,0.80,0.33), vec3(0.98,0.52,0.16), uSel);  // Texturé = jaune ; sélection = orange
    base=mix(base, vCol, min(uUseCol, 1.0));                          // objet coloré (ex. logo) : ses propres couleurs
    base=mix(base, uTint.rgb, uTint.a);                               // booléens : A orange, B bleu
    base=mix(base, vec3(0.74,0.77,0.84), uReflect*0.8);               // Reflets = acier métallique
    float g=dot(base, vec3(0.33));
    base=mix(base, vec3(g*0.65), uGray);                              // Défauts = gris désaturé (jaune/rouge ressortent)
    base=mix(base, vec3(0.55,0.78,1.0), step(uAlpha, 0.999));          // Transparent = bleu clair
    // objet coloré : éclairage plus doux (couleurs proches de l'original, néons lisibles)
    vec3 col=base*mix(0.35+0.65*diff, 0.62+0.38*diff, uUseCol*step(uUseCol, 1.5)); // 2 = couleurs + éclairage normal
    vec3 H=normalize(L+V);
    float shin=mix(32.0,96.0,uReflect);
    col+=vec3(0.25+0.45*uReflect)*pow(max(dot(N,H),0.0),shin);
    col+=uReflect*0.06;
    col*=uDim;
    // Mode transparent : plus incliné vers les bords (effet verre/fresnel) pour rester lisible.
    float a=mix(1.0, clamp(uAlpha+ (1.0-abs(dot(N,V)))*0.45, 0.0, 1.0), step(uAlpha, 0.999));
    frag=vec4(col,a);
}"#;

struct Uniforms { light: Option<glow::NativeUniformLocation>, usecol: Option<glow::NativeUniformLocation>, mvp: Option<glow::NativeUniformLocation>, model: Option<glow::NativeUniformLocation>, eye: Option<glow::NativeUniformLocation>, reflect: Option<glow::NativeUniformLocation>, dim: Option<glow::NativeUniformLocation>, sel: Option<glow::NativeUniformLocation>, gray: Option<glow::NativeUniformLocation>, alpha: Option<glow::NativeUniformLocation>, tint: Option<glow::NativeUniformLocation> }

/// Un objet de la scène : géométrie EN COORDONNÉES MONDE (la translation est cuite dans les sommets)
/// + son propre VAO/VBO. Déplacer = translater les sommets et ré-uploader.
/// `cols` : couleur par sommet (r g b), vide = objet monochrome. Perdue automatiquement si la
/// géométrie change de nombre de sommets (réparation, simplification…).
struct Obj { vao: glow::NativeVertexArray, vbo: glow::NativeBuffer, cbo: glow::NativeBuffer, cols: Vec<f32>, verts: Vec<f32>, tris: Vec<[Vec3; 3]>, nverts: i32, min: Vec3, max: Vec3 }
impl Obj {
    unsafe fn with_colors(gl: &glow::Context, verts: Vec<f32>, cols: Vec<f32>) -> Obj {
        let mut o = Obj::new(gl, verts);
        o.cols = cols; o.upload(gl); o
    }
    unsafe fn new(gl: &glow::Context, verts: Vec<f32>) -> Obj {
        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(vao));
        let vbo = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let b: &[u8] = core::slice::from_raw_parts(verts.as_ptr() as *const u8, verts.len() * 4);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, b, glow::DYNAMIC_DRAW);
        gl.enable_vertex_attrib_array(0); gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 6 * 4, 0);
        gl.enable_vertex_attrib_array(1); gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 6 * 4, 3 * 4);
        let cbo = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(cbo));
        gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, 3 * 4, 0);
        gl.disable_vertex_attrib_array(2);
        let mut o = Obj { vao, vbo, cbo, cols: Vec::new(), verts, tris: Vec::new(), nverts: 0, min: Vec3::ZERO, max: Vec3::ZERO };
        cache3d::touche();
        o.refresh(); o
    }
    fn refresh(&mut self) {
        self.nverts = (self.verts.len() / 6) as i32;
        self.tris = self.verts.chunks_exact(18)
            .map(|c| [Vec3::new(c[0], c[1], c[2]), Vec3::new(c[6], c[7], c[8]), Vec3::new(c[12], c[13], c[14])]).collect();
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for c in self.verts.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
        if self.verts.is_empty() { lo = Vec3::ZERO; hi = Vec3::ZERO; }
        self.min = lo; self.max = hi;
    }
    unsafe fn upload(&self, gl: &glow::Context) {
        gl.bind_vertex_array(Some(self.vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
        let b: &[u8] = core::slice::from_raw_parts(self.verts.as_ptr() as *const u8, self.verts.len() * 4);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, b, glow::DYNAMIC_DRAW);
        self.upload_colors(gl);
    }
    unsafe fn upload_colors(&self, gl: &glow::Context) {
        cache3d::touche();
        gl.bind_vertex_array(Some(self.vao));
        if self.has_colors() {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.cbo));
            let c: &[u8] = core::slice::from_raw_parts(self.cols.as_ptr() as *const u8, self.cols.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, c, glow::STATIC_DRAW);
            gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, 3 * 4, 0);
            gl.enable_vertex_attrib_array(2);
        } else {
            gl.disable_vertex_attrib_array(2);
        }
    }
    /// Couleurs valides seulement si elles correspondent encore aux sommets.
    fn has_colors(&self) -> bool { !self.cols.is_empty() && self.cols.len() / 3 == self.verts.len() / 6 }
    /// Translation cuite dans les sommets (positions seules, pas les normales).
    fn translate(&mut self, d: Vec3) {
        for c in self.verts.chunks_exact_mut(6) { c[0] += d.x; c[1] += d.y; c[2] += d.z; }
        for t in &mut self.tris { for p in t.iter_mut() { *p += d; } }
        self.min += d; self.max += d;
    }
    /// Redimensionne autour d'un point `c` par facteur `f` (par axe) et recalcule normales à plat.
    fn scale_about(&mut self, c: Vec3, f: Vec3) {
        for ch in self.verts.chunks_exact_mut(6) {
            ch[0] = c.x + (ch[0] - c.x) * f.x; ch[1] = c.y + (ch[1] - c.y) * f.y; ch[2] = c.z + (ch[2] - c.z) * f.z;
        }
        for t in self.verts.chunks_exact_mut(18) {
            let a = Vec3::new(t[0], t[1], t[2]); let b = Vec3::new(t[6], t[7], t[8]); let d = Vec3::new(t[12], t[13], t[14]);
            let n = (b - a).cross(d - a).normalize_or_zero();
            for k in 0..3 { t[k * 6 + 3] = n.x; t[k * 6 + 4] = n.y; t[k * 6 + 5] = n.z; }
        }
        self.refresh();
    }
    unsafe fn delete(&self, gl: &glow::Context) { cache3d::touche(); gl.delete_vertex_array(self.vao); gl.delete_buffer(self.vbo); gl.delete_buffer(self.cbo); }
}

fn scene_bounds(scene: &[Obj]) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut any = false;
    for o in scene { if !o.verts.is_empty() { lo = lo.min(o.min); hi = hi.max(o.max); any = true; } }
    if any { (lo, hi) } else { (Vec3::ZERO, Vec3::ZERO) }
}

#[derive(Clone, Copy, PartialEq)]
enum Unit { Mm, Cm, M, In, Ft, Yd }
impl Unit {
    /// Longueur d'une unité en mm (1 unité monde = 1 mm ; pouce et pied exacts par définition).
    fn factor(self) -> f32 { match self { Unit::Mm => 1.0, Unit::Cm => 10.0, Unit::M => 1000.0, Unit::In => 25.4, Unit::Ft => 304.8, Unit::Yd => 914.4 } }
    fn suffix(self) -> &'static str { match self { Unit::Mm => "mm", Unit::Cm => "cm", Unit::M => "m", Unit::In => "in", Unit::Ft => "ft", Unit::Yd => "yd" } }
    fn metrique(self) -> bool { matches!(self, Unit::Mm | Unit::Cm | Unit::M) }
    /// Décimales des cotes affichées : résolution ≈ 0,01 mm quelle que soit l'unité.
    fn decimales(self) -> usize { match self { Unit::Mm => 2, Unit::Cm => 3, Unit::M => 5, Unit::In => 4, Unit::Ft => 5, Unit::Yd => 5 } }
    fn code(self) -> &'static str { self.suffix() }
    fn from_code(c: &str) -> Option<Unit> { [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft, Unit::Yd].into_iter().find(|u| u.code() == c.trim()) }
    /// Unité au démarrage : celle mémorisée, sinon le millimètre.
    fn initiale() -> Unit {
        paths::prefs().get("unite").and_then(|c| Unit::from_code(c))
            .or_else(|| paths::setting("unite").and_then(|c| Unit::from_code(&c))).unwrap_or(Unit::Mm)
    }
    /// Pas de grille / règle « rond » (1 / 2 / 5 × 10ⁿ) dans l'unité affichée : les graduations
    /// tombent sur des valeurs rondes quelle que soit l'unité (mm, cm, m, in, ft, yd).
    fn pas_grille(self, cible_mm: f32) -> f32 {
        let f = self.factor();
        let pas = nice_step(cible_mm / f) * f;
        if self.metrique() { pas.max(1.0) } else { pas.max(0.1 * 25.4) }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum DisplayMode { Textured, Transparent, Wireframe, Defects }

#[derive(Clone, Copy)]
enum Drag { Orbit(ViewKind), Pan(ViewKind), Move(ViewKind), Zoom(ViewKind) }

// Outils de navigation de l'overlay (au survol de chaque vue).
#[derive(Clone, Copy, PartialEq)]
enum NavTool { Zone, Zoom, Pan, Rotate }

/// Forme traçable au cliquer-glisser sur la grille (onglet Courbes).
#[derive(Clone, Copy, PartialEq)]
enum DrawShape { Line, Rect, Circle }

#[derive(Clone, Copy, PartialEq)]
enum Lang { Fr, Es, It, Us, De }
impl Lang {
    fn code(self) -> &'static str {
        match self { Lang::Fr => "fr", Lang::Es => "es", Lang::It => "it", Lang::Us => "en", Lang::De => "de" }
    }
    fn from_code(c: &str) -> Option<Lang> {
        let c = c.trim().to_ascii_lowercase();
        LANGS.into_iter().find(|l| c.starts_with(l.code()))
    }
    /// Langue au démarrage : PATCH3D_LANG (tests), sinon celle mémorisée, sinon celle du système, sinon français.
    fn initiale() -> Lang {
        std::env::var("PATCH3D_LANG").ok().and_then(|c| Lang::from_code(&c))
            .or_else(|| paths::prefs().get("langue").and_then(|c| Lang::from_code(c)))
            .or_else(|| paths::setting("langue").and_then(|c| Lang::from_code(&c)))
            .or_else(|| ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty()).and_then(|v| Lang::from_code(&v)))
            .unwrap_or(Lang::Fr)
    }
    fn name(self) -> &'static str {
        match self { Lang::Fr => "Français", Lang::Es => "Español", Lang::It => "Italiano", Lang::Us => "English (US)", Lang::De => "Deutsch" }
    }
}
const LANGS: [Lang; 5] = [Lang::Fr, Lang::Es, Lang::It, Lang::Us, Lang::De];

/// Langue affichée, lisible par les modules (messages d'erreur) sans la leur passer.
static LANGUE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
fn langue_regler(l: Lang) { LANGUE.store(LANGS.iter().position(|&x| x == l).unwrap_or(0) as u8, std::sync::atomic::Ordering::Relaxed); }
/// Langue affichée (index FR, ES, IT, US, DE) pour les modules.
pub(crate) fn langue_idx() -> usize { (LANGUE.load(std::sync::atomic::Ordering::Relaxed) as usize).min(LANGS.len() - 1) }
/// Traduction dans la langue affichée (pour les modules).
pub(crate) fn trc(key: &str) -> &'static str { tr(LANGS[(LANGUE.load(std::sync::atomic::Ordering::Relaxed) as usize).min(LANGS.len() - 1)], key) }
/// Traduction d'une étiquette d'interface. Ordre des colonnes : FR, ES, IT, US, DE.
fn tr(l: Lang, key: &str) -> &'static str {
    let i = match l { Lang::Fr => 0, Lang::Es => 1, Lang::It => 2, Lang::Us => 3, Lang::De => 4 };
    let row: [&'static str; 5] = match key {
        "sauve_sel"  => ["Objets sélectionnés sauvegardés", "Objetos seleccionados guardados", "Oggetti selezionati salvati", "Selected objects saved", "Gewählte Objekte gespeichert"],
        "sauve_avert"=> ["Attention : {n} objets sur le plateau, seuls les {s} sélectionnés seront sauvegardés (Ctrl+A = tous)",
                         "Atención: {n} objetos en la bandeja, solo se guardarán los {s} seleccionados (Ctrl+A = todos)",
                         "Attenzione: {n} oggetti sul piatto, verranno salvati solo i {s} selezionati (Ctrl+A = tutti)",
                         "Warning: {n} objects on the plate, only the {s} selected will be saved (Ctrl+A = all)",
                         "Achtung: {n} Objekte auf der Platte, nur die {s} gewählten werden gespeichert (Strg+A = alle)"],
        "sauve_avert1"=> ["Attention : {n} objets sur le plateau, seul l'objet sélectionné sera sauvegardé (Ctrl+A = tous)",
                         "Atención: {n} objetos en la bandeja, solo se guardará el objeto seleccionado (Ctrl+A = todos)",
                         "Attenzione: {n} oggetti sul piatto, verrà salvato solo l'oggetto selezionato (Ctrl+A = tutti)",
                         "Warning: {n} objects on the plate, only the selected object will be saved (Ctrl+A = all)",
                         "Achtung: {n} Objekte auf der Platte, nur das gewählte Objekt wird gespeichert (Strg+A = alle)"],
        "sauve_objets"=> ["objets", "objetos", "oggetti", "objects", "Objekte"],
        "sauve_3mf"  => ["3MF : chaque objet reste séparé (positions, couleurs) — autres formats : fusionnés en un seul",
                         "3MF: cada objeto queda separado (posiciones, colores) — otros formatos: fusionados en uno",
                         "3MF: ogni oggetto resta separato (posizioni, colori) — altri formati: uniti in uno solo",
                         "3MF: each object stays separate (positions, colours) — other formats: merged into one",
                         "3MF: jedes Objekt bleibt getrennt (Positionen, Farben) — andere Formate: zu einem vereint"],
        "sauve_tout" => ["Aucune sélection : toute la scène est sauvegardée", "Sin selección: se guarda toda la escena", "Nessuna selezione: si salva tutta la scena", "No selection: the whole scene is saved", "Keine Auswahl: die ganze Szene wird gespeichert"],
        "fichier"    => ["Fichier", "Archivo", "File", "File", "Datei"],
        "f_ouvrir"   => ["Ouvrir…", "Abrir…", "Apri…", "Open…", "Öffnen…"],
        "f_ouvrir_tip" => ["Remplace la scène par le fichier choisi", "Sustituye la escena por el archivo elegido", "Sostituisce la scena con il file scelto", "Replaces the scene with the chosen file", "Ersetzt die Szene durch die gewählte Datei"],
        "f_importer" => ["Importer (ajouter à la scène)…", "Importar (añadir a la escena)…", "Importa (aggiungi alla scena)…", "Import (add to the scene)…", "Importieren (zur Szene hinzufügen)…"],
        "f_importer_tip" => ["Ajoute l'objet du fichier à côté de ceux déjà présents (pour les booléens, l'assemblage…)", "Añade el objeto del archivo junto a los ya presentes (booleanas, ensamblaje…)", "Aggiunge l'oggetto del file accanto a quelli presenti (booleane, assemblaggio…)", "Adds the file's object next to those already there (booleans, assembly…)", "Fügt das Objekt der Datei neben die vorhandenen ein (Boolesch, Montage…)"],
        "vis_fil_tip"   => ["Affichage fil de fer", "Vista alámbrica", "Vista wireframe", "Wireframe view", "Drahtgitteransicht"],
        "vis_plein_tip" => ["Affichage plein (facettes)", "Vista sólida (facetas)", "Vista piena (sfaccettature)", "Solid view (facets)", "Volle Ansicht (Facetten)"],
        "chargement" => ["CHARGEMENT", "CARGANDO", "CARICAMENTO", "LOADING", "LADEN"],
        "exportation" => ["EXPORTATION", "EXPORTACIÓN", "ESPORTAZIONE", "EXPORT", "EXPORT"],
        "export"     => ["Export", "Exportar", "Esporta", "Export", "Export"],
        "export_rep" => ["Réparation…", "Reparando…", "Riparazione…", "Repairing…", "Reparatur…"],
        "export_tip" => ["Exporter : corrige d'abord les défauts (trous, faces en erreur), puis choisis le format d'enregistrement",
                         "Exportar: corrige primero los defectos (agujeros, caras con error) y luego elige el formato",
                         "Esporta: corregge prima i difetti (buchi, facce errate), poi scegli il formato",
                         "Export: first fixes the defects (holes, faulty faces), then choose the save format",
                         "Exportieren: behebt zuerst die Fehler (Löcher, fehlerhafte Flächen), dann das Speicherformat wählen"],
        "sauver"     => ["Sauver", "Guardar", "Salva", "Save", "Speichern"],
        "enreg_sous" => ["Enregistrer sous", "Guardar como", "Salva come", "Save as", "Speichern unter"],
        "photo"      => ["Photo", "Foto", "Foto", "Snapshot", "Foto"],
        "photo_tit"  => ["Copie d'écran (JPG)", "Captura (JPG)", "Istantanea (JPG)", "Screenshot (JPG)", "Bildschirmfoto (JPG)"],
        "annuler"    => ["Annuler", "Deshacer", "Annulla", "Undo", "Zurück"],
        "refaire"    => ["Refaire", "Rehacer", "Ripeti", "Redo", "Vor"],
        "pp_copie"   => ["Copié (Ctrl+V pour coller)", "Copiado (Ctrl+V para pegar)", "Copiato (Ctrl+V per incollare)", "Copied (Ctrl+V to paste)", "Kopiert (Strg+V zum Einfügen)"],
        "pp_coupe"   => ["Coupé (Ctrl+V pour coller)", "Cortado (Ctrl+V para pegar)", "Tagliato (Ctrl+V per incollare)", "Cut (Ctrl+V to paste)", "Ausgeschnitten (Strg+V zum Einfügen)"],
        "piece_sel"     => ["Pièce {k} sur {n} choisie — clic droit : Dégrouper", "Pieza {k} de {n} elegida — clic derecho: Desagrupar", "Pezzo {k} di {n} scelto — clic destro: Separa", "Part {k} of {n} picked — right-click: Ungroup", "Teil {k} von {n} gewählt — Rechtsklick: Trennen"],
        "grouper"       => ["Grouper les objets ({n})", "Agrupar los objetos ({n})", "Raggruppa gli oggetti ({n})", "Group objects ({n})", "Objekte gruppieren ({n})"],
        "grouper_ok"    => ["Groupé : {n} objets en un seul", "Agrupado: {n} objetos en uno", "Raggruppato: {n} oggetti in uno", "Grouped: {n} objects into one", "Gruppiert: {n} Objekte zu einem"],
        "piece_titre"   => ["Pièce {k} sur {n}", "Pieza {k} de {n}", "Pezzo {k} di {n}", "Part {k} of {n}", "Teil {k} von {n}"],
        "degroup_piece" => ["Dégrouper la pièce", "Desagrupar la pieza", "Separa il pezzo", "Ungroup this part", "Teil abtrennen"],
        "degroup_tout"  => ["Dégrouper tout ({n})", "Desagrupar todas las piezas ({n})", "Separa tutti i pezzi ({n})", "Ungroup all parts ({n})", "Alle Teile trennen ({n})"],
        "degroup_ok"    => ["Dégroupé : {n} objets — déplacer, Suppr / Retour arrière, Ctrl+C / Ctrl+V", "Desagrupado: {n} objetos — mover, Supr / Retroceso, Ctrl+C / Ctrl+V", "Separato: {n} oggetti — sposta, Canc / Backspace, Ctrl+C / Ctrl+V", "Ungrouped: {n} objects — move, Del / Backspace, Ctrl+C / Ctrl+V", "Getrennt: {n} Objekte — verschieben, Entf / Rücktaste, Strg+C / Strg+V"],
        "pp_colle"   => ["Collé", "Pegado", "Incollato", "Pasted", "Eingefügt"],
        "effacer"    => ["Effacer", "Borrar", "Cancella", "Clear", "Löschen"],
        "effacer_tip"=> ["Efface tout : les objets et les outils (cadre de coupe…). ANNULER les rend.\nUn seul objet : sélectionne-le puis touche Suppr",
                         "Borra todo: objetos y herramientas (marco de corte…). DESHACER los recupera.\nUn solo objeto: selecciónalo y pulsa Supr",
                         "Cancella tutto: oggetti e strumenti (riquadro di taglio…). ANNULLA li ripristina.\nUn solo oggetto: selezionalo e premi Canc",
                         "Clears everything: objects and tools (cut frame…). UNDO brings them back.\nOne object only: select it and press Delete",
                         "Löscht alles: Objekte und Werkzeuge (Schnittrahmen…). RÜCKGÄNGIG stellt sie wieder her.\nNur ein Objekt: auswählen und Entf drücken"],
        "vues4"      => ["4 Vues", "4 Vistas", "4 Viste", "4 Views", "4 Ansichten"],
        "top"        => ["Dessus", "Arriba", "Alto", "Top", "Oben"],
        "front"      => ["Face", "Frente", "Fronte", "Front", "Vorne"],
        "right"      => ["Droite", "Derecha", "Destra", "Right", "Rechts"],
        "regle"      => ["Règles", "Reglas", "Righelli", "Rulers", "Lineale"],
        "regle_tip"  => ["Règles\nClic gauche : afficher / masquer les règles\nClic droit : changer les unités",
                         "Reglas\nClic izquierdo: mostrar / ocultar\nClic derecho: cambiar las unidades",
                         "Righelli\nClic sinistro: mostra / nascondi\nClic destro: cambia le unità",
                         "Rulers\nLeft click: show / hide the rulers\nRight click: change units",
                         "Lineale\nLinksklick: ein- / ausblenden\nRechtsklick: Einheiten ändern"],
        "unites"     => ["Unités", "Unidades", "Unità", "Units", "Einheiten"],
        "unites_tip" => ["Choisir le système d'unités (métrique ou notation anglaise) et l'unité affichée",
                         "Elegir el sistema de unidades (métrico o inglés) y la unidad mostrada",
                         "Scegli il sistema di unità (metrico o inglese) e l'unità mostrata",
                         "Choose the unit system (metric or imperial) and the displayed unit",
                         "Einheitensystem (metrisch oder angloamerikanisch) und angezeigte Einheit wählen"],
        "snap_grille"=> ["Snap Grille", "Rejilla", "Griglia", "Grid Snap", "Raster"],
        "snap_direct"=> ["Snap Direct", "Directo", "Diretto", "Direct", "Direkt"],
        "snap_objet" => ["Snap Objet", "Objeto", "Oggetto", "Obj Snap", "Objekt"],
        "affichage"  => ["Visualisations", "Visualizaciones", "Visualizzazioni", "Views", "Ansichten"],
        "defauts"    => ["Défauts & Erreurs", "Defectos y errores", "Difetti ed errori", "Defects & Errors", "Mängel & Fehler"],
        "faces"      => ["face(s)", "cara(s)", "faccia/e", "face(s)", "Fläche(n)"],
        "trous"      => ["trou(s)", "agujero(s)", "foro/i", "hole(s)", "Loch/Löcher"],
        "defauts_btn"=> ["Trous & erreurs de face", "Agujeros y errores de cara", "Fori ed errori di faccia", "Holes & face errors", "Löcher & Flächenfehler"],
        "repair"     => ["Réparation auto", "Reparación auto", "Riparazione auto", "Auto-repair", "Auto-Reparatur"],
        "options"    => ["Options", "Opciones", "Opzioni", "Options", "Optionen"],
        "aide"       => ["Aide", "Ayuda", "Aiuto", "Help", "Hilfe"],
        "a_propos"   => ["À propos", "Acerca de", "Informazioni", "About", "Über"],
        "licence"    => ["Licence", "Licencia", "Licenza", "License", "Lizenz"],
        "version"    => ["Version", "Versión", "Versione", "Version", "Version"],
        "composants" => ["Composants tiers", "Componentes", "Componenti", "Third-party", "Komponenten"],
        "selecteur"  => ["Sélecteur", "Selector", "Selettore", "Selector", "Auswahl"],
        "textured"   => ["Texturé", "Texturizado", "Texture", "Textured", "Texturiert"],
        "reflets"    => ["Reflets & Ombres", "Reflejos y sombras", "Riflessi e ombre", "Reflections & shadows", "Reflexe & Schatten"],
        "transparent"=> ["Transparent", "Transparente", "Trasparente", "Transparent", "Transparent"],
        "nav_zone"   => ["Zone", "Zona", "Zona", "Zone", "Zone"],
        "nav_zoom"   => ["Zoom", "Zoom", "Zoom", "Zoom", "Zoom"],
        "nav_mvt"    => ["Mvt", "Mov", "Mov", "Pan", "Bew"],
        "nav_rot"    => ["Rotation", "Rotación", "Rotazione", "Rotate", "Drehen"],
        "defauts_mode"=> ["Erreurs & Défauts", "Errores y defectos", "Errori e difetti", "Errors & Defects", "Fehler & Mängel"],
        "polygones"  => ["Polygones", "Polígonos", "Poligoni", "Polygons", "Polygone"],
        "cible"      => ["Cible", "Objetivo", "Obiettivo", "Target", "Ziel"],
        "simplif"    => ["Simplification", "Simplificación", "Semplificazione", "Simplification", "Vereinfachung"],
        "simp_env"   => ["Enveloppe unique (sans jours)", "Envoltura única (sin huecos)", "Involucro unico (senza fessure)", "Single envelope (no gaps)", "Einzelhülle (ohne Spalten)"],
        "simp_env_tip"=> ["Recompose d'abord l'objet en une seule surface fermée : évite les jours et éclats entre pièces séparées (cheveux, parties imbriquées) quand on réduit fortement",
                          "Recompone primero el objeto en una sola superficie cerrada: evita huecos entre piezas separadas",
                          "Ricompone prima l'oggetto in un'unica superficie chiusa: evita fessure tra pezzi separati",
                          "First rebuilds the object as one closed surface: avoids gaps and shards between separate pieces when reducing strongly",
                          "Baut das Objekt zuerst als eine geschlossene Fläche auf: vermeidet Spalten zwischen getrennten Teilen"],
        "simplif_tip"=> ["Ramène l'objet au nombre de triangles visé : en dessous = simplifie (garde la forme), au-dessus = subdivise et lisse",
                         "Lleva el objeto al número de triángulos: menos = simplifica, más = subdivide y alisa",
                         "Porta l'oggetto al numero di triangoli: meno = semplifica, più = suddivide e leviga",
                         "Brings the object to the target triangle count: lower = simplify (keeps shape), higher = subdivide and smooth",
                         "Bringt das Objekt auf die Ziel-Dreieckszahl: weniger = vereinfachen, mehr = unterteilen und glätten"],
        "relief_j"   => ["Relief", "Relieve", "Rilievo", "Relief", "Relief"],
        "calcul"     => ["Calcul", "Cálculo", "Calcolo", "Computing", "Berechnung"],
        "pret"       => ["Prêt", "Listo", "Pronto", "Ready", "Bereit"],
        "termine"    => ["Terminé", "Terminado", "Completato", "Done", "Fertig"],
        "assoc_on"   => ["3D-Patch en visionneuse 3D", "3D-Patch como visor 3D", "3D-Patch come visualizzatore 3D", "3D-Patch as 3D viewer", "3D-Patch als 3D-Betrachter"],
        "assoc_tip"  => ["Cochée : 3D-Patch devient l'application par défaut des fichiers 3D (STL, OBJ, STEP, IGES, DXF, BLEND, FreeCAD). Un double-clic sur un fichier l'ouvre en visionneuse (sans outils ; double-clic gauche dans la vue = interface complète).\nDécochée : les applications précédentes sont restaurées.",
                         "Marcada: 3D-Patch abre los archivos 3D con doble clic (modo visor). Desmarcada: se restauran las aplicaciones anteriores.",
                         "Selezionata: 3D-Patch apre i file 3D con doppio clic (visualizzatore). Deselezionata: ripristina le applicazioni precedenti.",
                         "Checked: 3D-Patch becomes the default app for 3D files (STL, OBJ, STEP, IGES, DXF, BLEND, FreeCAD); double-click opens them in viewer mode. Unchecked: previous apps are restored.",
                         "Aktiviert: 3D-Patch öffnet 3D-Dateien per Doppelklick (Betrachter). Deaktiviert: frühere Programme werden wiederhergestellt."],
        "gpu_on"     => ["Activer GPU", "Activar GPU", "Attiva GPU", "Enable GPU", "GPU aktivieren"],
        "m_adapt"    => ["L'appli s'ajuste à la machine : mesures prises au lancement. Le GPU n'est utilisé que s'il est plus rapide que le processeur ; la finesse des voxels suit la puissance de la machine (plafonnée par la mémoire).",
                         "La app se ajusta a la máquina: mediciones al iniciar, se elige el motor más rápido.",
                         "L'app si adatta alla macchina: misure all'avvio, viene scelto il motore più veloce.",
                         "The app adapts to the machine: measured at startup, the fastest engine is chosen and voxel detail follows its power (capped by memory).",
                         "Die App passt sich an: Messung beim Start, der schnellste Rechenweg wird gewählt."],
        "m_cpu"      => ["Processeur", "Procesador", "Processore", "Processor", "Prozessor"],
        "m_cores"    => ["cœurs", "núcleos", "core", "cores", "Kerne"],
        "m_ram"      => ["Mémoire vive disponible", "Memoria disponible", "Memoria disponibile", "Available RAM", "Freier Arbeitsspeicher"],
        "m_vram"     => ["Mémoire vidéo libre", "Memoria de vídeo libre", "Memoria video libera", "Free video memory", "Freier Grafikspeicher"],
        "m_vs_cpu"   => ["plus rapide que le processeur au test voxels", "más rápida que el procesador", "più veloce del processore", "faster than the processor (voxel test)", "schneller als der Prozessor"],
        "m_engine"   => ["Moteur des calculs voxels", "Motor de cálculo", "Motore di calcolo", "Voxel compute engine", "Voxel-Rechenweg"],
        "m_eng_gpu"  => ["GPU", "GPU", "GPU", "GPU", "GPU"],
        "m_eng_cpu"  => ["processeur (case décochée)", "procesador (casilla desmarcada)", "processore (casella non selezionata)", "processor (box unchecked)", "Prozessor (nicht aktiviert)"],
        "m_eng_slow" => ["processeur (le GPU est plus lent sur cette machine)", "procesador (la GPU es más lenta aquí)", "processore (la GPU è più lenta qui)", "processor (the GPU is slower on this machine)", "Prozessor (GPU hier langsamer)"],
        "m_res"      => ["Finesse des voxels", "Resolución de vóxeles", "Risoluzione voxel", "Voxel detail", "Voxel-Auflösung"],
        "fichier_en_cours" => ["Fichier…", "Archivo…", "File…", "File…", "Datei…"],
        "gpu_pow"    => ["Puissance mesurée", "Potencia medida", "Potenza misurata", "Measured power", "Gemessene Leistung"],
        "gpu_tip"    => ["Calcule sur la carte graphique la mise en voxels du Lissage interne et de l'Enveloppe unique (retour automatique au processeur en cas d'échec)",
                         "Calcula en la tarjeta gráfica el vóxelizado del alisado interno y de la envoltura",
                         "Calcola sulla scheda grafica la voxelizzazione di levigatura interna e involucro",
                         "Runs the voxel stages of Internal cleanup and Single envelope on the graphics card (automatic CPU fallback)",
                         "Berechnet die Voxel-Schritte von Innen glätten und Einzelhülle auf der Grafikkarte"],
        "lissage"    => ["Lissage interne", "Alisado interno", "Levigatura interna", "Internal cleanup", "Innen glätten"],
        "lissage_tip"=> ["Supprime toutes les excroissances et parois internes (invisibles de l'extérieur) — à faire avant la réparation",
                         "Elimina todas las protuberancias y paredes internas — antes de la reparación",
                         "Rimuove tutte le escrescenze e pareti interne — prima della riparazione",
                         "Removes every internal protrusion and wall (not visible from outside) — do it before repairing",
                         "Entfernt alle inneren Auswüchse und Wände — vor der Reparatur ausführen"],
        "wireframe"  => ["Fil de fer", "Malla", "Wireframe", "Wireframe", "Drahtgitter"],
        "show_grid"  => ["Montrer la grille", "Mostrar rejilla", "Mostra griglia", "Show grid", "Raster zeigen"],
        "snap_size"  => ["Taille du Snap :", "Tamaño de snap:", "Passo snap:", "Snap size:", "Fangraster:"],
        "grid_size"  => ["Taille de la grille :", "Tamaño de rejilla:", "Dimensione griglia:", "Grid size:", "Rastergröße:"],
        "outils"     => ["Outils", "Herramientas", "Strumenti", "Tools", "Werkzeuge"],
        "creer"      => ["Créer", "Crear", "Crea", "Create", "Erstellen"],
        "taille"     => ["Taille :", "Tamaño:", "Dimensione:", "Size:", "Größe:"],
        "box"        => ["Boîte", "Caja", "Scatola", "Box", "Quader"],
        "sphere"     => ["Sphère", "Esfera", "Sfera", "Sphere", "Kugel"],
        "cylinder"   => ["Cylindre", "Cilindro", "Cilindro", "Cylinder", "Zylinder"],
        "cone"       => ["Cône", "Cono", "Cono", "Cone", "Kegel"],
        "torus"      => ["Tore", "Toro", "Toro", "Torus", "Torus"],
        "operations" => ["Opérations", "Operaciones", "Operazioni", "Operations", "Operationen"],
        "boolean"    => ["Booléen", "Booleano", "Booleano", "Boolean", "Boolesch"],
        "fillet"     => ["Congé", "Redondeo", "Raccordo", "Fillet", "Verrundung"],
        "extrude"    => ["Extrusion", "Extrusión", "Estrusione", "Extrude", "Extrusion"],
        "soon"       => ["à venir (B-rep OCCT)", "próximamente (B-rep OCCT)", "in arrivo (B-rep OCCT)", "coming soon (OCCT B-rep)", "demnächst (OCCT B-rep)"],
        "triangles"  => ["triangles", "triángulos", "triangoli", "triangles", "Dreiecke"],
        "langue"     => ["Langue", "Idioma", "Lingua", "Language", "Sprache"],
        "theme"      => ["Thème", "Tema", "Tema", "Theme", "Thema"],
        "sombre"     => ["Sombre", "Oscuro", "Scuro", "Dark", "Dunkel"],
        "clair"      => ["Clair", "Claro", "Chiaro", "Light", "Hell"],
        "relief"     => ["Relief / Texture", "Relieve / Textura", "Rilievo / Texture", "Relief / Texture", "Relief / Textur"],
        "mapping"    => ["Projection", "Proyección", "Proiezione", "Mapping", "Projektion"],
        "hauteur"    => ["Hauteur", "Altura", "Altezza", "Height", "Höhe"],
        "echelle"    => ["Échelle", "Escala", "Scala", "Scale", "Maßstab"],
        "inverser"   => ["Inverser", "Invertir", "Inverti", "Invert", "Umkehren"],
        "appliquer"  => ["Appliquer le relief", "Aplicar relieve", "Applica rilievo", "Apply relief", "Relief anwenden"],
        "map_planar" => ["Planaire Z", "Planar Z", "Planare Z", "Planar Z", "Planar Z"],
        "map_cyl"    => ["Cylindrique", "Cilíndrico", "Cilindrico", "Cylindrical", "Zylindrisch"],
        "map_tri"    => ["Triplanaire", "Triplanar", "Triplanare", "Triplanar", "Triplanar"],
        "redim"      => ["Taille", "Tamaño", "Taglia", "Resize", "Größe"],
        "largeur"    => ["Largeur", "Ancho", "Larghezza", "Width", "Breite"],
        "longueur"   => ["Longueur", "Largo", "Lunghezza", "Length", "Länge"],
        "hauteur_o"  => ["Hauteur", "Alto", "Altezza", "Height", "Höhe"],
        "chaine"     => ["Garder les proportions (chaîne)", "Mantener proporciones", "Mantieni proporzioni", "Keep proportions (chain)", "Proportionen halten"],
        "aucun_obj"  => ["Aucun objet sélectionné", "Ningún objeto", "Nessun oggetto", "No object selected", "Kein Objekt"],
        "centrer"    => ["Centrer", "Centrar", "Centra", "Center", "Zentr."],
        "face_only"  => ["Face sélectionnée seulement", "Solo cara seleccionada", "Solo faccia selezionata", "Selected face only", "Nur gewählte Fläche"],
        "repair_ai"  => ["Réparation A.I", "Reparación A.I", "Riparazione A.I", "A.I Repair", "A.I-Reparatur"],
        "repair_tip" => ["Recoud les bords et réoriente les faces (garde la géométrie d'origine)",
                         "Cose los bordes y reorienta las caras (conserva la geometría)",
                         "Ricuce i bordi e riorienta le facce (mantiene la geometria)",
                         "Stitches edges and re-orients faces (keeps original geometry)",
                         "Näht Ränder und richtet Flächen neu aus (behält die Geometrie)"],
        "repair_ai_tip" => ["Enveloppe l'objet en une surface fermée (bouche tous les trous) puis remplace l'ancien",
                            "Envuelve el objeto en una superficie cerrada y reemplaza el anterior",
                            "Avvolge l'oggetto in una superficie chiusa e sostituisce il precedente",
                            "Wraps the object into a closed surface (fills every hole) and replaces the old one",
                            "Umhüllt das Objekt mit einer geschlossenen Fläche und ersetzt das alte"],
        "wire_make"  => ["Convertir en fil de fer", "Convertir en alambre", "Converti in fil di ferro", "Convert to wireframe", "In Drahtgitter umwandeln"],
        "wire_diam"  => ["Ø barreau", "Ø barra", "Ø asta", "Strut Ø", "Stab-Ø"],
        "wire_nozzle"=> ["Ø buse", "Ø boquilla", "Ø ugello", "Nozzle Ø", "Düsen-Ø"],
        "wire_thick" => ["Épaisseur du fil", "Grosor del hilo", "Spessore filo", "Wire thickness", "Drahtstärke"],
        "wire_thicktip"=> ["Diamètre réel du fil (jamais inférieur au Ø buse mini imprimable)",
            "Diámetro real del hilo (nunca menor que el Ø boquilla)",
            "Diametro reale del filo (mai inferiore al Ø ugello)",
            "Actual wire diameter (never below the printable nozzle Ø)",
            "Tatsächlicher Drahtdurchmesser (nie kleiner als Düsen-Ø)"],
        "wire_res"   => ["Facettes barreau", "Facetas barra", "Facce asta", "Strut facets", "Stab-Facetten"],
        "wire_meshres"=> ["Résolution maillage", "Resolución malla", "Risoluzione mesh", "Mesh resolution", "Netz-Auflösung"],
        "wire_found" => ["Fondation (sol)", "Base (suelo)", "Base (suolo)", "Floor base", "Boden-Basis"],
        "wire_foundtip"=> ["Pose l'objet sur le lit et colle au sol les fils à moins de cette hauteur (adhérence impression)",
                           "Apoya en la cama y pega al suelo los hilos por debajo de esta altura",
                           "Appoggia sul piatto e incolla al suolo i fili sotto questa altezza",
                           "Rests on the bed and welds to the floor the threads below this height (print adhesion)",
                           "Legt aufs Bett und heftet Fäden unter dieser Höhe an den Boden"],
        "wire_meshtip"=> ["Réduit d'abord le maillage à cette finesse (règle la densité du fil de fer et évite le « trop d'arêtes »)",
                          "Reduce la malla a esta finura antes del alambre",
                          "Riduce prima la mesh a questa finezza",
                          "Reduces the mesh to this fineness first (sets wireframe density, avoids 'too many edges')",
                          "Reduziert das Netz zuerst auf diese Feinheit"],
        "wire_minwarn"=> ["Ø barreau relevé au Ø buse (mini imprimable)", "Ø barra ajustada al Ø boquilla", "Ø asta portato al Ø ugello", "Strut Ø raised to nozzle Ø (min printable)", "Stab-Ø auf Düsen-Ø angehoben"],
        "wire_tip"   => ["Transforme chaque arête en barreau solide (treillis imprimable). À utiliser sur un objet peu dense.",
                         "Convierte cada arista en una barra sólida (celosía imprimible). Úsalo en un objeto poco denso.",
                         "Trasforma ogni spigolo in un'asta solida (traliccio stampabile). Usare su un oggetto poco denso.",
                         "Turns each edge into a solid strut (printable lattice). Use on a low-poly object.",
                         "Macht aus jeder Kante einen soliden Stab (druckbares Gitter). Für wenig dichte Objekte."],
        "courbes"    => ["Courbes", "Curvas", "Curve", "Curves", "Kurven"],
        "solides"    => ["Solides", "Sólidos", "Solidi", "Solids", "Körper"],
        "t_lignes"   => ["Lignes", "Líneas", "Linee", "Lines", "Linien"],
        "t_libre"    => ["Libre", "Libre", "Libera", "Freeform", "Frei"],
        "t_rect"     => ["Rect", "Rect", "Rett", "Rect", "Recht"],
        "t_polys"    => ["Polys", "Polís", "Poli", "Polys", "Polys"],
        "t_cercle"   => ["Cercle", "Círculo", "Cerchio", "Circle", "Kreis"],
        "t_arc"      => ["Arc", "Arco", "Arco", "Arc", "Bogen"],
        "t_ellipse"  => ["Ellipse", "Elipse", "Ellisse", "Ellipse", "Ellipse"],
        "t_plus"     => ["Plus", "Más", "Altro", "More", "Mehr"],
        "s_plan"     => ["Plan", "Plano", "Piano", "Plane", "Ebene"],
        "s_texte"    => ["Texte", "Texto", "Testo", "Text", "Text"],
        "sketch_soon"=> ["Esquisse 2D à venir", "Boceto 2D próximamente", "Schizzo 2D in arrivo", "2D sketch coming soon", "2D-Skizze demnächst"],
        "text_soon"  => ["Texte 3D à venir", "Texto 3D próximamente", "Testo 3D in arrivo", "3D text coming soon", "3D-Text demnächst"],
        // infobulles
        "g_aff"      => ["Affichage", "Visualización", "Visualizzazione", "Display", "Anzeige"],
        "g_etat0"    => ["Grille et axes", "Rejilla y ejes", "Griglia e assi", "Grid and axes", "Raster und Achsen"],
        "g_etat1"    => ["Grille seule", "Solo rejilla", "Solo griglia", "Grid only", "Nur Raster"],
        "g_etat2"    => ["Axes seuls", "Solo ejes", "Solo assi", "Axes only", "Nur Achsen"],
        "g_etat3"    => ["Ni grille ni axes", "Ni rejilla ni ejes", "Né griglia né assi", "No grid, no axes", "Weder Raster noch Achsen"],
        "g_cycle"    => ["Clic : grille + axes > grille seule > axes seuls > rien",
                         "Clic: rejilla + ejes > solo rejilla > solo ejes > nada",
                         "Clic: griglia + assi > solo griglia > solo assi > niente",
                         "Click: grid + axes > grid only > axes only > none",
                         "Klick: Raster + Achsen > nur Raster > nur Achsen > nichts"],
        "lum_tip"    => ["Choisissez la direction de l'éclairage de la scène",
                         "Elija la dirección de la iluminación de la escena",
                         "Scegli la direzione dell'illuminazione della scena",
                         "Choose the direction of the scene lighting",
                         "Wählen Sie die Richtung der Szenenbeleuchtung"],
        "grav"       => ["Gravité", "Gravedad", "Gravità", "Gravity", "Schwerkraft"],
        "sortir_btn" => ["SORTIR", "SALIR", "ESCI", "EXIT", "BEENDEN"],
        "sortir_tip" => ["Quitter ce mode (OBJ-AXE / BOOLÉENS) et revenir à l'affichage normal", "Salir de este modo (OBJ-AXE / BOOLEANOS) y volver a la vista normal", "Esci da questa modalità (OBJ-AXE / BOOLEANE) e torna alla vista normale", "Leave this mode (OBJ-AXE / BOOLEANS) and return to the normal view", "Diesen Modus (OBJ-AXE / BOOLESCH) verlassen und zur normalen Ansicht zurückkehren"],
        "surf_btn"   => ["SURFACE", "SUPERFICIE", "SUPERFICIE", "SURFACE", "FLÄCHE"],
        "surf_tip"   => ["Choisir la face plane sur laquelle poser l'objet : les faces possibles s'affichent en rouge, un clic sur l'une d'elles pose l'objet dessus (avec la chute)",
                         "Elegir la cara plana sobre la que apoyar el objeto: las caras posibles aparecen en rojo, un clic en una de ellas apoya el objeto (con la caída)",
                         "Scegli la faccia piana su cui appoggiare l'oggetto: le facce possibili appaiono in rosso, un clic su una di esse appoggia l'oggetto (con la caduta)",
                         "Choose the flat face to lay the object on: possible faces turn red, clicking one lays the object on it (with the drop)",
                         "Ebene Fläche zum Auflegen wählen: mögliche Flächen werden rot, ein Klick legt das Objekt darauf (mit Fall)"],
        "surf_nosel" => ["SURFACE : sélectionne d'abord un objet (clic gauche)", "SUPERFICIE: selecciona primero un objeto (clic izquierdo)", "SUPERFICIE: seleziona prima un oggetto (clic sinistro)", "SURFACE: select an object first (left click)", "FLÄCHE: zuerst ein Objekt wählen (Linksklick)"],
        "surf_analyse" => ["Analyse des surfaces", "Análisis de superficies", "Analisi delle superfici", "Surface analysis", "Flächenanalyse"],
        "surf_encours" => ["Recherche des faces planes où poser l'objet", "Buscando las caras planas donde apoyar el objeto", "Ricerca delle facce piane su cui appoggiare l'oggetto", "Looking for flat faces to lay the object on", "Suche nach ebenen Flächen zum Auflegen"],
        "surf_trouvees" => ["surfaces où poser l'objet (de la plus grande à la plus petite)", "superficies donde apoyar el objeto (de mayor a menor)", "superfici su cui appoggiare l'oggetto (dalla più grande)", "surfaces to lay the object on (largest first)", "Flächen zum Auflegen (größte zuerst)"],
        "surf_poser_tip" => ["Poser l'objet sur cette surface (avec la chute)", "Apoyar el objeto sobre esta superficie (con la caída)", "Appoggia l'oggetto su questa superficie (con la caduta)", "Lay the object on this surface (with the drop)", "Objekt auf diese Fläche legen (mit Fall)"],
        "surf_o_dessous" => ["dessous (actuel)", "debajo (actual)", "sotto (attuale)", "bottom (current)", "unten (aktuell)"],
        "surf_o_dessus" => ["dessus", "arriba", "sopra", "top", "oben"],
        "surf_o_gauche" => ["côté gauche", "lado izquierdo", "lato sinistro", "left side", "linke Seite"],
        "surf_o_droite" => ["côté droit", "lado derecho", "lato destro", "right side", "rechte Seite"],
        "surf_o_avant" => ["avant", "delante", "davanti", "front", "vorne"],
        "surf_o_arriere" => ["arrière", "detrás", "dietro", "back", "hinten"],
        "surf_o_incline" => ["face inclinée", "cara inclinada", "faccia inclinata", "slanted face", "schräge Fläche"],
        "surf_aucune"=> ["Aucune face plane sur laquelle poser cet objet", "Ninguna cara plana sobre la que apoyar este objeto", "Nessuna faccia piana su cui appoggiare l'oggetto", "No flat face to lay this object on", "Keine ebene Fläche zum Auflegen"],
        "surf_choisir"=> ["Clique une face rouge : elle sera posée sur le plateau (Échap = annuler)", "Haz clic en una cara roja: se apoyará en la bandeja (Esc = cancelar)", "Clicca una faccia rossa: verrà appoggiata sul piatto (Esc = annulla)", "Click a red face: it will be laid on the plate (Esc = cancel)", "Rote Fläche anklicken: sie wird auf die Platte gelegt (Esc = abbrechen)"],
        "grav_btn"   => ["GRAVITÉ", "GRAVEDAD", "GRAVITÀ", "GRAVITY", "SCHWERKRAFT"],
        "grav_tip"   => ["Lâche l'objet comme en vrai : il tombe sur le plateau, rebondit, roule et bascule jusqu'à une position stable (sur ses appuis, sur le dos, sur le côté…)",
                         "Suelta el objeto como de verdad: cae en la bandeja, rebota, rueda y vuelca hasta una posición estable (sobre sus apoyos, de espaldas, de lado…)",
                         "Lascia cadere l'oggetto come nella realtà: cade sul piatto, rimbalza, rotola e si ribalta fino a una posizione stabile (sugli appoggi, sulla schiena, di lato…)",
                         "Drops the object as in real life: it falls on the plate, bounces, rolls and tips over until it rests stably (on its feet, on its back, on its side…)",
                         "Lässt das Objekt wie in echt fallen: es fällt auf die Platte, springt, rollt und kippt bis in eine stabile Lage (auf den Füßen, auf dem Rücken, auf der Seite…)"],
        "grav_ok"    => ["Objet posé par gravité sur le plan XY", "Objeto apoyado por gravedad en el plano XY", "Oggetto appoggiato per gravità sul piano XY", "Object dropped by gravity onto the XY plane", "Objekt per Schwerkraft auf die XY-Ebene gelegt"],
        "grav_novol" => ["objet sans volume : impossible de le poser", "objeto sin volumen: imposible apoyarlo", "oggetto senza volume: impossibile appoggiarlo", "object has no volume: cannot be set down", "Objekt ohne Volumen: kann nicht abgelegt werden"],
        "vue_tip"    => ["Choisir la vue : 3D, 4 vues, dessus, face, droite",
                         "Elegir la vista: 3D, 4 vistas, arriba, frente, derecha",
                         "Scegli la vista: 3D, 4 viste, alto, fronte, destra",
                         "Choose the view: 3D, 4 views, top, front, right",
                         "Ansicht wählen: 3D, 4 Ansichten, oben, vorne, rechts"],
        "taille_tip" => ["Taille de l'objet sélectionné\n• Clic : saisir la largeur / la hauteur\n• Molette sur le bouton : pousser = agrandir, tirer = rapetisser\n  (proportions conservées, objet posé au même endroit)",
                         "Tamaño del objeto seleccionado\n• Clic: introducir el ancho / la altura\n• Rueda sobre el botón: empujar = agrandar, tirar = reducir\n  (proporciones conservadas, objeto en el mismo sitio)",
                         "Dimensione dell'oggetto selezionato\n• Clic: inserire larghezza / altezza\n• Rotella sul pulsante: spingere = ingrandire, tirare = rimpicciolire\n  (proporzioni mantenute, oggetto nello stesso punto)",
                         "Size of the selected object\n• Click: enter the width / height\n• Mouse wheel on the button: push = enlarge, pull = shrink\n  (proportions kept, object stays in place)",
                         "Größe des gewählten Objekts\n• Klick: Breite / Höhe eingeben\n• Mausrad auf der Schaltfläche: schieben = vergrößern, ziehen = verkleinern\n  (Proportionen bleiben, Objekt bleibt an seinem Platz)"],
        "objaxe_tip" => ["Double-clic droit sur l'objet pour passer en mode rotation d'objet",
                         "Doble clic derecho sobre el objeto para pasar al modo rotación de objeto",
                         "Doppio clic destro sull'oggetto per passare alla modalità rotazione oggetto",
                         "Double right-click on the object to switch to object rotation mode",
                         "Doppelter Rechtsklick auf das Objekt wechselt in den Objekt-Drehmodus"],
        "objaxe_nosel"=> ["OBJ-AXE : sélectionnez d'abord un objet (clic gauche)", "OBJ-AXE: seleccione primero un objeto (clic izquierdo)", "OBJ-AXE: seleziona prima un oggetto (clic sinistro)", "OBJ-AXE: select an object first (left click)", "OBJ-AXE: zuerst ein Objekt wählen (Linksklick)"],
        "u_mm"       => ["millimètres (mm)", "milímetros (mm)", "millimetri (mm)", "millimeters (mm)", "Millimeter (mm)"],
        "u_cm"       => ["centimètres (cm)", "centímetros (cm)", "centimetri (cm)", "centimeters (cm)", "Zentimeter (cm)"],
        "u_m"        => ["mètres (m)", "metros (m)", "metri (m)", "meters (m)", "Meter (m)"],
        "u_in"       => ["pouces (in)", "pulgadas (in)", "pollici (in)", "inches (in)", "Zoll (in)"],
        "u_ft"       => ["pieds (ft)", "pies (ft)", "piedi (ft)", "feet (ft)", "Fuß (ft)"],
        "u_yd"       => ["yards (yd)", "yardas (yd)", "iarde (yd)", "yards (yd)", "Yards (yd)"],
        "u_metrique" => ["Métrique", "Métrico", "Metrico", "Metric", "Metrisch"],
        "u_anglais"  => ["Notation anglaise", "Notación inglesa", "Notazione inglese", "Imperial (US)", "Angloamerikanisch"],
        "tous_modeles"=> ["Tous les modèles", "Todos los modelos", "Tutti i modelli", "All models", "Alle Modelle"],
        // messages
        "importe"    => ["Importé", "Importado", "Importato", "Imported", "Importiert"],
        "sauve_ok"   => ["Sauvegardé", "Guardado", "Salvato", "Saved", "Gespeichert"],
        "err_sauve"  => ["Sauvegarde impossible", "No se pudo guardar", "Salvataggio impossibile", "Cannot save", "Speichern nicht möglich"],
        "err_import" => ["Import impossible", "No se pudo importar", "Importazione impossibile", "Cannot import", "Import nicht möglich"],
        "err_photo"  => ["Copie d'écran impossible", "No se pudo capturar la pantalla", "Istantanea impossibile", "Cannot take screenshot", "Bildschirmfoto nicht möglich"],
        "err_assoc"  => ["Associations impossibles", "No se pudieron asociar los archivos", "Associazioni impossibili", "Cannot set file associations", "Dateizuordnung nicht möglich"],
        "sc_illisible"  => ["Conception SolidCraft illisible", "Diseño SolidCraft ilegible", "Progetto SolidCraft illeggibile", "Unreadable SolidCraft design", "SolidCraft-Konstruktion unlesbar"],
        "sc_parametrique" => ["Conception SolidCraft purement paramétrique : elle ne contient pas de géométrie enregistrée (seulement l'historique de construction). Installe SolidCraft (3D-Patch s'en servira pour la calculer), ou dans SolidCraft exporte-la en STEP ou 3MF.",
                            "Diseño SolidCraft puramente paramétrico: no contiene geometría guardada (solo el historial). Instala SolidCraft (3D-Patch lo usará para calcularlo), o expórtalo a STEP o 3MF desde SolidCraft.",
                            "Progetto SolidCraft puramente parametrico: non contiene geometria salvata (solo la cronologia). Installa SolidCraft (3D-Patch lo userà per calcolarlo), o esportalo in STEP o 3MF da SolidCraft.",
                            "Purely parametric SolidCraft design: it holds no saved geometry (only its construction history). Install SolidCraft (3D-Patch will use it to compute it), or export it to STEP or 3MF from SolidCraft.",
                            "Rein parametrische SolidCraft-Konstruktion: keine gespeicherte Geometrie (nur der Verlauf). SolidCraft installieren (3D-Patch berechnet sie damit) oder in SolidCraft als STEP oder 3MF exportieren."],
        "sc_vide"       => ["Conception SolidCraft vide", "Diseño SolidCraft vacío", "Progetto SolidCraft vuoto", "Empty SolidCraft design", "Leere SolidCraft-Konstruktion"],
        "cao_illisible" => ["Fichier CAO illisible", "Archivo CAD ilegible", "File CAD illeggibile", "Unreadable CAD file", "CAD-Datei unlesbar"],
        "bool_btn"      => ["Booléens", "Booleanos", "Booleane", "Booleans", "Boolesch"],
        "bool_tip"      => ["Booléens (comme 3D Builder) : fusionner, soustraire ou intersecter les objets sélectionnés\nRemplace le panneau d'outils ; un 2e clic le rétablit",
                            "Booleanas (como 3D Builder): fusionar, restar o intersecar los objetos seleccionados\nSustituye el panel de herramientas; un 2.º clic lo restablece",
                            "Booleane (come 3D Builder): unire, sottrarre o intersecare gli oggetti selezionati\nSostituisce il pannello strumenti; un 2° clic lo ripristina",
                            "Booleans (like 3D Builder): merge, subtract or intersect the selected objects\nReplaces the tool panel; click again to restore it",
                            "Boolesch (wie 3D Builder): gewählte Objekte vereinigen, subtrahieren oder schneiden\nErsetzt das Werkzeugfeld; erneuter Klick stellt es wieder her"],
        "bool_retour"   => ["Outils", "Herramientas", "Strumenti", "Tools", "Werkzeuge"],
        "bool_aide"     => ["Sélectionne au moins deux objets : clic sur le premier, puis Ctrl+clic sur les autres (Ctrl+A = tous). Pour SOUSTRAIRE, l'objet sélectionné en dernier est retiré des autres.",
                            "Selecciona al menos dos objetos: clic en el primero y Ctrl+clic en los demás (Ctrl+A = todos). Para RESTAR, el último objeto seleccionado se quita de los demás.",
                            "Seleziona almeno due oggetti: clic sul primo, poi Ctrl+clic sugli altri (Ctrl+A = tutti). Per SOTTRARRE, l'ultimo oggetto selezionato viene tolto dagli altri.",
                            "Select at least two objects: click the first, then Ctrl+click the others (Ctrl+A = all). To SUBTRACT, the object selected last is removed from the others.",
                            "Mindestens zwei Objekte wählen: erstes anklicken, dann Strg+Klick auf die anderen (Strg+A = alle). Beim SUBTRAHIEREN wird das zuletzt gewählte Objekt von den anderen abgezogen."],
        "bool_obj"      => ["Objet", "Objeto", "Oggetto", "Object", "Objekt"],
        "bool_nsel"     => ["Objets sélectionnés", "Objetos seleccionados", "Oggetti selezionati", "Selected objects", "Gewählte Objekte"],
        "bool_dernier"  => ["dernier : retiré par SOUSTRAIRE", "último: quitado por RESTAR", "ultimo: tolto da SOTTRARRE", "last: removed by SUBTRACT", "zuletzt: wird bei SUBTRAHIEREN abgezogen"],
        "bool_tout"     => ["Tout sélectionner", "Seleccionar todo", "Seleziona tutto", "Select all", "Alles auswählen"],
        "bool_union"    => ["Fusionner", "Fusionar", "Unisci", "Merge", "Vereinigen"],
        "bool_soustr"   => ["Soustraire", "Restar", "Sottrai", "Subtract", "Subtrahieren"],
        "bool_inter"    => ["Intersection", "Intersección", "Intersezione", "Intersect", "Schnittmenge"],
        "bool_union_tip"  => ["Fusionne tous les objets sélectionnés en un seul", "Fusiona todos los objetos seleccionados en uno", "Fonde tutti gli oggetti selezionati in uno", "Merges all selected objects into one", "Verschmilzt alle gewählten Objekte zu einem"],
        "bool_soustr_tip" => ["Retire l'objet sélectionné en dernier des autres (perçage, encoche…)", "Quita el último objeto seleccionado de los demás (taladro, muesca…)", "Toglie l'ultimo oggetto selezionato dagli altri (foro, tacca…)", "Cuts the object selected last out of the others (hole, notch…)", "Schneidet das zuletzt gewählte Objekt aus den anderen (Bohrung, Kerbe…)"],
        "bool_inter_tip"  => ["Ne garde que la partie commune à tous les objets sélectionnés", "Conserva solo la parte común a todos los objetos seleccionados", "Mantiene solo la parte comune a tutti gli oggetti selezionati", "Keeps only the part all selected objects share", "Behält nur den allen gewählten Objekten gemeinsamen Teil"],
        "bool_garder"   => ["Conserver l'objet soustrait", "Conservar el objeto restado", "Mantieni l'oggetto sottratto", "Keep the subtracted object", "Abgezogenes Objekt behalten"],
        "bool_garder_tip" => ["Coché : l'objet retiré par SOUSTRAIRE reste dans la scène", "Marcado: el objeto restado permanece en la escena", "Spuntato: l'oggetto sottratto resta nella scena", "Checked: the object used by SUBTRACT stays in the scene", "Aktiviert: das abgezogene Objekt bleibt in der Szene"],
        "bool_choisir"  => ["Sélectionne au moins deux objets (Ctrl+clic)", "Selecciona al menos dos objetos (Ctrl+clic)", "Seleziona almeno due oggetti (Ctrl+clic)", "Select at least two objects (Ctrl+click)", "Mindestens zwei Objekte wählen (Strg+Klick)"],
        "coupe_btn"     => ["Découpe", "Corte", "Taglio", "Cut", "Schnitt"],
        "coupe_tip"     => ["Ciseaux : trace une ligne de coupe, un cadre rouge s'élève perpendiculairement et coupe l'objet",
                            "Tijeras: traza una línea de corte; un marco rojo se eleva perpendicularmente y corta el objeto",
                            "Forbici: traccia una linea di taglio; un riquadro rosso si alza perpendicolarmente e taglia l'oggetto",
                            "Scissors: draw a cut line; a red frame rises perpendicular to it and cuts the object",
                            "Schere: Schnittlinie zeichnen; ein roter Rahmen steigt senkrecht auf und schneidet das Objekt"],
        "coupe_aide"    => ["Clique sur les ciseaux, puis 1er clic = départ du trait, 2e clic = arrivée (longueur L ; vue 3D ou Dessus = sur la grille, vues Face / Côté = dans leur plan). Déplace ensuite la souris pour déplier l autour du trait (roue d'inclinaison, aimantée tous les 45°) et clique une 3e fois. Le cadre rouge est le plan qui coupe.",
                            "Haz clic en las tijeras: 1.er clic = inicio del trazo, 2.º clic = final (longitud L; vista 3D o Superior = sobre la rejilla, vistas Frontal / Lateral = en su plano). Mueve luego el ratón para desplegar l alrededor del trazo (rueda de inclinación, imantada cada 45°) y haz un 3.er clic. El marco rojo es el plano que corta.",
                            "Clicca sulle forbici: 1° clic = inizio del tratto, 2° clic = arrivo (lunghezza L; vista 3D o Dall'alto = sulla griglia, viste Frontale / Laterale = nel loro piano). Muovi poi il mouse per aprire l attorno al tratto (ruota d'inclinazione, magnetica ogni 45°) e fai un 3° clic. Il riquadro rosso è il piano che taglia.",
                            "Click the scissors: 1st click = line start, 2nd click = end (length L; 3D or Top view = on the grid, Front / Side views = in their plane). Then move the mouse to unfold l around the line (tilt wheel, snapping every 45°) and click a 3rd time. The red frame is the cutting plane.",
                            "Schere anklicken: 1. Klick = Linienanfang, 2. Klick = Ende (Länge L; 3D- oder Draufsicht = auf dem Raster, Vorder-/Seitenansicht = in ihrer Ebene). Dann die Maus bewegen, um l um die Linie aufzuklappen (Neigungsrad, rastet alle 45° ein), und ein 3. Mal klicken. Der rote Rahmen ist die Schnittebene."],
        "coupe_t1"      => ["Clique le départ", "Clic: inicio", "Clic: inizio", "Click the start", "Start klicken"],
        "coupe_t2"      => ["Clique l'arrivée", "Clic: final", "Clic: arrivo", "Click the end", "Ende klicken"],
        "coupe_t3"      => ["Déplie l, clique", "Despliega l, clic", "Apri l, clic", "Unfold l, click", "l aufklappen, klicken"],
        "coupe_incl"    => ["Inclinaison", "Inclinación", "Inclinazione", "Tilt", "Neigung"],
        "coupe_incl_tip"=> ["Rotation du cadre autour du trait L : 90° = perpendiculaire au plan de tracé, 0° = à plat dedans",
                            "Giro del marco alrededor del trazo L: 90° = perpendicular al plano de trazado, 0° = plano en él",
                            "Rotazione del riquadro attorno al tratto L: 90° = perpendicolare al piano di tracciamento, 0° = piatto in esso",
                            "Frame rotation around line L: 90° = perpendicular to the drawing plane, 0° = flat in it",
                            "Drehung des Rahmens um die Linie L: 90° = senkrecht zur Zeichenebene, 0° = flach darin"],
        "coupe_l_tip"   => ["Longueur dépliée du cadre depuis le trait (0 = toute la hauteur de la scène). La coupe traverse toujours toute la pièce.",
                            "Longitud desplegada del marco desde el trazo (0 = toda la altura de la escena). El corte siempre atraviesa toda la pieza.",
                            "Lunghezza aperta del riquadro dal tratto (0 = tutta l'altezza della scena). Il taglio attraversa sempre tutto il pezzo.",
                            "Unfolded length of the frame from the line (0 = full scene height). The cut always goes through the whole part.",
                            "Aufgeklappte Länge des Rahmens ab der Linie (0 = ganze Szenenhöhe). Der Schnitt geht immer durch das ganze Teil."],
        "coupe_objet"   => ["Le cadre est un objet : clique-le pour le sélectionner (contour jaune), fais-le glisser pour le déplacer, touche Suppr pour l'enlever. Il disparaît après la coupe.",
                            "El marco es un objeto: haz clic para seleccionarlo (contorno amarillo), arrástralo para moverlo, Supr para quitarlo. Desaparece tras el corte.",
                            "Il riquadro è un oggetto: cliccalo per selezionarlo (contorno giallo), trascinalo per spostarlo, Canc per toglierlo. Sparisce dopo il taglio.",
                            "The frame is an object: click it to select it (yellow outline), drag it to move it, Delete to remove it. It disappears after the cut.",
                            "Der Rahmen ist ein Objekt: anklicken zum Auswählen (gelber Rand), ziehen zum Verschieben, Entf zum Entfernen. Nach dem Schnitt verschwindet er."],
        "coupe_angle"   => ["Angle", "Ángulo", "Angolo", "Angle", "Winkel"],
        "coupe_trait"   => ["Trait", "Trazo", "Tratto", "Line", "Linie"],
        "coupe_pos"     => ["Décalage", "Desplazamiento", "Spostamento", "Offset", "Versatz"],
        "coupe_pos_tip" => ["Déplace le cadre perpendiculairement à lui-même", "Desplaza el marco perpendicularmente", "Sposta il riquadro perpendicolarmente", "Moves the frame perpendicular to itself", "Verschiebt den Rahmen senkrecht zu sich selbst"],
        "coupe_garder"  => ["Garder", "Conservar", "Mantieni", "Keep", "Behalten"],
        "coupe_deux"    => ["Les deux", "Ambos", "Entrambi", "Both", "Beide"],
        "coupe_a"       => ["Côté flèche", "Lado flecha", "Lato freccia", "Arrow side", "Pfeilseite"],
        "coupe_b"       => ["Autre côté", "Otro lado", "Altro lato", "Other side", "Andere Seite"],
        "coupe_ecart"   => ["Écarter les morceaux", "Separar las piezas", "Separa i pezzi", "Move the pieces apart", "Teile auseinanderrücken"],
        "coupe_go"      => ["Couper", "Cortar", "Taglia", "Cut", "Schneiden"],
        "coupe_go_tip"  => ["Coupe les objets sélectionnés (ou tous) le long du cadre rouge", "Corta los objetos seleccionados (o todos) a lo largo del marco rojo", "Taglia gli oggetti selezionati (o tutti) lungo il riquadro rosso", "Cuts the selected objects (or all) along the red frame", "Schneidet die gewählten (oder alle) Objekte entlang des roten Rahmens"],
        "coupe_effacer" => ["Effacer le cadre", "Borrar el marco", "Cancella il riquadro", "Clear the frame", "Rahmen löschen"],
        "coupe_rate"    => ["Le cadre ne traverse pas l'objet : place la ligne à travers la pièce", "El marco no atraviesa el objeto: coloca la línea a través de la pieza", "Il riquadro non attraversa l'oggetto: metti la linea attraverso il pezzo", "The frame does not cross the object: draw the line across the part", "Der Rahmen schneidet das Objekt nicht: Linie quer durch das Teil ziehen"],
        "coupe_morceaux"=> ["morceaux", "piezas", "pezzi", "pieces", "Teile"],
        "at_quitter" => ["Quitter les textures", "Salir de texturas", "Esci dalle texture", "Leave textures", "Texturen verlassen"],
        "at_cible" => ["Clique sur l'objet à habiller (ou sélectionne-le) pour commencer", "Haz clic en el objeto a texturizar (o selecciónalo) para empezar", "Clicca sull'oggetto da rivestire (o selezionalo) per iniziare", "Click the object to texture (or select it) to start", "Klicke auf das zu texturierende Objekt (oder wähle es), um zu beginnen"],
        "at_s_texture" => ["1 · Texture", "1 · Textura", "1 · Texture", "1 · Texture", "1 · Textur"],
        "at_vide" => ["(aucune texture)", "(sin textura)", "(nessuna texture)", "(no texture)", "(keine Textur)"],
        "at_mes_images" => ["Mes images", "Mis imágenes", "Le mie immagini", "My images", "Meine Bilder"],
        "at_ajouter_image" => ["+ Ajouter une image…", "+ Añadir una imagen…", "+ Aggiungi un'immagine…", "+ Add an image…", "+ Bild hinzufügen…"],
        "at_ajouter_image_tip" => ["PNG, JPEG ou BMP : le clair forme les bosses, le sombre les creux. L'image reste dans ta bibliothèque.", "PNG, JPEG o BMP: lo claro forma los relieves, lo oscuro los huecos. La imagen queda en tu biblioteca.", "PNG, JPEG o BMP: il chiaro forma i rilievi, lo scuro gli incavi. L'immagine resta nella tua libreria.", "PNG, JPEG or BMP: light makes bumps, dark makes hollows. The image stays in your library.", "PNG, JPEG oder BMP: Hell ergibt Erhebungen, Dunkel Vertiefungen. Das Bild bleibt in deiner Bibliothek."],
        "at_retirer_image" => ["Retirer de ma bibliothèque", "Quitar de mi biblioteca", "Rimuovi dalla mia libreria", "Remove from my library", "Aus meiner Bibliothek entfernen"],
        "at_taille" => ["Taille", "Tamaño", "Dimensione", "Size", "Größe"],
        "at_taille_tip" => ["Largeur d'un motif sur l'objet", "Anchura de un motivo sobre el objeto", "Larghezza di un motivo sull'oggetto", "Width of one pattern tile on the object", "Breite eines Musters auf dem Objekt"],
        "at_prof" => ["Profondeur", "Profundidad", "Profondità", "Depth", "Tiefe"],
        "at_prof_tip" => ["Hauteur entre le creux et le sommet du relief", "Altura entre el hueco y la cima del relieve", "Altezza tra l'incavo e la cima del rilievo", "Height between the hollow and the top of the relief", "Höhe zwischen Vertiefung und Spitze des Reliefs"],
        "at_relief" => ["En relief", "En relieve", "In rilievo", "Raised", "Erhaben"],
        "at_grave" => ["Gravé", "Grabado", "Inciso", "Engraved", "Graviert"],
        "at_s_pinceau" => ["2 · Exclure : zones lisses", "2 · Excluir: zonas lisas", "2 · Escludere: zone lisce", "2 · Exclude: smooth areas", "2 · Ausschließen: glatte Bereiche"],
        "at_peindre" => ["Pinceau", "Pincel", "Pennello", "Brush", "Pinsel"],
        "at_peindre_tip" => ["Peins en rouge les parties de l'objet qui doivent rester lisses (Maj : gommer)", "Pinta en rojo las partes del objeto que deben quedar lisas (Mayús: borrar)", "Dipingi in rosso le parti dell'oggetto che devono restare lisce (Maiusc: cancella)", "Paint in red the parts of the object that must stay smooth (Shift: erase)", "Male rot die Teile des Objekts, die glatt bleiben sollen (Umschalt: radieren)"],
        "at_gommer" => ["Gomme", "Goma", "Gomma", "Eraser", "Radierer"],
        "at_gommer_tip" => ["Rends la texture aux parties peintes en rouge", "Devuelve la textura a las partes pintadas en rojo", "Restituisci la texture alle parti dipinte in rosso", "Give the texture back to the parts painted red", "Gib den rot bemalten Teilen die Textur zurück"],
        "at_rayon" => ["Pinceau", "Pincel", "Pennello", "Brush", "Pinsel"],
        "at_dessus" => ["Dessus lisse", "Arriba lisa", "Sopra liscio", "Smooth top", "Oben glatt"],
        "at_dessous" => ["Dessous lisse", "Abajo lisa", "Sotto liscio", "Smooth bottom", "Unten glatt"],
        "at_lisse_tip" => ["Garde lisses les faces tournées vers le haut / vers le bas (le dessous reste bien à plat sur le plateau)", "Mantiene lisas las caras orientadas hacia arriba / abajo (la base queda plana sobre la cama)", "Mantiene lisce le facce rivolte verso l'alto / il basso (la base resta piatta sul piatto)", "Keeps faces pointing up / down smooth (the bottom stays flat on the bed)", "Hält nach oben / unten zeigende Flächen glatt (die Unterseite bleibt flach auf dem Druckbett)"],
        "at_effacer" => ["Tout effacer", "Borrar todo", "Cancella tutto", "Clear all", "Alles löschen"],
        "at_effacer_tip" => ["Retire toutes les zones peintes", "Quita todas las zonas pintadas", "Rimuove tutte le zone dipinte", "Removes all painted areas", "Entfernt alle bemalten Bereiche"],
        "at_aide_pinceau" => ["Clic gauche sur l'objet : pinceau · Maj : outil inverse · clic droit : tourner la vue", "Clic izquierdo en el objeto: pincel · Mayús: herramienta inversa · clic derecho: girar la vista", "Clic sinistro sull'oggetto: pennello · Maiusc: strumento inverso · clic destro: ruota la vista", "Left-click the object: brush · Shift: opposite tool · right-click: turn the view", "Linksklick aufs Objekt: Pinsel · Umschalt: Gegenwerkzeug · Rechtsklick: Ansicht drehen"],
        "at_appliquer" => ["Appliquer la texture", "Aplicar la textura", "Applica la texture", "Apply the texture", "Textur anwenden"],
        "at_appliquer_tip" => ["Grave la texture dans l'objet (annulable)", "Graba la textura en el objeto (se puede deshacer)", "Incide la texture nell'oggetto (annullabile)", "Bakes the texture into the object (can be undone)", "Prägt die Textur ins Objekt (rückgängig machbar)"],
        "at_calcul" => ["Aperçu en cours de calcul…", "Calculando la vista previa…", "Anteprima in calcolo…", "Computing preview…", "Vorschau wird berechnet…"],
        "at_s_motif" => ["Réglages du motif", "Ajustes del motivo", "Impostazioni del motivo", "Pattern settings", "Mustereinstellungen"],
        "at_joint" => ["Joints", "Juntas", "Giunti", "Joints", "Fugen"],
        "at_arrondi" => ["Arrondi", "Redondeo", "Arrotondamento", "Roundness", "Rundung"],
        "at_densite" => ["Densité", "Densidad", "Densità", "Density", "Dichte"],
        "at_variante" => ["Variante", "Variante", "Variante", "Variant", "Variante"],
        "at_inverser" => ["Inverser creux et bosses", "Invertir huecos y relieves", "Inverti incavi e rilievi", "Swap hollows and bumps", "Vertiefungen und Erhebungen tauschen"],
        "at_contraste" => ["Contraste", "Contraste", "Contrasto", "Contrast", "Kontrast"],
        "at_s_placement" => ["Placement", "Colocación", "Posizionamento", "Placement", "Platzierung"],
        "at_projection" => ["Enveloppe", "Envoltura", "Avvolgimento", "Wrapping", "Umhüllung"],
        "at_axe" => ["Axe", "Eje", "Asse", "Axis", "Achse"],
        "at_fondu" => ["Fondu entre faces", "Fundido entre caras", "Sfumatura tra facce", "Blend between faces", "Übergang zwischen Flächen"],
        "at_rotation" => ["Rotation", "Rotación", "Rotazione", "Rotation", "Drehung"],
        "at_decalage_u" => ["Décalage horizontal", "Desplazamiento horizontal", "Spostamento orizzontale", "Horizontal offset", "Horizontaler Versatz"],
        "at_decalage_v" => ["Décalage vertical", "Desplazamiento vertical", "Spostamento verticale", "Vertical offset", "Vertikaler Versatz"],
        "at_proportion" => ["Garder les proportions", "Mantener proporciones", "Mantieni le proporzioni", "Keep proportions", "Proportionen beibehalten"],
        "at_taille_v" => ["Hauteur du motif", "Altura del motivo", "Altezza del motivo", "Pattern height", "Musterhöhe"],
        "at_s_finesse" => ["Finesse", "Finura", "Finezza", "Fineness", "Feinheit"],
        "at_auto" => ["Réglage automatique", "Ajuste automático", "Regolazione automatica", "Automatic setting", "Automatische Einstellung"],
        "at_resolution" => ["Plus petit détail", "Detalle más pequeño", "Dettaglio più piccolo", "Smallest detail", "Kleinstes Detail"],
        "at_resolution_tip" => ["Longueur d'arête la plus fine produite : plus petit = plus net, plus lourd", "Arista más fina producida: más pequeño = más nítido, más pesado", "Spigolo più fine prodotto: più piccolo = più nitido, più pesante", "Finest edge produced: smaller = sharper, heavier", "Feinste erzeugte Kante: kleiner = schärfer, schwerer"],
        "at_tolerance" => ["Précision", "Precisión", "Precisione", "Accuracy", "Genauigkeit"],
        "at_tolerance_tip" => ["Écart toléré entre le relief voulu et le relief obtenu : plus petit = plus fidèle, plus de triangles", "Desviación tolerada entre el relieve deseado y el obtenido: más pequeño = más fiel, más triángulos", "Scarto tollerato tra rilievo voluto e ottenuto: più piccolo = più fedele, più triangoli", "Allowed gap between wanted and obtained relief: smaller = more faithful, more triangles", "Erlaubte Abweichung zwischen gewolltem und erhaltenem Relief: kleiner = genauer, mehr Dreiecke"],
        "at_max" => ["Triangles au maximum", "Triángulos como máximo", "Triangoli al massimo", "Maximum triangles", "Höchstens Dreiecke"],
        "at_adoucir" => ["Fondu au bord des zones lisses", "Fundido en el borde de las zonas lisas", "Sfumatura al bordo delle zone lisce", "Fade at the edge of smooth areas", "Übergang am Rand glatter Bereiche"],
        "at_angle" => ["Angle du dessus / dessous", "Ángulo de arriba / abajo", "Angolo sopra / sotto", "Top / bottom angle", "Winkel oben / unten"],
        "at_s_calques" => ["Calques", "Capas", "Livelli", "Layers", "Ebenen"],
        "at_ajouter_calque" => ["+ Ajouter un calque", "+ Añadir una capa", "+ Aggiungi un livello", "+ Add a layer", "+ Ebene hinzufügen"],
        "at_ajoute" => ["ajoute", "suma", "aggiunge", "adds", "addiert"],
        "at_remplace" => ["remplace", "reemplaza", "sostituisce", "replaces", "ersetzt"],
        "at_melange_tip" => ["Ajoute : s'additionne aux calques du dessous · Remplace : prend leur place", "Suma: se añade a las capas inferiores · Reemplaza: ocupa su lugar", "Aggiunge: si somma ai livelli sotto · Sostituisce: prende il loro posto", "Adds: stacks on the layers below · Replaces: takes their place", "Addiert: kommt zu den Ebenen darunter hinzu · Ersetzt: nimmt ihren Platz ein"],
        "p_triplanaire" => ["Triplanaire (raccords fondus)", "Triplanar (uniones fundidas)", "Triplanare (raccordi sfumati)", "Triplanar (blended seams)", "Triplanar (weiche Übergänge)"],
        "p_cube" => ["Cube (raccords nets)", "Cubo (uniones nítidas)", "Cubo (raccordi netti)", "Cube (sharp seams)", "Würfel (scharfe Übergänge)"],
        "p_plan" => ["À plat, comme un tampon", "Plano, como un sello", "Piatto, come un timbro", "Flat, like a stamp", "Flach, wie ein Stempel"],
        "p_cylindre" => ["Cylindre", "Cilindro", "Cilindro", "Cylinder", "Zylinder"],
        "p_sphere" => ["Sphère", "Esfera", "Sfera", "Sphere", "Kugel"],
        "m_briques" => ["Briques", "Ladrillos", "Mattoni", "Bricks", "Ziegel"],
        "m_dalles" => ["Dalles", "Losas", "Piastrelle", "Tiles", "Fliesen"],
        "m_hexagones" => ["Hexagones", "Hexágonos", "Esagoni", "Hexagons", "Sechsecke"],
        "m_losanges" => ["Moletage en losanges", "Moleteado en rombos", "Zigrinatura a rombi", "Diamond knurling", "Rautenrändel"],
        "m_stries" => ["Stries", "Estrías", "Scanalature", "Ridges", "Rillen"],
        "m_vagues" => ["Vagues", "Olas", "Onde", "Waves", "Wellen"],
        "m_ondulations" => ["Ondulations", "Ondulaciones", "Increspature", "Ripples", "Kreiswellen"],
        "m_pois" => ["Pois", "Lunares", "Pois", "Dots", "Punkte"],
        "m_ecailles" => ["Écailles", "Escamas", "Squame", "Scales", "Schuppen"],
        "m_chevrons" => ["Chevrons", "Espiga", "Spina di pesce", "Chevrons", "Fischgrät"],
        "m_tissage" => ["Vannerie", "Cestería", "Intreccio", "Basket weave", "Korbgeflecht"],
        "m_carbone" => ["Fibre de carbone", "Fibra de carbono", "Fibra di carbonio", "Carbon fibre", "Kohlefaser"],
        "m_pierres" => ["Pierres", "Piedras", "Pietre", "Stones", "Steine"],
        "m_craquelures" => ["Craquelures", "Grietas", "Crepe", "Cracks", "Risse"],
        "m_grain" => ["Grain", "Grano", "Grana", "Grain", "Körnung"],
        "m_bois" => ["Bois", "Madera", "Legno", "Wood", "Holz"],
        "m_cuir" => ["Cuir", "Cuero", "Pelle", "Leather", "Leder"],
        "m_martele" => ["Martelé", "Martillado", "Martellato", "Hammered", "Gehämmert"],
        "m_pyramides" => ["Pyramides", "Pirámides", "Piramidi", "Pyramids", "Pyramiden"],
        "m_grillage" => ["Grillage", "Malla", "Rete", "Mesh", "Gitter"],
        "m_damier" => ["Damier", "Damero", "Scacchiera", "Checkerboard", "Schachbrett"],
        "m_triangles" => ["Triangles", "Triángulos", "Triangoli", "Triangles", "Dreiecke"],
        "m_terrasses" => ["Terrasses", "Terrazas", "Terrazze", "Terraces", "Terrassen"],
        "m_tressage" => ["Tresse", "Trenza", "Treccia", "Braid", "Zopf"],
        "at_surface" => ["Surface", "Superficie", "Superficie", "Surface", "Fläche"],
        "at_surface_tip" => ["Clic sur une face : toute la surface reste lisse, petites marches comprises (Maj : retirer)", "Clic en una cara: toda la superficie queda lisa, pequeños escalones incluidos (Mayús: quitar)", "Clic su una faccia: tutta la superficie resta liscia, piccoli gradini compresi (Maiusc: togli)", "Click a face: the whole surface stays smooth, small steps included (Shift: remove)", "Klick auf eine Fläche: die ganze Fläche bleibt glatt, kleine Stufen inbegriffen (Umschalt: entfernen)"],
        "at_marche" => ["Marches franchies", "Escalones salvados", "Gradini superati", "Steps crossed", "Überbrückte Stufen"],
        "at_marche_tip" => ["Hauteur de marche jusqu'à laquelle la surface continue (escaliers, paliers) ; 0 = surface strictement continue", "Altura de escalón hasta la que la superficie continúa (escaleras); 0 = superficie estrictamente continua", "Altezza del gradino fino a cui la superficie continua (scale); 0 = superficie strettamente continua", "Step height up to which the surface carries on (stairs, terraces); 0 = strictly continuous surface", "Stufenhöhe, bis zu der die Fläche weitergeht (Treppen, Absätze); 0 = streng zusammenhängende Fläche"],
        "at_s_reglages_tex" => ["Réglages de la texture", "Ajustes de la textura", "Impostazioni della texture", "Texture settings", "Textureinstellungen"],
        "at_s_reglages_pinceau" => ["Réglages du pinceau", "Ajustes del pincel", "Impostazioni del pennello", "Brush settings", "Pinseleinstellungen"],
        "m_batons" => ["Bâtons rompus", "Espiga de madera", "Spina di pesce", "Herringbone planks", "Fischgrätparkett"],
        "m_toler" => ["Tôle larmée", "Chapa lagrimada", "Lamiera mandorlata", "Tread plate", "Tränenblech"],
        "m_golf" => ["Balle de golf", "Pelota de golf", "Pallina da golf", "Golf ball", "Golfball"],
        "m_plots" => ["Plots", "Tetones", "Bottoni", "Studs", "Noppen"],
        "m_croix" => ["Croix", "Cruces", "Croci", "Crosses", "Kreuze"],
        "m_etoiles" => ["Étoiles", "Estrellas", "Stelle", "Stars", "Sterne"],
        "m_capitonne" => ["Capitonné", "Capitoné", "Capitonné", "Quilted", "Gesteppt"],
        "m_grecque" => ["Frise grecque", "Greca", "Greca", "Greek key", "Mäander"],
        "m_zigzag" => ["Zigzag", "Zigzag", "Zigzag", "Zigzag", "Zickzack"],
        "m_cercles" => ["Cercles", "Círculos", "Cerchi", "Circles", "Kreise"],
        "m_spirale" => ["Spirale", "Espiral", "Spirale", "Spiral", "Spirale"],
        "m_truchet" => ["Arcs de Truchet", "Arcos de Truchet", "Archi di Truchet", "Truchet arcs", "Truchet-Bögen"],
        "m_labyrinthe" => ["Labyrinthe", "Laberinto", "Labirinto", "Maze", "Labyrinth"],
        "m_niveaux" => ["Courbes de niveau", "Curvas de nivel", "Curve di livello", "Contour lines", "Höhenlinien"],
        "m_ecorce" => ["Écorce", "Corteza", "Corteccia", "Bark", "Rinde"],
        "m_sable" => ["Sable", "Arena", "Sabbia", "Sand", "Sand"],
        "m_crepi" => ["Crépi", "Gotelé", "Intonaco", "Stucco", "Putz"],
        "m_beton" => ["Béton", "Hormigón", "Calcestruzzo", "Concrete", "Beton"],
        "m_eponge" => ["Éponge", "Esponja", "Spugna", "Sponge", "Schwamm"],
        "m_peau_orange" => ["Peau d'orange", "Piel de naranja", "Buccia d'arancia", "Orange peel", "Orangenhaut"],
        "m_alveoles" => ["Nid d'abeille", "Panal", "Nido d'ape", "Honeycomb", "Waben"],
        "m_oeufs" => ["Boîte à œufs", "Huevera", "Portauova", "Egg crate", "Eierkarton"],
        "m_gaufre" => ["Gaufre", "Gofre", "Waffle", "Waffle", "Waffel"],
        "m_pneu" => ["Pneu", "Neumático", "Pneumatico", "Tyre tread", "Reifenprofil"],
        "m_crampons" => ["Crampons", "Tacos", "Tasselli", "Lugs", "Stollen"],
        "m_picots" => ["Grip · picots", "Grip · púas", "Grip · punte", "Grip · spikes", "Grip · Spitzen"],
        "m_obliques" => ["Rainures obliques", "Ranuras oblicuas", "Scanalature oblique", "Slanted grooves", "Schrägrillen"],
        "m_toile" => ["Toile", "Lona", "Tela", "Canvas", "Leinwand"],
        "m_jean" => ["Jean", "Vaquero", "Jeans", "Denim", "Jeansstoff"],
        "m_tricot" => ["Tricot", "Punto", "Maglia", "Knit", "Strick"],
        "m_corde" => ["Corde", "Cuerda", "Corda", "Rope", "Seil"],
        "m_mailles" => ["Cotte de mailles", "Cota de malla", "Cotta di maglia", "Chainmail", "Kettenhemd"],
        "m_bambou" => ["Bambou", "Bambú", "Bambù", "Bamboo", "Bambus"],
        "m_paves" => ["Pavés", "Adoquines", "Sampietrini", "Cobbles", "Pflaster"],
        "m_tuiles" => ["Tuiles", "Tejas", "Tegole", "Roof tiles", "Dachziegel"],
        "m_artdeco" => ["Art déco", "Art déco", "Art déco", "Art Deco", "Art déco"],
        "m_argyle" => ["Losanges écossais", "Rombos escoceses", "Rombi scozzesi", "Argyle", "Argyle"],
        "m_ondes" => ["Ondes croisées", "Ondas cruzadas", "Onde incrociate", "Cross waves", "Kreuzwellen"],
        "m_bulles" => ["Bulles", "Burbujas", "Bolle", "Bubbles", "Blasen"],
        "m_hachures" => ["Hachures", "Rayado cruzado", "Tratteggio", "Cross-hatch", "Schraffur"],
        "m_quadrillage" => ["Grip · quadrillage", "Grip · cuadriculado", "Grip · zigrinatura", "Grip · checkering", "Grip · Fischhaut"],
        "m_pointillage" => ["Grip · pointillage", "Grip · punteado", "Grip · puntinatura", "Grip · stippling", "Grip · Punzierung"],
        "m_vulcanise" => ["Caoutchouc vulcanisé", "Caucho vulcanizado", "Gomma vulcanizzata", "Vulcanised rubber", "Vulkanisiertes Gummi"],
        "m_micropointes" => ["Grip · micro-pointes", "Grip · micropúas", "Grip · micropunte", "Grip · micro-spikes", "Grip · Mikrospitzen"],
        "m_pointes_diamant" => ["Grip · pointes de diamant", "Grip · puntas de diamante", "Grip · punte di diamante", "Grip · diamond points", "Grip · Diamantspitzen"],
        "m_rape" => ["Grip · râpe", "Grip · escofina", "Grip · raspa", "Grip · rasp", "Grip · Raspel"],
        "m_ventouses" => ["Ventouses", "Ventosas", "Ventose", "Suction cups", "Saugnäpfe"],
        "m_griptape" => ["Grip · ruban antidérapant", "Grip · cinta antideslizante", "Grip · nastro antiscivolo", "Grip · grip tape", "Grip · Griptape"],
        "m_anneaux" => ["Anneaux de manche", "Anillos de mango", "Anelli di manico", "Handle rings", "Griffringe"],
        "f_toutes" => ["Toutes", "Todas", "Tutte", "All", "Alle"],
        "f_geo" => ["Géométriques", "Geométricas", "Geometriche", "Geometric", "Geometrisch"],
        "f_prise" => ["Prise & pneus", "Agarre y neumáticos", "Presa e pneumatici", "Traction & tyres", "Halt & Reifen"],
        "f_grip" => ["Grip", "Grip", "Grip", "Grip", "Grip"],
        "m_moletage_pointu" => ["Grip · moletage pointu", "Grip · moleteado afilado", "Grip · zigrinatura appuntita", "Grip · sharp knurl", "Grip · spitze Rändelung"],
        "m_pyramides_tri" => ["Grip · pyramides triangulaires", "Grip · pirámides triangulares", "Grip · piramidi triangolari", "Grip · triangular pyramids", "Grip · Dreieckspyramiden"],
        "m_epines" => ["Grip · épines", "Grip · espinas", "Grip · spine", "Grip · thorns", "Grip · Dornen"],
        "at_grip_titre" => ["GRIP — pointes qui accrochent la peau et les gants", "GRIP — puntas que se agarran a la piel y los guantes", "GRIP — punte che fanno presa su pelle e guanti", "GRIP — points that bite into skin and gloves", "GRIP — Spitzen, die an Haut und Handschuhen greifen"],
        "at_autres" => ["Autres textures", "Otras texturas", "Altre texture", "Other textures", "Weitere Texturen"],
        "f_nature" => ["Nature", "Naturaleza", "Natura", "Nature", "Natur"],
        "f_textile" => ["Tissus", "Tejidos", "Tessuti", "Fabrics", "Stoffe"],
        "f_pavage" => ["Pavages", "Pavimentos", "Pavimentazioni", "Tilings", "Pflaster"],
        "at_aide_exclure" => ["Choisis un outil puis clique sur l'objet : les zones rouges restent lisses. Clic droit : annuler la dernière action.", "Elige una herramienta y haz clic en el objeto: las zonas rojas quedan lisas. Clic derecho: deshacer la última acción.", "Scegli uno strumento e clicca sull'oggetto: le zone rosse restano lisce. Clic destro: annulla l'ultima azione.", "Pick a tool and click the object: red areas stay smooth. Right-click: undo the last action.", "Werkzeug wählen und aufs Objekt klicken: rote Bereiche bleiben glatt. Rechtsklick: letzte Aktion rückgängig."],
        "at_negatif" => ["Négatif", "Negativo", "Negativo", "Negative", "Negativ"],
        "at_negatif_tip" => ["Inverse la texture sur l'objet : les creux deviennent des bosses et inversement", "Invierte la textura sobre el objeto: los huecos se vuelven relieves y viceversa", "Inverte la texture sull'oggetto: gli incavi diventano rilievi e viceversa", "Inverts the texture on the object: hollows become bumps and vice versa", "Kehrt die Textur auf dem Objekt um: Vertiefungen werden Erhebungen und umgekehrt"],
        "tx_titre" => ["Textures", "Texturas", "Texture", "Textures", "Texturen"],
        "tx_tip" => ["Habille l'objet d'une texture en relief : motifs calculés ou tes propres images, zones lisses au pinceau", "Viste el objeto con una textura en relieve: motivos calculados o tus propias imágenes, zonas lisas con el pincel", "Riveste l'oggetto con una texture in rilievo: motivi calcolati o le tue immagini, zone lisce col pennello", "Gives the object a relief texture: generated patterns or your own images, smooth areas with the brush", "Versieht das Objekt mit einer Relieftextur: berechnete Muster oder eigene Bilder, glatte Bereiche mit dem Pinsel"],
        "tx_toutes" => ["Toutes", "Todas", "Tutte", "All", "Alle"],
        "tx_cherche" => ["Rechercher…", "Buscar…", "Cerca…", "Search…", "Suchen…"],
        "tx_perso" => ["Image perso…", "Imagen propia…", "Immagine propria…", "Custom image…", "Eigenes Bild…"],
        "tx_perso_tip" => ["Utiliser ta propre image (PNG, JPG) comme carte de hauteur : blanc = haut, noir = bas", "Usar tu propia imagen (PNG, JPG) como mapa de altura: blanco = alto, negro = bajo", "Usa una tua immagine (PNG, JPG) come mappa di altezza: bianco = alto, nero = basso", "Use your own image (PNG, JPG) as a height map: white = high, black = low", "Eigenes Bild (PNG, JPG) als Höhenkarte: Weiß = hoch, Schwarz = tief"],
        "tx_proj" => ["Projection", "Proyección", "Proiezione", "Projection", "Projektion"],
        "tx_p_tri" => ["Triplanaire", "Triplanar", "Triplanare", "Triplanar", "Triplanar"],
        "tx_p_cube" => ["Cubique", "Cúbica", "Cubica", "Cubic (box)", "Kubisch"],
        "tx_p_cyl" => ["Cylindrique", "Cilíndrica", "Cilindrica", "Cylindrical", "Zylindrisch"],
        "tx_p_sph" => ["Sphérique", "Esférica", "Sferica", "Spherical", "Sphärisch"],
        "tx_p_xy" => ["Plan XY", "Plano XY", "Piano XY", "Planar XY", "Ebene XY"],
        "tx_p_xz" => ["Plan XZ", "Plano XZ", "Piano XZ", "Planar XZ", "Ebene XZ"],
        "tx_p_yz" => ["Plan YZ", "Plano YZ", "Piano YZ", "Planar YZ", "Ebene YZ"],
        "tx_proj_tip" => ["Triplanaire : formes complexes · Cubique : pièces à faces planes · Cylindrique : objets de révolution (axe Z)", "Triplanar: formas complejas · Cúbica: piezas de caras planas · Cilíndrica: objetos de revolución (eje Z)", "Triplanare: forme complesse · Cubica: pezzi a facce piane · Cilindrica: oggetti di rivoluzione (asse Z)", "Triplanar: complex shapes · Cubic: flat-faced parts · Cylindrical: turned objects (Z axis)", "Triplanar: komplexe Formen · Kubisch: Teile mit ebenen Flächen · Zylindrisch: Drehteile (Z-Achse)"],
        "tx_taille" => ["Taille du motif", "Tamaño del motivo", "Dimensione motivo", "Pattern size", "Mustergröße"],
        "tx_lier" => ["Lier U et V", "Vincular U y V", "Collega U e V", "Link U and V", "U und V koppeln"],
        "tx_decal" => ["Décalage", "Desplazamiento", "Spostamento", "Offset", "Versatz"],
        "tx_rot" => ["Rotation", "Rotación", "Rotazione", "Rotation", "Drehung"],
        "tx_haut" => ["Hauteur du relief", "Altura del relieve", "Altezza rilievo", "Texture depth", "Relieftiefe"],
        "tx_creuser" => ["Relief rentrant (creuse)", "Relieve hacia dentro", "Rilievo rientrante", "Inward (engrave)", "Nach innen (gravieren)"],
        "tx_sym" => ["Symétrique (gris 50 % neutre)", "Simétrico (gris 50 % neutro)", "Simmetrico (grigio 50 % neutro)", "Symmetric (50 % grey = neutral)", "Symmetrisch (50 % Grau neutral)"],
        "tx_sym_tip" => ["Le blanc ressort, le noir rentre : le volume de l'objet est conservé", "El blanco sale, el negro entra: se conserva el volumen", "Il bianco esce, il nero rientra: il volume è conservato", "White pushes out, black pushes in: volume is preserved", "Weiß nach außen, Schwarz nach innen: Volumen bleibt erhalten"],
        "tx_inv" => ["Inverser la texture", "Invertir la textura", "Inverti la texture", "Invert texture", "Textur invertieren"],
        "tx_lisse" => ["Lissage de la texture", "Suavizado de la textura", "Ammorbidimento texture", "Texture smoothing", "Textur-Glättung"],
        "tx_fondu" => ["Fondu des raccords", "Fundido de uniones", "Sfumatura giunzioni", "Seam blending", "Nahtüberblendung"],
        "tx_bande" => ["Largeur de raccord", "Ancho de unión", "Larghezza giunzione", "Seam band width", "Nahtbreite"],
        "tx_cap" => ["Angle des capots", "Ángulo de tapas", "Angolo dei tappi", "Cap angle", "Deckelwinkel"],
        "tx_masque" => ["Laisser lisse les faces", "Dejar lisas las caras", "Lasciare lisce le facce", "Keep faces smooth", "Flächen glatt lassen"],
        "tx_bas" => ["Dessous jusqu'à", "Inferior hasta", "Sotto fino a", "Bottom up to", "Unten bis"],
        "tx_dessus" => ["Dessus jusqu'à", "Superior hasta", "Sopra fino a", "Top up to", "Oben bis"],
        "tx_masque_tip" => ["Faces inclinées de moins de cet angle par rapport à l'horizontale laissées sans texture (0 = tout texturer). Le dessous reste ainsi bien à plat sur le plateau.", "Las caras con inclinación menor que este ángulo quedan sin textura (0 = todo)", "Le facce inclinate meno di questo angolo restano senza texture (0 = tutto)", "Faces tilted less than this angle from horizontal stay untextured (0 = texture everything). Keeps the bottom flat on the bed.", "Flächen mit weniger Neigung als dieser Winkel bleiben ohne Textur (0 = alles)"],
        "tx_surplomb" => ["Pas de nouveau porte-à-faux", "Sin nuevos voladizos", "Nessuno sbalzo nuovo", "No new overhangs", "Keine neuen Überhänge"],
        "tx_surplomb_tip" => ["Aucun point ne descend : le relief part sur le côté au lieu de créer un porte-à-faux", "Ningún punto baja: el relieve va hacia el lado", "Nessun punto scende: il rilievo va di lato", "No vertex moves down: texture is pushed sideways instead of creating overhangs", "Kein Punkt sinkt: Relief geht zur Seite statt Überhang"],
        "tx_plat" => ["Dessous parfaitement plat", "Base perfectamente plana", "Base perfettamente piana", "Perfectly flat bottom", "Perfekt flache Unterseite"],
        "tx_res" => ["Résolution", "Resolución", "Risoluzione", "Resolution", "Auflösung"],
        "tx_res_tip" => ["Longueur d'arête maxi après subdivision : plus petit = détails plus fins, calcul plus long", "Longitud máx. de arista tras subdividir: más pequeño = más detalle", "Lunghezza max spigolo dopo suddivisione: più piccolo = più dettaglio", "Max edge length after subdivision: smaller = finer detail, slower", "Max. Kantenlänge nach Unterteilung: kleiner = feiner, langsamer"],
        "tx_max" => ["Triangles maxi", "Triángulos máx.", "Triangoli max", "Max triangles", "Max. Dreiecke"],
        "tx_avance" => ["Réglages avancés", "Ajustes avanzados", "Impostazioni avanzate", "Advanced settings", "Erweiterte Einstellungen"],
        "tx_apercu" => ["Aperçu en direct", "Vista previa en vivo", "Anteprima dal vivo", "Live preview", "Live-Vorschau"],
        "tx_apercu_tip" => ["Calcule un aperçu allégé à chaque réglage ; l'objet n'est modifié qu'avec APPLIQUER", "Calcula una vista previa ligera; el objeto solo cambia con APLICAR", "Calcola un'anteprima leggera; l'oggetto cambia solo con APPLICA", "Computes a light preview on every change; the object only changes with APPLY", "Berechnet eine leichte Vorschau; das Objekt ändert sich erst mit ANWENDEN"],
        "tx_appliquer" => ["Appliquer la texture", "Aplicar textura", "Applica texture", "Apply texture", "Textur anwenden"],
        "tx_retirer" => ["Retirer l'aperçu", "Quitar vista previa", "Rimuovi anteprima", "Remove preview", "Vorschau entfernen"],
        "tx_calcul" => ["Aperçu en calcul…", "Calculando vista previa…", "Calcolo anteprima…", "Computing preview…", "Vorschau wird berechnet…"],
        "tx_vu" => ["Aperçu", "Vista previa", "Anteprima", "Preview", "Vorschau"],
        "tx_ok" => ["Texture appliquée", "Textura aplicada", "Texture applicata", "Texture applied", "Textur angewendet"],
        "tx_plafond" => ["plafond de subdivision atteint : augmente la résolution", "límite de subdivisión alcanzado: aumenta la resolución", "limite di suddivisione raggiunto: aumenta la risoluzione", "subdivision cap reached: increase the resolution", "Unterteilungsgrenze erreicht: Auflösung erhöhen"],
        "tx_c_geometric" => ["Géométriques", "Geométricas", "Geometriche", "Geometric", "Geometrisch"],
        "tx_c_patterns" => ["Motifs", "Motivos", "Motivi", "Patterns", "Muster"],
        "tx_c_organic" => ["Organiques", "Orgánicas", "Organiche", "Organic", "Organisch"],
        "tx_c_fabric" => ["Tissus", "Tejidos", "Tessuti", "Fabric", "Stoff"],
        "tx_c_natural" => ["Naturelles", "Naturales", "Naturali", "Natural", "Natürlich"],
        "tx_c_grip" => ["Grip", "Agarre", "Presa", "Grip", "Griff"],
        "tx_c_mold" => ["Grains de moule", "Granos de molde", "Grane stampo", "Mold grains", "Formnarben"],
        "tx_c_tires" => ["Pneus", "Neumáticos", "Pneumatici", "Tires", "Reifen"],
        "creux_btn"     => ["Creuser", "Vaciar", "Svuota", "Hollow", "Aushöhlen"],
        "creux_tip"     => ["L'objet devient une coque de l'épaisseur choisie : moins de matière et d'impression, même aspect extérieur",
                            "El objeto se convierte en una carcasa del grosor elegido: menos material y tiempo de impresión, mismo aspecto exterior",
                            "L'oggetto diventa un guscio dello spessore scelto: meno materiale e tempo di stampa, stesso aspetto esterno",
                            "The object becomes a shell of the chosen thickness: less material and print time, same outside look",
                            "Das Objekt wird zu einer Schale der gewählten Dicke: weniger Material und Druckzeit, gleiche Außenform"],
        "creux_ep"      => ["Épaisseur de paroi", "Grosor de pared", "Spessore parete", "Wall thickness", "Wandstärke"],
        "creux_ep_tip"  => ["Épaisseur de la coque qui reste (au moins 2 à 3 fois le Ø de la buse)", "Grosor de la carcasa restante (al menos 2 a 3 veces el Ø de la boquilla)", "Spessore del guscio che resta (almeno 2-3 volte il Ø dell'ugello)", "Thickness of the remaining shell (at least 2 to 3 times the nozzle Ø)", "Dicke der verbleibenden Schale (mindestens 2- bis 3-facher Düsen-Ø)"],
        "creux_trou"    => ["Trou d'évacuation", "Orificio de vaciado", "Foro di drenaggio", "Drain hole", "Ablaufloch"],
        "creux_trou_tip"=> ["Percé sous le point le plus bas de la cavité : la résine ou la poudre peut s'écouler", "Perforado bajo el punto más bajo de la cavidad: la resina o el polvo pueden salir", "Forato sotto il punto più basso della cavità: resina o polvere possono uscire", "Drilled under the lowest point of the cavity: resin or powder can drain out", "Unter dem tiefsten Punkt des Hohlraums gebohrt: Harz oder Pulver kann abfließen"],
        "creux_diam"    => ["Ø du trou", "Ø del orificio", "Ø del foro", "Hole Ø", "Loch-Ø"],
        "creux_ok"      => ["matière économisée", "material ahorrado", "materiale risparmiato", "material saved", "Material gespart"],
        "creux_plein"   => ["Paroi trop épaisse : il ne reste pas de cavité (réduis l'épaisseur)", "Pared demasiado gruesa: no queda cavidad (reduce el grosor)", "Parete troppo spessa: non resta alcuna cavità (riduci lo spessore)", "Wall too thick: no cavity left (reduce the thickness)", "Wand zu dick: kein Hohlraum übrig (Dicke verringern)"],
        "forme_btn"     => ["Forme", "Forma", "Forma", "Shape", "Form"],
        "forme_tip"     => ["Mise en forme : position au millimètre, rotation, miroir, échelle, alignement, répartition, duplication en ligne ou en cercle",
                            "Modelado: posición al milímetro, rotación, espejo, escala, alineación, distribución, duplicación en línea o en círculo",
                            "Modellazione: posizione al millimetro, rotazione, specchio, scala, allineamento, distribuzione, duplicazione in linea o in cerchio",
                            "Shaping: position to the millimetre, rotation, mirror, scale, alignment, distribution, linear or circular duplication",
                            "Formen: Position auf den Millimeter, Drehung, Spiegeln, Skalieren, Ausrichten, Verteilen, lineares oder kreisförmiges Duplizieren"],
        "forme_pos"     => ["POSITION (mm)", "POSICIÓN (mm)", "POSIZIONE (mm)", "POSITION (mm)", "POSITION (mm)"],
        "forme_pos_tip" => ["Coin bas de la sélection : saisis ou fais glisser une valeur pour la déplacer", "Esquina inferior de la selección: escribe o arrastra un valor para moverla", "Angolo inferiore della selezione: digita o trascina un valore per spostarla", "Lower corner of the selection: type or drag a value to move it", "Untere Ecke der Auswahl: Wert eingeben oder ziehen, um sie zu verschieben"],
        "forme_poser"   => ["Poser sur le plateau", "Apoyar en la bandeja", "Appoggia sul piatto", "Lay on the plate", "Auf die Platte legen"],
        "forme_poser_tip"=> ["Descend la sélection jusqu'au plateau (Z = 0) sans la tourner", "Baja la selección hasta la bandeja (Z = 0) sin girarla", "Abbassa la selezione fino al piatto (Z = 0) senza ruotarla", "Lowers the selection onto the plate (Z = 0) without rotating it", "Senkt die Auswahl ohne Drehung auf die Platte (Z = 0)"],
        "forme_rot"     => ["ROTATION", "ROTACIÓN", "ROTAZIONE", "ROTATION", "DREHUNG"],
        "forme_rot_tip" => ["Tourne la sélection autour de son centre ; elle reste posée à la même hauteur", "Gira la selección alrededor de su centro; queda apoyada a la misma altura", "Ruota la selezione intorno al suo centro; resta appoggiata alla stessa altezza", "Rotates the selection about its centre; it stays at the same height", "Dreht die Auswahl um ihre Mitte; sie bleibt auf gleicher Höhe"],
        "forme_appl"    => ["OK", "OK", "OK", "OK", "OK"],
        "forme_miroir"  => ["MIROIR", "ESPEJO", "SPECCHIO", "MIRROR", "SPIEGELN"],
        "forme_miroir_tip"=> ["Symétrie de la sélection selon cet axe (faces gardées vers l'extérieur)", "Simetría de la selección según este eje (caras hacia fuera)", "Simmetria della selezione lungo questo asse (facce verso l'esterno)", "Mirrors the selection along this axis (faces kept facing outwards)", "Spiegelt die Auswahl entlang dieser Achse (Flächen bleiben außen)"],
        "forme_ech"     => ["ÉCHELLE", "ESCALA", "SCALA", "SCALE", "SKALIEREN"],
        "forme_ech_tip" => ["Agrandit ou réduit la sélection autour de son centre au sol", "Agranda o reduce la selección alrededor de su centro en el suelo", "Ingrandisce o riduce la selezione attorno al suo centro a terra", "Enlarges or shrinks the selection about its centre on the floor", "Vergrößert oder verkleinert die Auswahl um ihre Bodenmitte"],
        "forme_ech_axes"=> ["Échelle par axe", "Escala por eje", "Scala per asse", "Scale per axis", "Pro Achse skalieren"],
        "forme_aligner" => ["ALIGNER (plusieurs objets)", "ALINEAR (varios objetos)", "ALLINEA (più oggetti)", "ALIGN (several objects)", "AUSRICHTEN (mehrere Objekte)"],
        "forme_aligner_tip"=> ["Aligne les objets sélectionnés sur le bord ou le centre de leur ensemble", "Alinea los objetos seleccionados en el borde o el centro del conjunto", "Allinea gli oggetti selezionati sul bordo o sul centro dell'insieme", "Aligns the selected objects on the edge or centre of their group", "Richtet die gewählten Objekte an Rand oder Mitte der Gruppe aus"],
        "forme_g"       => ["Gauche", "Izq.", "Sinistra", "Left", "Links"],
        "forme_cx"      => ["Centre", "Centro", "Centro", "Centre", "Mitte"],
        "forme_d"       => ["Droite", "Der.", "Destra", "Right", "Rechts"],
        "forme_av"      => ["Avant", "Delante", "Davanti", "Front", "Vorne"],
        "forme_cy"      => ["Centre", "Centro", "Centro", "Centre", "Mitte"],
        "forme_ar"      => ["Arrière", "Detrás", "Dietro", "Back", "Hinten"],
        "forme_bas"     => ["Bas", "Abajo", "Basso", "Bottom", "Unten"],
        "forme_mil"     => ["Milieu", "Medio", "Metà", "Middle", "Mitte"],
        "forme_haut"    => ["Haut", "Arriba", "Alto", "Top", "Oben"],
        "forme_repartir"=> ["Répartir", "Repartir", "Distribuisci", "Distribute", "Verteilen"],
        "forme_repartir_tip"=> ["Espaces égaux entre les objets (3 au moins) ; le premier et le dernier ne bougent pas", "Espacios iguales entre objetos (3 como mínimo); el primero y el último no se mueven", "Spazi uguali tra gli oggetti (almeno 3); il primo e l'ultimo non si muovono", "Equal gaps between objects (at least 3); the first and last stay put", "Gleiche Abstände zwischen Objekten (mind. 3); erstes und letztes bleiben"],
        "forme_dup"     => ["DUPLIQUER", "DUPLICAR", "DUPLICA", "DUPLICATE", "DUPLIZIEREN"],
        "forme_lin"     => ["En ligne", "En línea", "In linea", "Linear", "Linear"],
        "forme_pas"     => ["Pas", "Paso", "Passo", "Step", "Abstand"],
        "forme_lin_tip" => ["Copies alignées le long de l'axe choisi, espacées du pas (nombre total d'objets)", "Copias alineadas a lo largo del eje elegido, separadas por el paso (número total)", "Copie allineate lungo l'asse scelto, distanziate del passo (numero totale)", "Copies lined up along the chosen axis, one step apart (total number)", "Kopien entlang der gewählten Achse im Abstand (Gesamtzahl)"],
        "forme_circ"    => ["En cercle", "En círculo", "In cerchio", "Circular", "Kreisförmig"],
        "forme_rayon"   => ["Rayon", "Radio", "Raggio", "Radius", "Radius"],
        "forme_circ_tip"=> ["Copies réparties en cercle autour d'un axe vertical placé à ce rayon (nombre total d'objets)", "Copias repartidas en círculo alrededor de un eje vertical a este radio (número total)", "Copie distribuite in cerchio attorno a un asse verticale a questo raggio (numero totale)", "Copies spread in a circle around a vertical axis at this radius (total number)", "Kopien im Kreis um eine senkrechte Achse in diesem Radius (Gesamtzahl)"],
        "texte_btn"     => ["Texte & relief", "Texto y relieve", "Testo e rilievo", "Text & relief", "Text & Relief"],
        "texte_tip"     => ["Incruster un texte dans l'objet : en relief ou gravé, à plat ou en suivant sa forme", "Incrustar un texto en el objeto: en relieve o grabado, plano o siguiendo su forma", "Incidere un testo nell'oggetto: in rilievo o inciso, piatto o seguendo la forma", "Emboss text into the object: raised or engraved, flat or following its shape", "Text ins Objekt einprägen: erhaben oder graviert, flach oder der Form folgend"],
        "texte_saisie"  => ["Tape ton texte ici (Entrée = nouvelle ligne)", "Escribe tu texto aquí (Intro = nueva línea)", "Scrivi qui il testo (Invio = nuova riga)", "Type your text here (Enter = new line)", "Text hier eingeben (Eingabe = neue Zeile)"],
        "texte_j_g"     => ["Aligné à gauche", "Alineado a la izquierda", "Allineato a sinistra", "Left aligned", "Linksbündig"],
        "texte_j_c"     => ["Centré", "Centrado", "Centrato", "Centred", "Zentriert"],
        "texte_j_d"     => ["Aligné à droite", "Alineado a la derecha", "Allineato a destra", "Right aligned", "Rechtsbündig"],
        "texte_aide"    => ["Le texte se pose sur la face de l'objet visée au centre de la vue. Glisse-le pour le déplacer sur la surface (il change de face tout seul), clique ailleurs sur l'objet pour l'y poser, tourne-le par les roues jaunes au milieu des arêtes (aimant tous les 45°), agrandis-le par les coins blancs. Flèche bleue : sa longueur donne la profondeur, sa direction incline le texte (aimant 0° / 45°) ; dehors = relief, dedans = gravé. Puis APPLIQUER.",
                            "El texto se coloca en la cara del objeto apuntada en el centro de la vista. Arrástralo para moverlo por la superficie (cambia de cara solo), haz clic en otro punto del objeto para ponerlo allí, gíralo con las ruedas amarillas en el centro de las aristas (imán cada 45°), agrándalo por las esquinas blancas. Flecha azul: su longitud da la profundidad, su dirección inclina el texto (imán 0° / 45°); fuera = relieve, dentro = grabado. Y APLICAR.",
                            "Il testo si posa sulla faccia dell'oggetto mirata al centro della vista. Trascinalo per spostarlo sulla superficie (cambia faccia da solo), clicca altrove sull'oggetto per metterlo lì, ruotalo con le ruote gialle al centro degli spigoli (magnete ogni 45°), ingrandiscilo dagli angoli bianchi. Freccia blu: la lunghezza dà la profondità, la direzione inclina il testo (magnete 0° / 45°); fuori = rilievo, dentro = inciso. Poi APPLICA.",
                            "The text lands on the face of the object aimed at the centre of the view. Drag it to move it over the surface (it changes face by itself), click elsewhere on the object to put it there, turn it with the yellow wheels at the middle of the edges (snapping every 45°), resize it with the white corners. Blue arrow: its length sets the depth, its direction tilts the text (snaps 0° / 45°); out = raised, in = engraved. Then APPLY.",
                            "Der Text legt sich auf die Fläche des Objekts in der Bildmitte. Ziehen, um ihn über die Oberfläche zu bewegen (er wechselt die Fläche von selbst), woanders aufs Objekt klicken, um ihn dort abzulegen, mit den gelben Rädern in der Kantenmitte drehen (rastet alle 45° ein), an den weißen Ecken vergrößern. Blauer Pfeil: Länge = Tiefe, Richtung neigt den Text (rastet bei 0° / 45° ein); außen = erhaben, innen = graviert. Dann ANWENDEN."],
        "texte_recentrer" => ["Replacer au centre de la vue", "Recolocar en el centro de la vista", "Riposiziona al centro della vista", "Put back at view centre", "In Bildmitte zurücksetzen"],
        "texte_x"       => ["Longueur X", "Longitud X", "Lunghezza X", "Length X", "Länge X"],
        "texte_y"       => ["Largeur Y", "Anchura Y", "Larghezza Y", "Width Y", "Breite Y"],
        "texte_prof"    => ["Profondeur Z", "Profundidad Z", "Profondità Z", "Depth Z", "Tiefe Z"],
        "texte_angle"   => ["Angle", "Ángulo", "Angolo", "Angle", "Winkel"],
        "texte_relief"  => ["En relief (extérieur)", "En relieve (exterior)", "In rilievo (esterno)", "Raised (outside)", "Erhaben (außen)"],
        "texte_horiz"   => ["horizontal", "horizontal", "orizzontale", "horizontal", "waagerecht"],
        "texte_vert"    => ["vertical", "vertical", "verticale", "vertical", "senkrecht"],
        "texte_relief_c"=> ["relief", "relieve", "rilievo", "raised", "erhaben"],
        "texte_grave_c" => ["gravé", "grabado", "inciso", "engraved", "graviert"],
        "texte_grave"   => ["Gravé (intérieur)", "Grabado (interior)", "Inciso (interno)", "Engraved (inside)", "Graviert (innen)"],
        "texte_suivre"  => ["Suivre la forme", "Seguir la forma", "Segui la forma", "Follow the shape", "Form folgen"],
        "texte_suivre_tip"=> ["Le texte épouse la courbe de l'objet sur lequel il repose (en relief comme gravé)", "El texto se adapta a la curva del objeto (en relieve o grabado)", "Il testo segue la curva dell'oggetto (in rilievo o inciso)", "The text hugs the curve of the object it rests on (raised or engraved)", "Der Text folgt der Krümmung des Objekts (erhaben oder graviert)"],
        "texte_appl"    => ["Appliquer", "Aplicar", "Applica", "Apply", "Anwenden"],
        "texte_appl_tip"=> ["Incruste le texte dans l'objet sélectionné", "Incrusta el texto en el objeto seleccionado", "Incide il testo nell'oggetto selezionato", "Embosses the text into the selected object", "Prägt den Text ins gewählte Objekt"],
        "texte_police"  => ["Police", "Fuente", "Carattere", "Font", "Schrift"],
        "texte_taille"  => ["Taille", "Tamaño", "Dimensione", "Size", "Größe"],
        "texte_taille_tip"=> ["Hauteur de corps de la police", "Altura del cuerpo de la fuente", "Altezza del corpo del carattere", "Font body height", "Schriftgrad (Höhe)"],
        "texte_filtre"  => ["Rechercher une police…", "Buscar una fuente…", "Cerca un carattere…", "Search a font…", "Schrift suchen…"],
        "img_btn"       => ["Insère image", "Insertar imagen", "Inserisci immagine", "Insert image", "Bild einfügen"],
        "img_tip"       => ["Vectoriser une image (réduite en noir & blanc) et l'incruster dans l'objet comme un texte : en relief ou gravée, à plat ou en suivant sa forme", "Vectorizar una imagen (reducida a blanco y negro) e incrustarla en el objeto como un texto: en relieve o grabada, plana o siguiendo su forma", "Vettorizzare un'immagine (ridotta in bianco e nero) e inciderla nell'oggetto come un testo: in rilievo o incisa, piatta o seguendo la forma", "Vectorise an image (reduced to black & white) and emboss it into the object like text: raised or engraved, flat or following its shape", "Ein Bild vektorisieren (auf Schwarzweiß reduziert) und wie Text ins Objekt einprägen: erhaben oder graviert, flach oder der Form folgend"],
        "img_choisir"   => ["Choisir une image…", "Elegir una imagen…", "Scegli un'immagine…", "Choose an image…", "Bild wählen…"],
        "img_aucune"    => ["Aucune image : choisis un fichier PNG, JPEG ou BMP", "Ninguna imagen: elige un archivo PNG, JPEG o BMP", "Nessuna immagine: scegli un file PNG, JPEG o BMP", "No image: choose a PNG, JPEG or BMP file", "Kein Bild: PNG-, JPEG- oder BMP-Datei wählen"],
        "img_illisible" => ["Image illisible", "Imagen ilegible", "Immagine illeggibile", "Unreadable image", "Bild nicht lesbar"],
        "img_vide"      => ["Rien de plein dans l'image : change la réduction ou inverse", "Nada lleno en la imagen: cambia la reducción o invierte", "Niente di pieno nell'immagine: cambia la riduzione o inverti", "Nothing filled in the image: change the reduction or invert", "Nichts Gefülltes im Bild: Reduktion ändern oder umkehren"],
        "img_vecto"     => ["Vectorisation", "Vectorización", "Vettorizzazione", "Vectorisation", "Vektorisierung"],
        "img_reduc"     => ["Réduction en noir & blanc", "Reducción a blanco y negro", "Riduzione in bianco e nero", "Reduction to black & white", "Reduktion auf Schwarzweiß"],
        "img_m_seuil"   => ["Seuil manuel", "Umbral manual", "Soglia manuale", "Manual threshold", "Manuelle Schwelle"],
        "img_m_auto"    => ["Seuil automatique", "Umbral automático", "Soglia automatica", "Automatic threshold", "Automatische Schwelle"],
        "img_m_adapt"   => ["Adaptatif (photos)", "Adaptativo (fotos)", "Adattivo (foto)", "Adaptive (photos)", "Adaptiv (Fotos)"],
        "img_m_coul"    => ["Par couleurs", "Por colores", "Per colori", "By colours", "Nach Farben"],
        "img_seuil"     => ["Seuil", "Umbral", "Soglia", "Threshold", "Schwelle"],
        "img_seuil_tip" => ["Les pixels plus sombres que ce seuil deviennent pleins", "Los píxeles más oscuros que este umbral se vuelven llenos", "I pixel più scuri di questa soglia diventano pieni", "Pixels darker than this threshold become filled", "Pixel dunkler als diese Schwelle werden gefüllt"],
        "img_ecart"     => ["Écart", "Desviación", "Scarto", "Offset", "Abstand"],
        "img_ecart_tip" => ["Un pixel est plein s'il est plus sombre que ses voisins d'au moins cet écart (plus grand = moins de bruit)", "Un píxel está lleno si es más oscuro que sus vecinos al menos en esta desviación (mayor = menos ruido)", "Un pixel è pieno se è più scuro dei vicini almeno di questo scarto (più grande = meno rumore)", "A pixel is filled if it is darker than its neighbours by at least this offset (larger = less noise)", "Ein Pixel ist gefüllt, wenn es um mindestens diesen Abstand dunkler als seine Nachbarn ist (größer = weniger Rauschen)"],
        "img_ncoul"     => ["Couleurs", "Colores", "Colori", "Colours", "Farben"],
        "img_coul_tip"  => ["Clique une couleur pour la mettre en plein (noir) ou en vide (blanc)", "Haz clic en un color para ponerlo lleno (negro) o vacío (blanco)", "Clicca un colore per renderlo pieno (nero) o vuoto (bianco)", "Click a colour to make it filled (black) or empty (white)", "Farbe anklicken: gefüllt (schwarz) oder leer (weiß)"],
        "img_inverser"  => ["Négatif", "Negativo", "Negativo", "Negative", "Negativ"],
        "img_inverser_tip" => ["Inverse le noir et le blanc : le fond devient plein, le dessin devient creux", "Invierte negro y blanco: el fondo se vuelve lleno, el dibujo hueco", "Inverte nero e bianco: lo sfondo diventa pieno, il disegno vuoto", "Swaps black and white: the background becomes filled, the drawing hollow", "Tauscht Schwarz und Weiß: der Hintergrund wird gefüllt, die Zeichnung hohl"],
        "img_sans_cadre"=> ["Supprimer le cadre", "Quitar el marco", "Rimuovi la cornice", "Remove frame", "Rahmen entfernen"],
        "img_sans_cadre_tip" => ["Retire tout ce qui est plein et touche le bord de l'image (bordure, fond sombre, fond du négatif) : seul le dessin intérieur reste", "Quita todo lo lleno que toca el borde de la imagen (borde, fondo oscuro, fondo del negativo): solo queda el dibujo interior", "Rimuove tutto ciò che è pieno e tocca il bordo dell'immagine (cornice, sfondo scuro, sfondo del negativo): resta solo il disegno interno", "Removes everything filled that touches the image edge (border, dark background, negative background): only the inner drawing remains", "Entfernt alles Gefüllte, das den Bildrand berührt (Rahmen, dunkler Hintergrund, Negativ-Hintergrund): nur die innere Zeichnung bleibt"],
        "img_flou"      => ["Lissage", "Suavizado", "Levigatura", "Smoothing", "Glättung"],
        "img_flou_tip"  => ["Adoucit l'image avant la réduction (moins de petits détails et de bruit)", "Suaviza la imagen antes de la reducción (menos detalles pequeños y ruido)", "Ammorbidisce l'immagine prima della riduzione (meno dettagli e rumore)", "Softens the image before reduction (fewer small details and less noise)", "Glättet das Bild vor der Reduktion (weniger kleine Details und Rauschen)"],
        "img_res"       => ["Résolution", "Resolución", "Risoluzione", "Resolution", "Auflösung"],
        "img_res_tip"   => ["Plus grand côté de l'image analysée : plus élevé = plus de détails, plus lent", "Lado mayor de la imagen analizada: más alto = más detalle, más lento", "Lato maggiore dell'immagine analizzata: più alto = più dettagli, più lento", "Longest side of the analysed image: higher = more detail, slower", "Längste Seite des analysierten Bildes: höher = mehr Details, langsamer"],
        "img_nettoyage" => ["Nettoyage", "Limpieza", "Pulizia", "Clean-up", "Bereinigung"],
        "img_nettoyage_tip" => ["Supprime les taches et les trous plus petits que cette surface (en pixels)", "Elimina las manchas y agujeros más pequeños que esta superficie (en píxeles)", "Rimuove macchie e buchi più piccoli di questa superficie (in pixel)", "Removes specks and holes smaller than this area (in pixels)", "Entfernt Flecken und Löcher kleiner als diese Fläche (in Pixeln)"],
        "img_tol"       => ["Tolérance", "Tolerancia", "Tolleranza", "Tolerance", "Toleranz"],
        "img_tol_tip"   => ["Écart maximal entre le tracé et l'image (en pixels) : petit = fidèle, grand = lignes plus simples et plus lisses", "Desviación máxima entre el trazado y la imagen (en píxeles): pequeña = fiel, grande = líneas más simples y suaves", "Scarto massimo tra il tracciato e l'immagine (in pixel): piccolo = fedele, grande = linee più semplici e lisce", "Maximum gap between the outline and the image (in pixels): small = faithful, large = simpler, smoother lines", "Maximaler Abstand zwischen Kontur und Bild (in Pixeln): klein = genau, groß = einfachere, glattere Linien"],
        "img_largeur"   => ["Largeur", "Anchura", "Larghezza", "Width", "Breite"],
        "img_largeur_tip" => ["Largeur du dessin incrusté (la hauteur suit les proportions de l'image)", "Anchura del dibujo incrustado (la altura sigue las proporciones)", "Larghezza del disegno inciso (l'altezza segue le proporzioni)", "Width of the embossed drawing (height follows the image proportions)", "Breite der eingeprägten Zeichnung (Höhe folgt den Proportionen)"],
        "img_stats"     => ["contours", "contornos", "contorni", "outlines", "Konturen"],
        "img_points"    => ["points", "puntos", "punti", "points", "Punkte"],
        "texte_charge"  => ["Recherche des polices du système…", "Buscando las fuentes del sistema…", "Ricerca dei caratteri di sistema…", "Looking for system fonts…", "Systemschriften werden gesucht…"],
        "imp_tip" => ["3D-IMP : préparation à l'impression (vérifications, matière, envoi au trancheur)", "3D-IMP: preparación para imprimir (comprobaciones, material, envío al laminador)", "3D-IMP: preparazione alla stampa (verifiche, materiale, invio allo slicer)", "3D-IMP: print preparation (checks, material, send to slicer)", "3D-IMP: Druckvorbereitung (Prüfungen, Material, an den Slicer senden)"],
        "imp_titre" => ["Préparation à l'impression", "Preparación para imprimir", "Preparazione alla stampa", "Print preparation", "Druckvorbereitung"],
        "imp_sel" => ["Objets sélectionnés", "Objetos seleccionados", "Oggetti selezionati", "Selected objects", "Gewählte Objekte"],
        "imp_tout" => ["Toute la scène (aucune sélection), objets", "Toda la escena (sin selección), objetos", "Tutta la scena (nessuna selezione), oggetti", "Whole scene (no selection), objects", "Ganze Szene (keine Auswahl), Objekte"],
        "imp_verif" => ["Vérification avant impression", "Comprobación antes de imprimir", "Verifica prima della stampa", "Pre-print check", "Prüfung vor dem Druck"],
        "imp_calcul" => ["Analyse en cours…", "Análisis en curso…", "Analisi in corso…", "Analysing…", "Analyse läuft…"],
        "imp_dims" => ["Dimensions", "Dimensiones", "Dimensioni", "Size", "Abmessungen"],
        "imp_etanche" => ["Pièce étanche (aucun trou, aucune erreur)", "Pieza estanca (sin agujeros ni errores)", "Pezzo chiuso (nessun foro, nessun errore)", "Watertight part (no holes, no errors)", "Dichtes Teil (keine Löcher, keine Fehler)"],
        "imp_non_etanche" => ["Pièce non étanche", "Pieza no estanca", "Pezzo non chiuso", "Part not watertight", "Teil nicht dicht"],
        "imp_pose" => ["Posée sur le plateau", "Apoyada en la bandeja", "Appoggiata sul piatto", "Lying on the plate", "Liegt auf der Platte"],
        "imp_flotte" => ["Ne touche pas le plateau : flotte de", "No toca la bandeja: flota", "Non tocca il piatto: sospeso di", "Not touching the plate: floats by", "Berührt die Platte nicht: schwebt"],
        "imp_sous" => ["Passe sous le plateau de", "Pasa bajo la bandeja", "Scende sotto il piatto di", "Goes below the plate by", "Ragt unter die Platte um"],
        "imp_pas_surplomb" => ["Aucun surplomb : pas de supports nécessaires", "Sin voladizos: no hacen falta soportes", "Nessuno sbalzo: niente supporti", "No overhangs: no supports needed", "Keine Überhänge: keine Stützen nötig"],
        "imp_surplombs" => ["Surplombs à soutenir", "Voladizos que soportar", "Sbalzi da supportare", "Overhangs to support", "Zu stützende Überhänge"],
        "imp_voir_orange" => ["Les voir en orange", "Verlos en naranja", "Vederli in arancione", "Show them in orange", "Orange anzeigen"],
        "imp_angle" => ["Angle sans support", "Ángulo sin soporte", "Angolo senza supporto", "Angle without support", "Winkel ohne Stütze"],
        "imp_angle_tip" => ["Inclinaison maximale (depuis la verticale) imprimable sans support, souvent 45°", "Inclinación máxima (desde la vertical) imprimible sin soporte, a menudo 45°", "Inclinazione massima (dalla verticale) stampabile senza supporto, spesso 45°", "Maximum slope (from vertical) printable without support, often 45°", "Maximale Neigung (von der Senkrechten) ohne Stütze, meist 45°"],
        "imp_pas_fine" => ["Aucune paroi trop fine", "Ninguna pared demasiado fina", "Nessuna parete troppo sottile", "No wall too thin", "Keine zu dünne Wand"],
        "imp_fines" => ["Parois trop fines", "Paredes demasiado finas", "Pareti troppo sottili", "Walls too thin", "Zu dünne Wände"],
        "imp_voir_violet" => ["Les voir en violet", "Verlas en violeta", "Vederle in viola", "Show them in purple", "Violett anzeigen"],
        "imp_buse" => ["Ø buse", "Ø boquilla", "Ø ugello", "Nozzle Ø", "Düsen-Ø"],
        "imp_buse_tip" => ["Diamètre de la buse : une paroi doit faire au moins 2 fois ce diamètre", "Diámetro de la boquilla: una pared debe medir al menos 2 veces este diámetro", "Diametro dell'ugello: una parete deve essere almeno 2 volte questo diametro", "Nozzle diameter: a wall must be at least twice this size", "Düsendurchmesser: eine Wand muss mindestens doppelt so dick sein"],
        "imp_orient" => ["Orientation conseillée", "Orientación aconsejada", "Orientamento consigliato", "Suggested orientation", "Empfohlene Ausrichtung"],
        "imp_orient_tip" => ["Pose la pièce sur la face qui demande le moins de supports", "Apoya la pieza en la cara que pide menos soportes", "Appoggia il pezzo sulla faccia che richiede meno supporti", "Lays the part on the face needing the fewest supports", "Legt das Teil auf die Fläche mit den wenigsten Stützen"],
        "imp_orient_ok" => ["Orientation conseillée appliquée, surplombs", "Orientación aplicada, voladizos", "Orientamento applicato, sbalzi", "Suggested orientation applied, overhangs", "Ausrichtung angewendet, Überhänge"],
        "imp_orient_deja" => ["La pièce est déjà dans la meilleure orientation", "La pieza ya está en la mejor orientación", "Il pezzo è già nell'orientamento migliore", "The part is already in the best orientation", "Das Teil ist bereits optimal ausgerichtet"],
        "imp_matiere" => ["Matière", "Material", "Materiale", "Material", "Material"],
        "imp_plein" => ["Pièce pleine", "Pieza maciza", "Pezzo pieno", "Solid part", "Volles Teil"],
        "imp_plein_note" => ["Valeurs maximales (100 % de remplissage) : le trancheur donnera la consommation réelle", "Valores máximos (100 % de relleno): el laminador dará el consumo real", "Valori massimi (riempimento 100 %): lo slicer darà il consumo reale", "Maximum values (100 % infill): the slicer gives the actual usage", "Höchstwerte (100 % Füllung): der Slicer nennt den echten Verbrauch"],
        "imp_trancheur" => ["Trancheur", "Laminador", "Slicer", "Slicer", "Slicer"],
        "imp_autre" => ["Autre programme…", "Otro programa…", "Altro programma…", "Other program…", "Anderes Programm…"],
        "imp_recherche" => ["Recherche des trancheurs…", "Buscando laminadores…", "Ricerca degli slicer…", "Looking for slicers…", "Slicer werden gesucht…"],
        "imp_installe" => ["installé", "instalado", "installato", "installed", "installiert"],
        "imp_absent" => ["non trouvé", "no encontrado", "non trovato", "not found", "nicht gefunden"],
        "imp_choisir_prog" => ["Choisir…", "Elegir…", "Scegli…", "Choose…", "Wählen…"],
        "imp_non_trouve" => ["Ce trancheur n'est pas installé : choisis-en un autre, ou « Autre programme… »", "Este laminador no está instalado: elige otro u « Otro programa… »", "Questo slicer non è installato: scegline un altro o « Altro programma… »", "This slicer is not installed: pick another one or « Other program… »", "Dieser Slicer ist nicht installiert: anderen wählen oder « Anderes Programm… »"],
        "imp_envoyer" => ["Envoyer au trancheur", "Enviar al laminador", "Invia allo slicer", "Send to slicer", "An den Slicer senden"],
        "imp_envoyer_tip" => ["Ouvre les objets (3MF, séparés, à leur position) dans le trancheur choisi", "Abre los objetos (3MF, separados, en su posición) en el laminador elegido", "Apre gli oggetti (3MF, separati, nella loro posizione) nello slicer scelto", "Opens the objects (3MF, separate, in place) in the chosen slicer", "Öffnet die Objekte (3MF, getrennt, an ihrer Position) im gewählten Slicer"],
        "imp_envoye" => ["Envoyé au trancheur", "Enviado al laminador", "Inviato allo slicer", "Sent to the slicer", "An den Slicer gesendet"],
        "imp_echec" => ["Envoi impossible", "Envío imposible", "Invio impossibile", "Sending failed", "Senden fehlgeschlagen"],
        "texte_vide"    => ["Rien à incruster : tape un texte", "Nada que incrustar: escribe un texto", "Niente da incidere: scrivi un testo", "Nothing to emboss: type some text", "Nichts einzuprägen: Text eingeben"],
        "texte_hors"    => ["Le cadre du texte n'est pas au-dessus de l'objet", "El marco del texto no está sobre el objeto", "Il riquadro del testo non è sopra l'oggetto", "The text frame is not over the object", "Der Textrahmen liegt nicht über dem Objekt"],
        "bool_j"        => ["Booléen", "Booleana", "Booleana", "Boolean", "Boolesch"],
        "bool_ouvert"   => ["objet ouvert (trous) : lance d'abord RÉPARATION AUTO", "objeto abierto (agujeros): ejecuta primero REPARACIÓN AUTO", "oggetto aperto (fori): esegui prima RIPARAZIONE AUTO", "open object (holes): run AUTO REPAIR first", "offenes Objekt (Löcher): zuerst AUTO-REPARATUR ausführen"],
        "bool_invalide" => ["objet inutilisable pour un booléen", "objeto no válido para una booleana", "oggetto non valido per un booleano", "object not usable for a boolean", "Objekt für Boolesche Operation unbrauchbar"],
        "bool_echec"    => ["Le calcul booléen a échoué", "La operación booleana falló", "L'operazione booleana non è riuscita", "The boolean operation failed", "Boolesche Operation fehlgeschlagen"],
        "bool_vide"     => ["Résultat vide : les objets ne se touchent pas", "Resultado vacío: los objetos no se tocan", "Risultato vuoto: gli oggetti non si toccano", "Empty result: the objects do not touch", "Leeres Ergebnis: die Objekte berühren sich nicht"],
        "err_interne"  => ["Erreur interne", "Error interno", "Errore interno", "Internal error", "Interner Fehler"],
        "fmt_indispo"  => ["Format non disponible dans cette version", "Formato no disponible en esta versión", "Formato non disponibile in questa versione", "Format not available in this version", "Format in dieser Version nicht verfügbar"],
        "fmt_non_gere" => ["Format non géré", "Formato no compatible", "Formato non supportato", "Unsupported format", "Nicht unterstütztes Format"],
        "fmt_propri"   => ["format propriétaire : exporte-le en STEP, STL ou OBJ depuis le logiciel d'origine pour l'importer ici",
                           "formato propietario: expórtalo en STEP, STL u OBJ desde el programa original para importarlo aquí",
                           "formato proprietario: esportalo in STEP, STL o OBJ dal programma originale per importarlo qui",
                           "proprietary format: export it as STEP, STL or OBJ from the original software to import it here",
                           "proprietäres Format: im Originalprogramm als STEP, STL oder OBJ exportieren, um es hier zu importieren"],
        "blender_absent" => ["Blender n'est pas installé : impossible de lire un .blend", "Blender no está instalado: no se puede leer un .blend", "Blender non è installato: impossibile leggere un .blend", "Blender is not installed: cannot read a .blend file", "Blender ist nicht installiert: .blend kann nicht gelesen werden"],
        "blender_introuv" => ["Blender introuvable", "Blender no encontrado", "Blender non trovato", "Blender not found", "Blender nicht gefunden"],
        "blender_echec"  => ["L'export Blender a échoué", "La exportación de Blender falló", "Esportazione Blender non riuscita", "Blender export failed", "Blender-Export fehlgeschlagen"],
        "blender_stl"    => ["STL produit par Blender illisible", "STL de Blender ilegible", "STL di Blender illeggibile", "Unreadable STL from Blender", "STL aus Blender unlesbar"],
        "unzip_absent"   => ["Décompresseur (unzip) indisponible", "Descompresor (unzip) no disponible", "Decompressore (unzip) non disponibile", "Unzip tool not available", "Entpacker (unzip) nicht verfügbar"],
        "fcstd_illisible" => ["Archive FreeCAD illisible", "Archivo FreeCAD ilegible", "Archivio FreeCAD illeggibile", "Unreadable FreeCAD archive", "FreeCAD-Archiv unlesbar"],
        "dxf_illisible"  => ["DXF illisible", "DXF ilegible", "DXF illeggibile", "Unreadable DXF", "DXF unlesbar"],
        "dxf_sans_face"  => ["Aucune face 3D (3DFACE) dans ce DXF : seuls les DXF maillés en 3D sont pris en charge", "Ningún 3DFACE en este DXF: solo se admiten DXF de malla 3D", "Nessuna faccia 3D (3DFACE) in questo DXF: sono supportati solo DXF a maglia 3D", "No 3D faces (3DFACE) in this DXF: only 3D mesh DXF files are supported", "Keine 3D-Flächen (3DFACE) in dieser DXF: nur 3D-Netz-DXF werden unterstützt"],
        "ply_be"         => ["PLY gros-boutiste (big-endian) non pris en charge", "PLY big-endian no compatible", "PLY big-endian non supportato", "Big-endian PLY not supported", "Big-Endian-PLY nicht unterstützt"],
        "tmf_illisible"  => ["Archive 3MF illisible", "Archivo 3MF ilegible", "Archivio 3MF illeggibile", "Unreadable 3MF archive", "3MF-Archiv unlesbar"],
        "tmf_vide"       => ["Le 3MF ne contient aucun triangle", "El 3MF no contiene ningún triángulo", "Il 3MF non contiene triangoli", "The 3MF contains no triangles", "Die 3MF enthält keine Dreiecke"],
        "obj_vide"       => ["Objet vide", "Objeto vacío", "Oggetto vuoto", "Empty object", "Leeres Objekt"],
        "trop_aretes"    => ["Trop d'arêtes pour le fil de fer : réduis d'abord le maillage (Simplification)", "Demasiadas aristas para el alambre: simplifica primero la malla", "Troppi spigoli per il fil di ferro: semplifica prima la mesh", "Too many edges for the wireframe: simplify the mesh first", "Zu viele Kanten für das Drahtgitter: zuerst das Netz vereinfachen"],
        "aretes"         => ["arêtes", "aristas", "spigoli", "edges", "Kanten"],
        "assoc_ok"       => ["3D-Patch ouvre maintenant les fichiers 3D", "3D-Patch abre ahora los archivos 3D", "3D-Patch ora apre i file 3D", "3D-Patch now opens 3D files", "3D-Patch öffnet jetzt 3D-Dateien"],
        "assoc_ok_win"   => ["3D-Patch ouvre maintenant les fichiers 3D (si Windows le demande, choisis 3D-Patch puis « Toujours »)", "3D-Patch abre ahora los archivos 3D (si Windows lo pide, elige 3D-Patch y « Siempre »)", "3D-Patch ora apre i file 3D (se Windows lo chiede, scegli 3D-Patch e « Sempre »)", "3D-Patch now opens 3D files (if Windows asks, choose 3D-Patch and « Always »)", "3D-Patch öffnet jetzt 3D-Dateien (falls Windows fragt: 3D-Patch und „Immer“ wählen)"],
        "assoc_rest"     => ["Associations précédentes restaurées", "Asociaciones anteriores restauradas", "Associazioni precedenti ripristinate", "Previous associations restored", "Vorherige Zuordnungen wiederhergestellt"],
        "types"          => ["types", "tipos", "tipi", "types", "Typen"],
        "err_io"     => ["Erreur interne pendant l'ouverture / la sauvegarde", "Error interno al abrir / guardar", "Errore interno durante apertura / salvataggio", "Internal error while opening / saving", "Interner Fehler beim Öffnen / Speichern"],
        "err_calc"   => ["Le calcul a échoué (erreur interne)", "El cálculo ha fallado (error interno)", "Il calcolo è fallito (errore interno)", "Computation failed (internal error)", "Berechnung fehlgeschlagen (interner Fehler)"],
        "int_suppr"  => ["triangle(s) internes supprimés", "triángulo(s) internos eliminados", "triangolo/i interni rimossi", "internal triangle(s) removed", "innere(s) Dreieck(e) entfernt"],
        "passes"     => ["passe(s)", "pasada(s)", "passaggio/i", "pass(es)", "Durchgang/-gänge"],
        "faces_err"  => ["face(s) en erreur", "cara(s) con error", "faccia/e con errore", "face error(s)", "fehlerhafte Fläche(n)"],
        "relief_ok"  => ["Relief appliqué", "Relieve aplicado", "Rilievo applicato", "Relief applied", "Relief angewendet"],
        "modeleur"   => ["Modeleur 3D · cœur OpenCASCADE", "Modelador 3D · núcleo OpenCASCADE", "Modellatore 3D · nucleo OpenCASCADE", "3D modeler · OpenCASCADE core", "3D-Modellierer · OpenCASCADE-Kern"],
        "droits"     => ["Logiciel libre sous licence GNU GPL-3.0", "Software libre bajo licencia GNU GPL-3.0", "Software libero con licenza GNU GPL-3.0", "Free software under the GNU GPL-3.0 licence", "Freie Software unter der Lizenz GNU GPL-3.0"],
        _ => ["?", "?", "?", "?", "?"],
    };
    row[i]
}

const SNAP_NAMES: [&str; 14] = ["Origine","Axes","Fin","Milieu","Centre","Intersection","Quadrant","Point","Sur","Surface","Perpendiculaire","Tangente","Perp / Perp","Tan / Tan"];

fn code_de_vue(m: ViewMode) -> &'static str {
    match m { ViewMode::Quad => "quad", ViewMode::Single(ViewKind::ThreeD) => "3d", ViewMode::Single(ViewKind::Top) => "dessus",
              ViewMode::Single(ViewKind::Front) => "face", ViewMode::Single(ViewKind::Right) => "droite" }
}
fn vue_de_code(c: &str) -> Option<ViewMode> {
    [ViewMode::Quad, ViewMode::Single(ViewKind::ThreeD), ViewMode::Single(ViewKind::Top), ViewMode::Single(ViewKind::Front), ViewMode::Single(ViewKind::Right)]
        .into_iter().find(|m| code_de_vue(*m) == c)
}

fn fmt_measure(mm: f32, unit: Unit) -> String {
    let v = mm / unit.factor();
    let s = format!("{:.*}", unit.decimales(), v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

fn write_stl(path: &std::path::Path, mesh: &stl::Mesh) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(f, "solid objet")?;
    for c in mesh.verts.chunks_exact(18) {
        writeln!(f, " facet normal {} {} {}", c[3], c[4], c[5])?;
        writeln!(f, "  outer loop")?;
        for k in 0..3 { let o = k * 6; writeln!(f, "   vertex {} {} {}", c[o], c[o+1], c[o+2])?; }
        writeln!(f, "  endloop")?;
        writeln!(f, " endfacet")?;
    }
    writeln!(f, "endsolid objet")?;
    Ok(())
}

fn ray_tri(orig: Vec3, dir: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-7 { return None; }
    let inv = 1.0 / det;
    let s = orig - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 { return None; }
    let d = e2.dot(q) * inv;
    if d > 1e-4 { Some(d) } else { None }
}

/// Rayon monde (origine, direction) sous le curseur dans un rectangle de vue.
fn ray_from(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32)) -> (Vec3, Vec3) {
    let (rx, ry, rw, rh) = rect;
    let nx = ((cursor.0 as f32 - rx) / rw) * 2.0 - 1.0;
    let ny = 1.0 - ((cursor.1 as f32 - ry) / rh) * 2.0;
    let inv = mvp.inverse();
    let np = { let p = inv * Vec4::new(nx, ny, -1.0, 1.0); p.xyz() / p.w };
    let fp = { let p = inv * Vec4::new(nx, ny, 1.0, 1.0); p.xyz() / p.w };
    (np, (fp - np).normalize())
}

/// Objet sous le curseur (le plus proche) + point touché + normale de la facette touchée.
fn pick_scene(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32), scene: &[Obj]) -> Option<(usize, Vec3, Vec3)> {
    let (orig, dir) = ray_from(mvp, cursor, rect);
    let mut best: Option<(usize, f32, Vec3)> = None;
    for (i, o) in scene.iter().enumerate() {
        for t in &o.tris {
            if let Some(d) = ray_tri(orig, dir, t) {
                if best.map_or(true, |(_, b, _)| d < b) {
                    let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                    best = Some((i, d, n));
                }
            }
        }
    }
    best.map(|(i, d, n)| (i, orig + dir * d, n))
}

/// Point d'intersection du rayon avec le plan « coordonnée `axe` = 0 » (tracé de la ligne de coupe).
fn plane_point(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32), axe: usize) -> Option<Vec3> {
    let (orig, dir) = ray_from(mvp, cursor, rect);
    if dir[axe].abs() < 1e-6 { return None; }
    let d = -orig[axe] / dir[axe];
    if d.is_finite() { let mut p = orig + dir * d; p[axe] = 0.0; Some(p) } else { None }
}
/// Hauteur du cadre de coupe étendu : toute la scène le long de la direction de dépli, plus une marge.
fn coupe_hauteurs(c: &booleen::Cadre, scene: &[Obj]) -> (f32, f32) {
    let (lo, hi) = scene_bounds(scene);
    let (b, h) = c.etendue_l(lo, hi);
    let m = ((h - b) * 0.08).max(2.0);
    (b.min(0.0) - m, h + m)
}
/// Cadre tel qu'il est montré : le rectangle L × l tracé, ou (l = 0) le cadre étendu à toute la scène.
fn cadre_affiche(c: &booleen::Cadre, scene: &[Obj]) -> (booleen::Cadre, f32, f32) {
    if c.l > 0.0 { return (*c, 0.0, c.l); }
    let ce = cadre_etendu(c, scene);
    let (bas, haut) = coupe_hauteurs(&ce, scene);
    (ce, bas, haut)
}
/// Cadre étendu à toute la largeur des objets (le long de la ligne tracée) : la coupe sépare
/// franchement la pièce même si la ligne tracée est plus courte qu'elle.
fn cadre_etendu(c: &booleen::Cadre, scene: &[Obj]) -> booleen::Cadre {
    let Some((u, _, l)) = c.reperes() else { return *c };
    let (lo, hi) = scene_bounds(scene);
    let (mut s0, mut s1) = (0.0f32, l);
    for i in 0..8 {
        let q = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let s = (q - c.p0).dot(u);
        s0 = s0.min(s); s1 = s1.max(s);
    }
    let m = ((hi - lo).length() * 0.05).max(2.0);
    booleen::Cadre { p0: c.p0 + u * (s0 - m), p1: c.p0 + u * (s1 + m), ..*c }
}
/// Ligne de coupe : là où le cadre traverse les objets (segments en coordonnées monde),
/// limitée à l'emprise du cadre (longueur de la ligne tracée × hauteur affichée).
fn section_cadre(c: &booleen::Cadre, scene: &[Obj]) -> Vec<[Vec3; 2]> {
    let c = &cadre_etendu(c, scene);
    let (Some((u, n, l)), Some(w)) = (c.reperes(), c.dir_l()) else { return Vec::new() };
    let (bas, haut) = coupe_hauteurs(c, scene);
    let og = c.origine();
    let mut out = Vec::new();
    for o in scene {
        for t in &o.tris {
            let d = [(t[0] - c.p0).dot(n), (t[1] - c.p0).dot(n), (t[2] - c.p0).dot(n)];
            if (d[0] > 0.0 && d[1] > 0.0 && d[2] > 0.0) || (d[0] < 0.0 && d[1] < 0.0 && d[2] < 0.0) { continue; }
            let mut pts: Vec<Vec3> = Vec::with_capacity(2);
            for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                if (d[i] < 0.0) != (d[j] < 0.0) && (d[i] - d[j]).abs() > 1e-12 {
                    pts.push(t[i] + (t[j] - t[i]) * (d[i] / (d[i] - d[j])));
                }
            }
            if pts.len() != 2 { continue; }
            // découpage du segment au rectangle du cadre (coordonnées s le long de la ligne, h le long de l'axe)
            let (a, b) = (pts[0], pts[1]);
            let (sa, sb) = ((a - c.p0).dot(u), (b - c.p0).dot(u));
            let (ha, hb) = ((a - og).dot(w), (b - og).dot(w));
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            let mut ok = true;
            for (p0, dp, lo, hi) in [(sa, sb - sa, 0.0, l), (ha, hb - ha, bas, haut)] {
                if dp.abs() < 1e-12 { if p0 < lo || p0 > hi { ok = false; } continue; }
                let (mut ta, mut tb) = ((lo - p0) / dp, (hi - p0) / dp);
                if ta > tb { std::mem::swap(&mut ta, &mut tb); }
                t0 = t0.max(ta); t1 = t1.min(tb);
            }
            if ok && t0 < t1 { out.push([a + (b - a) * t0, a + (b - a) * t1]); }
        }
    }
    out
}
/// Lignes de coupe / d'intersection : trait rouge fin sur un liseré sombre à peine plus large
/// (lisible sur un objet clair sans masquer la précision du tracé).
fn lignes_rouges(p: &egui::Painter, segs: &[[Vec3; 2]], mvp: &Mat4, sr: (f32, f32, f32, f32), ppp: f32) {
    let pr: Vec<[egui::Pos2; 2]> = segs.iter().filter_map(|s| Some([project(mvp, s[0], sr, ppp)?, project(mvp, s[1], sr, ppp)?])).collect();
    for s in &pr { p.line_segment(*s, egui::Stroke::new(2.2, egui::Color32::from_rgba_unmultiplied(30, 0, 0, 110))); }
    for s in &pr { p.line_segment(*s, egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 35, 35))); }
}
/// Contours du texte ou de l'image en fil rouge 3D : en surface, à la profondeur `dh`, et quelques
/// arêtes entre les deux (au plus ~200, quel que soit le nombre de points). Une polyligne par contour.
fn contours_rouges(p: &egui::Painter, contours: &[Vec<[f64; 2]>], c: &CadreTexte, dh: f32, mvp: &Mat4, sr: (f32, f32, f32, f32), ppp: f32) {
    let total: usize = contours.iter().map(|ct| ct.len()).sum();
    let pas = (total / 200).max(3);
    let (fond, trait_) = (egui::Stroke::new(2.2, egui::Color32::from_rgba_unmultiplied(30, 0, 0, 110)), egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 35, 35)));
    let mut lignes: Vec<Vec<egui::Pos2>> = Vec::new();
    for ct in contours {
        for z in [0.0, dh] {
            // morceaux continus (un point hors champ coupe la ligne)
            let mut cur: Vec<egui::Pos2> = Vec::new();
            for q in ct.iter().chain(ct.first()) {
                match project(mvp, c.pt(q[0] as f32, q[1] as f32, z), sr, ppp) {
                    Some(e) => cur.push(e),
                    None => if cur.len() >= 2 { lignes.push(std::mem::take(&mut cur)); } else { cur.clear(); },
                }
            }
            if cur.len() >= 2 { lignes.push(cur); }
        }
        for q in ct.iter().step_by(pas) {
            if let (Some(a), Some(b)) = (project(mvp, c.pt(q[0] as f32, q[1] as f32, 0.0), sr, ppp), project(mvp, c.pt(q[0] as f32, q[1] as f32, dh), sr, ppp)) { lignes.push(vec![a, b]); }
        }
    }
    for l in &lignes { p.add(egui::Shape::line(l.clone(), fond)); }
    for l in lignes { p.add(egui::Shape::line(l, trait_)); }
}
/// Outil à rétablir si l'on annule juste après l'avoir appliqué (le cadre revient tel qu'il était).
#[derive(Clone, Copy)]
enum OutilAnn { Texte(CadreTexte), Coupe(booleen::Cadre) }

/// TEXTE : cadre du texte dans le repère de la vue où il a été tracé (`u` droite, `v` haut, `n`
/// vers l'observateur), posé à la hauteur `h` (le long de `n`), centre, tailles et angle.
#[derive(Clone, Copy)]
struct CadreTexte { u: Vec3, v: Vec3, n: Vec3, centre: Vec3, h: f32, lx: f32, ly: f32, prof: f32, angle: f32 }
impl CadreTexte {
    /// Point du cadre : (x, y) dans le cadre (centre = 0), à la hauteur `h` + `dh`.
    fn pt(&self, x: f32, y: f32, dh: f32) -> Vec3 {
        let (sn, cs) = self.angle.sin_cos();
        let c = self.centre - self.n * self.centre.dot(self.n);
        c + self.u * (x * cs - y * sn) + self.v * (x * sn + y * cs) + self.n * (self.h + dh)
    }
    fn coins(&self, dh: f32) -> [Vec3; 4] {
        let (a, b) = (self.lx * 0.5, self.ly * 0.5);
        [self.pt(-a, -b, dh), self.pt(a, -b, dh), self.pt(a, b, dh), self.pt(-a, b, dh)]
    }
    /// Roues de rotation : au milieu des arêtes L (bas, haut) et l (droite, gauche).
    fn roues(&self) -> [Vec3; 4] {
        let (a, b) = (self.lx * 0.5, self.ly * 0.5);
        [self.pt(0.0, -b, 0.0), self.pt(a, 0.0, 0.0), self.pt(0.0, b, 0.0), self.pt(-a, 0.0, 0.0)]
    }
}
/// Le cadre est-il vu par la tranche dans cette vue (une vue à plat qui ne regarde pas son plan) ?
fn de_tranche(c: &CadreTexte, k: ViewKind) -> bool {
    if k == ViewKind::ThreeD { return false; }
    let (r, u) = Ortho::right_up(k);
    r.cross(u).dot(c.n).abs() < 0.3
}
/// Surface de la scène sous le point `p`, regardée depuis +`n` : (objet, hauteur le long de `n`).
fn surface_sous(scene: &[Obj], p: Vec3, n: Vec3) -> Option<(usize, f32)> {
    let o = p + n * 1.0e5;
    let mut best: Option<(usize, f32)> = None;
    for (i, ob) in scene.iter().enumerate() {
        for t in &ob.tris { if let Some(d) = ray_tri(o, -n, t) { if best.map_or(true, |b| d < b.1) { best = Some((i, d)); } } }
    }
    best.map(|(i, d)| (i, (o - n * d).dot(n)))
}
/// Le curseur est-il sur une roue de rotation (milieu d'une arête L ou l) ? (rien quand le cadre est vu
/// par la tranche : tourner n'y aurait pas de sens)
/// `sf` = facteur d'échelle de l'écran : la zone de clic suit la taille dessinée (en points), même en HiDPI.
fn poignee_sous(c: &CadreTexte, k: ViewKind, mvp: &Mat4, rect: (f32, f32, f32, f32), cursor: (f64, f64), sf: f32) -> bool {
    if de_tranche(c, k) { return false; }
    let m = egui::pos2(cursor.0 as f32, cursor.1 as f32);
    c.roues().iter().any(|q| project(mvp, *q, rect, 1.0).map_or(false, |sp| (sp - m).length() < 13.0 * sf))
}
/// Le curseur est-il sur une poignée de taille (coins du cadre) ?
fn coin_sous(c: &CadreTexte, k: ViewKind, mvp: &Mat4, rect: (f32, f32, f32, f32), cursor: (f64, f64), sf: f32) -> bool {
    if de_tranche(c, k) { return false; }
    let m = egui::pos2(cursor.0 as f32, cursor.1 as f32);
    c.coins(0.0).iter().any(|q| project(mvp, *q, rect, 1.0).map_or(false, |sp| (sp - m).length() < 11.0 * sf))
}
/// Rayon monde d'une roue de ≈ 70 px centrée en `o` dans le plan (eu, ew) ; None si ce plan est vu par la tranche.
fn rayon_roue(mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32, o: Vec3, eu: Vec3, ew: Vec3) -> Option<f32> {
    let o2 = project(mvp, o, rect, ppp)?;
    let v: Vec<egui::Vec2> = [eu, ew].iter().filter_map(|&d| project(mvp, o + d, rect, ppp)).map(|q| q - o2).collect();
    if v.len() < 2 { return None; }
    let pxu = v[0].length().max(v[1].length());
    // vu par la tranche : les deux axes projetés presque alignés (aire du parallélogramme faible)
    (pxu > 1e-6 && (v[0].x * v[1].y - v[0].y * v[1].x).abs() >= 0.2 * pxu * pxu).then(|| 70.0 / pxu)
}
/// Longueur monde de la flèche de profondeur : la profondeur, mais au moins 40 px à l'écran (None si le
/// texte est regardé de face : flèche invisible).
fn long_fleche(c: &CadreTexte, mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32) -> Option<f32> {
    let o = c.pt(0.0, 0.0, 0.0);
    let l1 = (project(mvp, o + c.n, rect, ppp)? - project(mvp, o, rect, ppp)?).length();
    (l1 >= 0.15).then(|| c.prof.max(40.0 / l1))
}
/// Poignée de profondeur et d'orientation : flèche depuis le centre du texte, le long de ±n ; (base, pointe) à l'écran.
fn poignee_prof(c: &CadreTexte, mvp: &Mat4, rect: (f32, f32, f32, f32), ppp: f32, relief: bool) -> Option<(egui::Pos2, egui::Pos2)> {
    let o = c.pt(0.0, 0.0, 0.0);
    let l = long_fleche(c, mvp, rect, ppp)?;
    Some((project(mvp, o, rect, ppp)?, project(mvp, o + c.n * if relief { l } else { -l }, rect, ppp)?))
}
/// Cadre de texte posé sur la surface touchée (point `hit`, normale `nrm`), droit par rapport à la verticale.
fn cadre_sur(hit: Vec3, nrm: Vec3, base: Option<&CadreTexte>) -> CadreTexte {
    let n = nrm.normalize_or(Vec3::Z);
    let haut = if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::Y };
    let u = haut.cross(n).normalize_or(Vec3::X);
    let (lx, ly, prof, angle) = base.map_or((1.0, 1.0, 1.0, 0.0), |b| (b.lx, b.ly, b.prof, b.angle));
    CadreTexte { u, v: n.cross(u), n, centre: hit, h: hit.dot(n), lx, ly, prof, angle }
}
/// Le curseur est-il à l'intérieur du cadre de texte (tel qu'il est dessiné) ?
fn dans_cadre(c: &CadreTexte, mvp: &Mat4, rect: (f32, f32, f32, f32), cursor: (f64, f64)) -> bool {
    let q: Vec<egui::Pos2> = c.coins(0.0).iter().filter_map(|w| project(mvp, *w, rect, 1.0)).collect();
    if q.len() < 4 { return false; }
    let m = egui::pos2(cursor.0 as f32, cursor.1 as f32);
    let cr: Vec<f32> = (0..4).map(|i| { let (a, b) = (q[i], q[(i + 1) % 4]); (b - a).x * (m - a).y - (b - a).y * (m - a).x }).collect();
    // ou près d'un bord (cadre vu par la tranche : un simple trait)
    let pres = (0..4).any(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]); let ab = b - a;
        let t = ((m - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
        (a + ab * t - m).length() < 8.0
    });
    pres || cr.iter().all(|&x| x >= 0.0) || cr.iter().all(|&x| x <= 0.0)
}
/// Repère d'une vue : droite, haut, vers l'observateur (vue 3D : comme la vue de dessus).
fn repere_vue(k: ViewKind) -> (Vec3, Vec3, Vec3) {
    match k { ViewKind::ThreeD | ViewKind::Top => (Vec3::X, Vec3::Y, Vec3::Z), _ => { let (r, up) = Ortho::right_up(k); (r, up, r.cross(up)) } }
}
/// Point du curseur sur le plan n·p = h.
fn plan_h(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32), n: Vec3, h: f32) -> Option<Vec3> {
    let (o, d) = ray_from(mvp, cursor, rect);
    let dn = d.dot(n);
    if dn.abs() < 1e-6 { return None; }
    let t = (h - o.dot(n)) / dn;
    if t.is_finite() { Some(o + d * t) } else { None }
}
/// Icône de justification : 4 lignes alignées à gauche (0), centrées (1) ou à droite (2).
fn draw_justif(p: &egui::Painter, r: egui::Rect, j: u8, col: egui::Color32) {
    for (i, l) in [0.9f32, 0.6, 0.8, 0.5].iter().enumerate() {
        let y = r.top() + r.height() * (0.2 + 0.2 * i as f32);
        let w = r.width() * l;
        let x0 = match j { 0 => r.left(), 1 => r.center().x - w * 0.5, _ => r.right() - w };
        p.line_segment([egui::pos2(x0, y), egui::pos2(x0 + w, y)], egui::Stroke::new(2.2, col));
    }
}

/// SURFACE : indice de la face de pose sous le curseur (objet touché + plan de la face).
fn face_sous(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32), scene: &[Obj], s: &(usize, Vec<gravite::FacePose>)) -> Option<usize> {
    let (i, hit, nrm) = pick_scene(mvp, cursor, rect, scene)?;
    if i != s.0 { return None; }
    s.1.iter().position(|f| f.n.dot(nrm) > 0.999 && (f.n.dot(hit) - f.d).abs() < f.tol * 4.0)
}
/// Le curseur est-il sur le cadre de coupe tel qu'il est dessiné (surface, ou trait s'il est vu par la tranche) ?
fn coupe_touche(c: &booleen::Cadre, scene: &[Obj], mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32)) -> bool {
    let (c, bas, haut) = cadre_affiche(c, scene);
    let q: Vec<egui::Pos2> = c.coins(bas, haut).iter().filter_map(|w| project(mvp, *w, rect, 1.0)).collect();
    if q.len() < 4 { return false; }
    let m = egui::pos2(cursor.0 as f32, cursor.1 as f32);
    // à moins de 6 px d'un bord
    let pres = (0..4).any(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]); let ab = b - a;
        let t = ((m - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
        (a + ab * t - m).length() < 6.0
    });
    // ou à l'intérieur (même signe pour les 4 produits vectoriels)
    let cr: Vec<f32> = (0..4).map(|i| { let (a, b) = (q[i], q[(i + 1) % 4]); (b - a).x * (m - a).y - (b - a).y * (m - a).x }).collect();
    pres || cr.iter().all(|&x| x > 0.0) || cr.iter().all(|&x| x < 0.0)
}
/// Axe d'élévation du cadre de coupe selon la vue où l'on trace (perpendiculaire au plan de la vue).
fn axe_coupe(k: ViewKind) -> usize { match k { ViewKind::Front => 1, ViewKind::Right => 0, _ => 2 } }
/// Axes (horizontal, vertical) du plan de tracé perpendiculaire à l'axe `e` : l'angle de coupe se mesure de l'un vers l'autre.
fn axes_plan(e: usize) -> (usize, usize) { match e { 0 => (1, 2), 1 => (0, 2), _ => (0, 1) } }
/// Angle du trait de coupe dans son plan, en degrés (0 à 360).
fn angle_coupe(c: &booleen::Cadre) -> f32 {
    let (iu, iw) = axes_plan(c.e);
    let d = c.p1 - c.p0;
    (d[iw].atan2(d[iu]).to_degrees() + 360.0) % 360.0
}
/// Roue aimantée : à moins de 5° d'un multiple de 45°, l'angle (degrés) s'y cale.
fn aimante_angle(a: f32) -> f32 { let cran = (a / 45.0).round() * 45.0; if (a - cran).abs() <= 5.0 { cran } else { a } }
/// Roue aimantée : à moins de 5° d'un multiple de 45°, le trait s'y cale (sa longueur est gardée).
fn aimante_45(p0: Vec3, p: Vec3, e: usize) -> Vec3 {
    let (iu, iw) = axes_plan(e);
    let d = p - p0;
    let l = (d[iu] * d[iu] + d[iw] * d[iw]).sqrt();
    if l < 1e-6 { return p; }
    let a = d[iw].atan2(d[iu]).to_degrees();
    let cran = (a / 45.0).round() * 45.0;
    if (a - cran).abs() > 5.0 { return p; }
    let mut q = p0; let r = cran.to_radians();
    q[iu] += l * r.cos(); q[iw] += l * r.sin();
    q
}
/// Roue d'angle de la découpe, centrée en `o` dans le plan (eu, ew) : graduée tous les 15°, crans aimantés
/// tous les 45° (chiffrés ; celui atteint s'allume), arc jaune de 0° (eu) à l'angle courant `ang` (degrés).
/// Rien si ce plan est vu par la tranche.
fn roue_angle(p: &egui::Painter, mvp: &Mat4, sr: (f32, f32, f32, f32), ppp: f32, o: Vec3, eu: Vec3, ew: Vec3, ang: f32) {
    // rayon monde pour ≈ 70 px à l'écran
    let Some(rw) = rayon_roue(mvp, sr, ppp, o, eu, ew) else { return };
    let pt = |deg: f32, k: f32| { let a = deg.to_radians(); project(mvp, o + (eu * a.cos() + ew * a.sin()) * (rw * k), sr, ppp) };
    let blanc = egui::Color32::from_rgba_unmultiplied(235, 240, 255, 200);
    let cercle: Vec<egui::Pos2> = (0..=72).filter_map(|i| pt(i as f32 * 5.0, 1.0)).collect();
    if cercle.len() < 10 { return; }
    p.add(egui::Shape::line(cercle, egui::Stroke::new(1.5, blanc)));
    let ang = (ang + 360.0) % 360.0;
    let aimante = ((ang / 45.0).round() * 45.0 - ang).abs() < 0.01;
    // arc de 0° à l'angle courant
    let arc: Vec<egui::Pos2> = (0..=48).filter_map(|i| pt(ang * i as f32 / 48.0, 0.42)).collect();
    p.add(egui::Shape::line(arc, egui::Stroke::new(2.5, JAUNE)));
    for i in 0..24 {
        let deg = i as f32 * 15.0;
        let cran = i % 3 == 0;
        if let (Some(a), Some(b)) = (pt(deg, if cran { 0.80 } else { 0.90 }), pt(deg, 1.0)) {
            p.line_segment([a, b], egui::Stroke::new(if cran { 2.0 } else { 1.0 }, blanc));
        }
        if cran {
            let actif = aimante && ((deg - ang).abs() < 0.01 || (deg - ang).abs() > 359.99);
            if let Some(q) = pt(deg, 1.0) {
                p.circle_filled(q, if actif { 6.0 } else { 3.0 }, if actif { JAUNE } else { blanc });
                if actif { p.circle_stroke(q, 6.0, egui::Stroke::new(1.5, egui::Color32::BLACK)); }
            }
            if let Some(q) = pt(deg, 1.25) {
                p.text(q, egui::Align2::CENTER_CENTER, format!("{deg:.0}°"), egui::FontId::proportional(11.0), if actif { JAUNE } else { blanc });
            }
        }
    }
}
/// Partie d'un segment écran visible dans `r` (Liang-Barsky).
fn segment_dans(a: egui::Pos2, b: egui::Pos2, r: egui::Rect) -> Option<(egui::Pos2, egui::Pos2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x - r.left()), (d.x, r.right() - a.x), (-d.y, a.y - r.top()), (d.y, r.bottom() - a.y)] {
        if p.abs() < 1e-6 { if q < 0.0 { return None; } continue; }
        let t = q / p;
        if p < 0.0 { t0 = t0.max(t); } else { t1 = t1.min(t); }
    }
    (t0 < t1).then(|| (a + d * t0, a + d * t1))
}
/// Cote du cadre de coupe sur celui de deux bords opposés (`bords`) qui se voit le mieux dans la vue `vue` :
/// au milieu de sa partie visible, poussée vers l'extérieur ; rien si moins de 40 px sont visibles.
fn cote_bord(p: &egui::Painter, pr: &[egui::Pos2], bords: [(usize, usize); 2], vue: egui::Rect, t: String) {
    let centre = egui::pos2(pr.iter().map(|q| q.x).sum::<f32>() / 4.0, pr.iter().map(|q| q.y).sum::<f32>() / 4.0);
    let vue = vue.shrink(20.0);
    let Some((a, b)) = bords.iter().filter_map(|&(i, j)| segment_dans(pr[i], pr[j], vue))
        // à longueur visible égale : le bord de droite / du bas (loin des règles, à gauche et en haut)
        .max_by(|x, y| { let k = |s: &(egui::Pos2, egui::Pos2)| ((s.1 - s.0).length().round(), s.0.x + s.1.x + s.0.y + s.1.y);
                         let (kx, ky) = (k(x), k(y)); kx.0.total_cmp(&ky.0).then(kx.1.total_cmp(&ky.1)) }) else { return };
    if (b - a).length() < 40.0 { return; }
    let m = egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    let dir = if (m - centre).length() < 1.0 { egui::vec2(-(b - a).y, (b - a).x).normalized() } else { (m - centre).normalized() };
    let gal = p.layout_no_wrap(t, egui::FontId::proportional(13.0), egui::Color32::WHITE);
    // écart selon l'orientation du bord (largeur de l'étiquette sur un bord vertical), sans couvrir les poignées
    let ecart = dir.x.abs() * gal.size().x * 0.5 + dir.y.abs() * gal.size().y * 0.5 + 14.0;
    let r = egui::Rect::from_center_size(m + dir * ecart, gal.size());
    p.rect_filled(r.expand(3.0), 3.0, egui::Color32::from_rgba_unmultiplied(20, 20, 20, 210));
    p.galley(r.min, gal, egui::Color32::WHITE);
}

/// Point d'intersection du rayon avec le plan Z=0 (déplacement « sur la grille »).
fn ground_point(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32)) -> Option<Vec3> {
    let (orig, dir) = ray_from(mvp, cursor, rect);
    if dir.z.abs() < 1e-6 { return None; }
    let d = -orig.z / dir.z;
    if d.is_finite() { Some(orig + dir * d) } else { None }
}

/// Position d'un chiffre de règle : décalé hors de la règle, à GAUCHE si l'axe est plutôt
/// vertical à l'écran, EN DESSOUS s'il est plutôt horizontal (jamais posé sur l'objet).
/// `tw`, `th` = taille du texte (mêmes unités que le résultat), `gap` = écart à la règle.
fn ruler_label_pos(mvp: &Mat4, wp: Vec3, dir: Vec3, rect: (f32, f32, f32, f32), ppp: f32, tw: f32, th: f32, gap: f32) -> Option<egui::Pos2> {
    let p = project(mvp, wp, rect, ppp)?;
    let q = project(mvp, wp + dir * 0.01 * (1.0 + wp.length()), rect, ppp)?;
    let d = q - p;
    if d.length() < 0.05 { return None; } // axe vu de bout : pas de règle lisible, rien à afficher
    if d.y.abs() > d.x.abs() {
        Some(p - egui::vec2(tw * 0.5 + gap, 0.0))
    } else {
        Some(p + egui::vec2(0.0, th * 0.5 + gap))
    }
}

fn project(mvp: &Mat4, p: Vec3, rect: (f32, f32, f32, f32), ppp: f32) -> Option<egui::Pos2> {
    let clip = *mvp * p.extend(1.0);
    if clip.w <= 1e-4 { return None; }
    let ndc = clip.xyz() / clip.w;
    if ndc.z < -1.0 || ndc.z > 1.0 { return None; }
    let (rx, ry, rw, rh) = rect;
    let sx = rx + (ndc.x * 0.5 + 0.5) * rw;
    let sy = ry + (1.0 - (ndc.y * 0.5 + 0.5)) * rh;
    Some(egui::pos2(sx / ppp, sy / ppp))
}

/// Vrai si le segment œil→point est coupé par un triangle de l'objet (point masqué).
/// Vrai si un texte centré en `c` (pixels, même repère que `rect`) de largeur `tw` recouvre
/// un objet à l'écran (rayons au centre et aux deux bouts ; boîte englobante testée d'abord).
fn over_object(mvp: &Mat4, c: egui::Pos2, tw: f32, rect: (f32, f32, f32, f32), scene: &[Obj]) -> bool {
    for dx in [0.0, -0.45 * tw, 0.45 * tw] {
        let (o, d) = ray_from(mvp, ((c.x + dx) as f64, c.y as f64), rect);
        for ob in scene {
            if ob.nverts == 0 { continue; }
            // rayon / boîte englobante (méthode des dalles)
            let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
            for k in 0..3 {
                if d[k].abs() < 1e-12 { if o[k] < ob.min[k] || o[k] > ob.max[k] { t0 = 1.0; t1 = 0.0; } continue; }
                let (a, b) = ((ob.min[k] - o[k]) / d[k], (ob.max[k] - o[k]) / d[k]);
                t0 = t0.max(a.min(b)); t1 = t1.min(a.max(b));
            }
            if t0 > t1 || t1 < 0.0 { continue; }
            if ob.tris.iter().any(|t| ray_tri(o, d, t).is_some()) { return true; }
        }
    }
    false
}

fn occluded(eye: Vec3, p: Vec3, tris: &[[Vec3; 3]]) -> bool {
    let d = p - eye;
    let len = d.length();
    if len < 1e-4 { return false; }
    let dir = d / len;
    for t in tris { if let Some(hit) = ray_tri(eye, dir, t) { if hit < len - 0.5 { return true; } } }
    false
}

fn kind_tr(l: Lang, k: ViewKind) -> &'static str {
    match k { ViewKind::ThreeD => "3D", ViewKind::Top => tr(l, "top"), ViewKind::Front => tr(l, "front"), ViewKind::Right => tr(l, "right") }
}

/// Nom affiché d'un motif de relief (noms universels pour la galerie).
fn pattern_name(p: relief::Pattern) -> &'static str {
    use relief::Pattern::*;
    match p {
        Knurling => "Knurling", Ribs => "Ribs", Reeds => "Reeds", Ripples => "Ripples", Waves => "Waves",
        Bricks => "Bricks", Herringbone => "Herringbone", Grid => "Grid", Dots => "Dots", Cobblestone => "Cobblestone",
        Hammered => "Hammered", Stipple => "Grain", Concrete => "Concrete", Leather => "Leather", Brushed => "Brushed",
        Bark => "Bark", CarbonTwill => "Carbon", DiamondPlate => "Diamond plate", Chainmail => "Chainmail",
        Scales => "Scales", Pyramids => "Pyramids", Hexes => "Hexagons",
    }
}

/// Thème clair actif (lu par les étiquettes dessinées sur la vue).
static THEME_CLAIR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn clair() -> bool { THEME_CLAIR.load(std::sync::atomic::Ordering::Relaxed) }
/// Thème clair CONTRASTÉ : textes quasi noirs (y compris les textes secondaires), boutons gris
/// soutenus bordés, sélection bleu franc à texte blanc, liens bleu foncé.
fn visuels_clairs() -> egui::Visuals {
    use egui::{Color32 as C, Stroke};
    let mut v = egui::Visuals::light();
    v.panel_fill = C::from_gray(240);
    v.window_fill = C::from_gray(250);
    v.extreme_bg_color = C::WHITE;
    v.faint_bg_color = C::from_gray(232);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, C::from_gray(18));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, C::from_gray(170));
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.fg_stroke = Stroke::new(1.5, C::from_gray(10));
    }
    v.widgets.inactive.bg_fill = C::from_gray(212); v.widgets.inactive.weak_bg_fill = C::from_gray(212);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, C::from_gray(150));
    v.widgets.hovered.bg_fill = C::from_gray(196); v.widgets.hovered.weak_bg_fill = C::from_gray(196);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, C::from_rgb(30, 100, 190));
    v.widgets.active.bg_fill = C::from_gray(180); v.widgets.active.weak_bg_fill = C::from_gray(180);
    v.selection.bg_fill = C::from_rgb(30, 100, 190);
    v.selection.stroke = Stroke::new(1.0, C::WHITE);
    v.hyperlink_color = C::from_rgb(0, 70, 160);
    v.window_stroke = Stroke::new(1.2, C::from_gray(150));
    v
}

/// Texte cerclé de noir, ancrage au choix.
fn label_at(painter: &egui::Painter, pos: egui::Pos2, align: egui::Align2, text: &str, color: egui::Color32, size: f32) {
    let font = egui::FontId::proportional(size);
    let o = (size / 12.0).max(1.0);
    // thème clair : texte sombre cerclé de blanc (les textes blancs/gris clairs deviennent noirs)
    let (halo, color) = if clair() { (egui::Color32::from_white_alpha(235), if color.r() > 200 && color.g() > 200 && color.b() > 200 { egui::Color32::from_gray(15) } else { assombrir(color) }) }
                        else { (egui::Color32::from_black_alpha(230), color) };
    for off in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
        painter.text(pos + egui::vec2(off.0, off.1), align, text, font.clone(), halo);
    }
    painter.text(pos, align, text, font, color);
}
/// Couleur vive assombrie pour rester lisible sur fond clair.
fn assombrir(c: egui::Color32) -> egui::Color32 { egui::Color32::from_rgb((c.r() as f32 * 0.55) as u8, (c.g() as f32 * 0.55) as u8, (c.b() as f32 * 0.55) as u8) }
fn label(painter: &egui::Painter, pos: egui::Pos2, text: &str, color: egui::Color32, size: f32) {
    label_at(painter, pos, egui::Align2::CENTER_CENTER, text, color, size)
}

/// Logo 3D-Patch en 3D (exporté de Blender) : [x y z nx ny nz r g b] × 3 × triangles, f32 LE.
/// Renvoie (sommets [pos nrm], couleurs [r g b]).
fn logo3d() -> (Vec<f32>, Vec<f32>) {
    let b = include_bytes!("../assets/logo3d.bin");
    let f: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let mut v = Vec::with_capacity(f.len() / 9 * 6);
    let mut c = Vec::with_capacity(f.len() / 9 * 3);
    for s in f.chunks_exact(9) { v.extend_from_slice(&s[..6]); c.extend_from_slice(&s[6..]); }
    (v, c)
}

/// Formats proposés par le menu Sauver : (extension, libellé du bouton).
const FORMATS_SAUVE: &[(&str, &str)] = &[
    ("stl", "STL"), ("3mf", "3MF"), ("obj", "OBJ"), ("ply", "PLY"),
    ("3dm", "3DM MoI (v5)"), ("3dm8", "3DM Rhino 8"), ("skp", "SKP SketchUp"), ("3ds", "3DS"),
    ("fbx", "FBX"), ("dae", "DAE"), ("glb", "GLB"), ("gltf", "glTF"), ("x3d", "X3D"), ("solidcraft", "SolidCraft"),
];
/// Extension du fichier pour une entrée de FORMATS_SAUVE (« 3dm8 » = 3DM version Rhino 8).
fn ext_fichier(cle: &str) -> &str { if cle == "3dm8" { "3dm" } else { cle } }

/// Extensions ouvertes par l'appli (bouton Fichier, double-clic, ligne de commande).
const EXT_OUVERTURE: &[&str] = &["stl","obj","ply","3mf","3dm","skp","skb","3ds","dae","fbx","gltf","glb","x3d","step","stp","iges","igs","blend","fcstd","fcstd1","dxf","f3d","f3z","dwg","solidcraft","solvecraft"];

/// Charge un modèle selon son extension : (maillage, couleurs par sommet éventuelles).
fn load_model(path: &str) -> Result<(stl::Mesh, Option<Vec<f32>>), String> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mesh_of = |v: Vec<f32>| {
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for c in v.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
        stl::Mesh { verts: v, min: lo, max: hi }
    };
    match ext.as_str() {
        "step" | "stp" => cao_en_cache(path, || occt::import_cad(path, false, occt::DEFLEXION).map(|m| (m.verts, None)).map_err(|rc| format!("{} (STEP, code {rc})", trc("cao_illisible")))).map(|(v, c)| (mesh_of(v), c)),
        "iges" | "igs" => cao_en_cache(path, || occt::import_cad(path, true, occt::DEFLEXION).map(|m| (m.verts, None)).map_err(|rc| format!("{} (IGES, code {rc})", trc("cao_illisible")))).map(|(v, c)| (mesh_of(v), c)),
        "stl" | "obj" | "ply" => stl::load_any(path).map(|m| (m, None)).map_err(|e| format!("{e}")),
        "3mf" => threemf::read(path).map(|(v, c)| (mesh_of(v), c)),
        // conception SolidCraft : ses objets réunis (l'import par le bouton Fichier les garde séparés)
        "solidcraft" | "solvecraft" => solidcraft::lire(path).map(|objs| {
            let colore = objs.iter().any(|o| o.1.is_some());
            let (mut v, mut c) = (Vec::new(), Vec::new());
            for (ov, oc) in &objs {
                v.extend_from_slice(ov);
                if colore { match oc { Some(x) => c.extend_from_slice(x), None => for _ in 0..ov.len() / 6 { c.extend_from_slice(&[0.93, 0.80, 0.33]); } } }
            }
            (mesh_of(v), colore.then_some(c))
        }),
        "3dm" => cao_en_cache(path, || rhino::import(path)).map(|(v, c)| (mesh_of(v), c)),
        "skp" | "skb" => sketchup::import(path).map(|(v, c)| (mesh_of(v), c)),
        e if assimp::EXT_IMPORT.contains(&e) => assimp::import(path).map(|(v, c)| (mesh_of(v), c)),
        // formats d'applis tierces (Blender, FreeCAD, Fusion 360, AutoCAD)
        other => convert::import_external(path, other, occt::DEFLEXION).map(|m| (m, None)),
    }
}
/// Modèles CAO (3DM, STEP, IGES) : leur conversion en maillage peut prendre de longues secondes (couture
/// des faces NURBS par OpenCASCADE). Le maillage obtenu est gardé sous l'empreinte du fichier : rouvrir le
/// même fichier est immédiat. Cache ~/.cache/3d-patch/cao, 30 modèles au plus.
fn cao_en_cache(path: &str, convertir: impl FnOnce() -> Result<(Vec<f32>, Option<Vec<f32>>), String>) -> Result<(Vec<f32>, Option<Vec<f32>>), String> {
    use sha2::{Digest, Sha256};
    const MAGIC: &[u8; 8] = b"3DPCAO01";
    let Ok(octets) = std::fs::read(path) else { return convertir() };
    let cle: String = Sha256::digest(&octets).iter().map(|b| format!("{b:02x}")).collect();
    let dossier = if cfg!(windows) { std::env::temp_dir().join("3d-patch") } else {
        std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join(".cache").join("3d-patch")
    }.join("cao");
    // la précision de maillage fait partie de la clé : la changer refait la conversion
    let fichier = dossier.join(format!("{cle}-{}.bin", occt::DEFLEXION));
    let f32s = |b: &[u8]| -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect() };
    if let Ok(b) = std::fs::read(&fichier) {
        if b.len() >= 24 && &b[..8] == MAGIC {
            let (nv, nc) = (u64::from_le_bytes(b[8..16].try_into().unwrap()) as usize, u64::from_le_bytes(b[16..24].try_into().unwrap()) as usize);
            if b.len() == 24 + (nv + nc) * 4 {
                let v = f32s(&b[24..24 + nv * 4]);
                let c = (nc > 0).then(|| f32s(&b[24 + nv * 4..]));
                return Ok((v, c));
            }
        }
    }
    let (v, c) = convertir()?;
    let _ = std::fs::create_dir_all(&dossier);
    let mut b = Vec::with_capacity(24 + (v.len() + c.as_ref().map_or(0, |c| c.len())) * 4);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&(v.len() as u64).to_le_bytes());
    b.extend_from_slice(&(c.as_ref().map_or(0, |c| c.len()) as u64).to_le_bytes());
    for x in &v { b.extend_from_slice(&x.to_le_bytes()); }
    if let Some(c) = &c { for x in c { b.extend_from_slice(&x.to_le_bytes()); } }
    let tmp = fichier.with_extension("part");
    if std::fs::write(&tmp, &b).is_ok() && std::fs::rename(&tmp, &fichier).is_ok() {
        // cache borné : les modèles les plus anciens sont oubliés
        if let Ok(rd) = std::fs::read_dir(&dossier) {
            let mut f: Vec<(std::time::SystemTime, std::path::PathBuf)> = rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
            f.sort();
            for (_, p) in f.iter().rev().skip(30) { let _ = std::fs::remove_file(p); }
        }
    }
    Ok((v, c))
}
/// Ouverture d'un fichier par l'interface (bouton Fichier, démarrage de la visionneuse) : ses objets,
/// séparés pour un 3MF ou une conception SolidCraft à plusieurs corps, le groupe posé à l'origine.
fn charger_objets(p: &std::path::Path) -> Result<Vec<(Vec<f32>, Option<Vec<f32>>)>, String> {
    // jauge : lecture 0-60 % (le lecteur STL avance au fil des triangles), analyse des défauts ensuite
    progress::reset(); progress::span(0.0, 0.6);
    let chemin = p.to_str().unwrap_or("");
    let ext_o = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    // fichier lu UNE seule fois (une conception SolidCraft paramétrique demande un calcul long à SolidCraft)
    let mut objs = match ext_o.as_str() {
        // 3MF / SolidCraft à plusieurs objets : chacun reste séparé (positions relatives gardées)
        "3mf" => match threemf::read_objets(chemin) { Ok(o) if !o.is_empty() => o, _ => { let (nm, c) = load_model(chemin)?; vec![(nm.verts, c)] } },
        "solidcraft" | "solvecraft" => solidcraft::lire(chemin)?,
        _ => { let (nm, c) = load_model(chemin)?; vec![(nm.verts, c)] }
    };
    progress::set(1.0);
    // le groupe est posé à l'origine (coin bas à 0), comme un objet seul
    let mut lo = Vec3::splat(f32::INFINITY);
    for (v, _) in &objs { for q in v.chunks_exact(6) { lo = lo.min(Vec3::new(q[0], q[1], q[2])); } }
    for (v, _) in objs.iter_mut() { for q in v.chunks_exact_mut(6) { q[0] -= lo.x; q[1] -= lo.y; q[2] -= lo.z; } }
    Ok(objs)
}
/// Défauts du dernier objet ouvert (celui qui sera actif), analysés dans le fil de chargement : la fenêtre
/// n'est pas figée par l'analyse d'un gros maillage à son arrivée. Jauge : 60-95 %.
fn analyse_ouverture(objs: &[(Vec<f32>, Option<Vec<f32>>)]) -> Option<defects::Defects> {
    progress::span(0.6, 0.95);
    let d = objs.last().map(|(v, _)| defects::analyze_suivi(v, true));
    progress::set(1.0);
    d
}
/// Couleurs du dernier modèle chargé en ligne de commande (pour l'affichage au démarrage).
static CLI_COLS: std::sync::Mutex<Option<Vec<f32>>> = std::sync::Mutex::new(None);

fn mesh_from_arg(path: &str) -> stl::Mesh {
    if path == "prim:logo" || path == "logo" {
        let (v, _) = logo3d();
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for c in v.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
        return stl::Mesh { verts: v, min: lo, max: hi };
    }
    if path == "prim:demo" || path == "demo" {
        // Boîte à congés : vitrine du noyau OCCT, objet de démarrage par défaut.
        let mut m = occt::demo(40.0, 30.0, 20.0, 5.0, occt::DEFLEXION)
            .unwrap_or_else(|rc| { eprintln!("démo OCCT a échoué (code {rc})"); std::process::exit(1); });
        clean_prim(&mut m);
        return m;
    }
    if let Some(name) = path.strip_prefix("prim:") {
        let p = match name {
            "box" => occt::Prim::Box { dx: 40.0, dy: 30.0, dz: 20.0 },
            "sphere" => occt::Prim::Sphere { r: 15.0 },
            "cyl" | "cylinder" => occt::Prim::Cylinder { r: 12.0, h: 30.0 },
            "cone" => occt::Prim::Cone { r1: 15.0, r2: 0.0, h: 30.0 },
            "torus" => occt::Prim::Torus { big: 18.0, small: 6.0 },
            other => { eprintln!("primitive inconnue : {other}"); std::process::exit(1); }
        };
        { let mut m = occt::primitive(p, occt::DEFLEXION).unwrap_or_else(|rc| { eprintln!("OCCT a échoué (code {rc})"); std::process::exit(1); }); clean_prim(&mut m); m }
    } else {
        match load_model(path) {
            Ok((m, c)) => { *CLI_COLS.lock().unwrap() = c; m }
            Err(e) => { eprintln!("modèle illisible ({path}) : {e}"); std::process::exit(1); }
        }
    }
}

fn ic_stroke() -> egui::Stroke { egui::Stroke::new(1.6, egui::Color32::from_gray(235)) }
fn draw_arc(p: &egui::Painter, c: egui::Pos2, rad: f32, a0: f32, a1: f32, st: egui::Stroke) {
    let n = 18; let mut prev = None;
    for i in 0..=n { let a = a0 + (a1 - a0) * (i as f32 / n as f32); let pt = egui::pos2(c.x + rad * a.cos(), c.y + rad * a.sin()); if let Some(pp) = prev { p.line_segment([pp, pt], st); } prev = Some(pt); }
}
fn draw_arrowhead(p: &egui::Painter, tip: egui::Pos2, dir: egui::Vec2, st: egui::Stroke) {
    let d = dir.normalized(); let n = egui::vec2(-d.y, d.x); let back = tip - d * 6.0;
    p.line_segment([tip, back + n * 4.0], st); p.line_segment([tip, back - n * 4.0], st);
}
fn draw_folder(p: &egui::Painter, r: egui::Rect) {
    let y = egui::Color32::from_rgb(226, 190, 90);
    let tab = egui::Rect::from_min_max(egui::pos2(r.left(), r.top() + r.height() * 0.16), egui::pos2(r.left() + r.width() * 0.5, r.top() + r.height() * 0.4));
    let body = egui::Rect::from_min_max(egui::pos2(r.left(), r.top() + r.height() * 0.3), r.max);
    p.rect_filled(tab, 1.0, y); p.rect_filled(body, 2.0, y);
    p.rect_stroke(body, 2.0, egui::Stroke::new(1.0, egui::Color32::from_gray(70)));
}
/// Appareil photo : boîtier arrondi, bosse du viseur, objectif, flash.
fn draw_camera(p: &egui::Painter, r: egui::Rect) {
    let col = egui::Color32::from_gray(235);
    let body = egui::Rect::from_min_max(egui::pos2(r.left() + r.width() * 0.06, r.top() + r.height() * 0.30), egui::pos2(r.right() - r.width() * 0.06, r.bottom() - r.height() * 0.12));
    let bump = egui::Rect::from_min_max(egui::pos2(r.left() + r.width() * 0.32, r.top() + r.height() * 0.16), egui::pos2(r.left() + r.width() * 0.62, body.top() + 1.0));
    p.rect_filled(bump, 2.0, col);
    p.rect_filled(body, 3.0, col);
    let c = egui::pos2(body.center().x, body.center().y + 0.5);
    let rad = body.height() * 0.36;
    p.circle_filled(c, rad, egui::Color32::from_gray(45));
    p.circle_filled(c, rad * 0.62, egui::Color32::from_rgb(95, 165, 230));
    p.circle_filled(egui::pos2(c.x - rad * 0.25, c.y - rad * 0.25), rad * 0.18, egui::Color32::WHITE);
    p.circle_filled(egui::pos2(body.right() - body.width() * 0.13, body.top() + body.height() * 0.2), body.height() * 0.08, egui::Color32::from_rgb(232, 168, 66));
}
fn draw_floppy(p: &egui::Painter, r: egui::Rect) {
    p.rect_filled(r, 2.0, egui::Color32::from_rgb(90, 140, 200));
    p.rect_stroke(r, 2.0, egui::Stroke::new(1.0, egui::Color32::from_gray(40)));
    let sl = egui::Rect::from_min_max(egui::pos2(r.left() + r.width() * 0.25, r.top()), egui::pos2(r.right() - r.width() * 0.2, r.top() + r.height() * 0.32));
    p.rect_filled(sl, 0.0, egui::Color32::from_gray(235));
    let lb = egui::Rect::from_min_max(egui::pos2(r.left() + r.width() * 0.22, r.center().y), egui::pos2(r.right() - r.width() * 0.22, r.bottom() - r.height() * 0.1));
    p.rect_filled(lb, 0.0, egui::Color32::from_gray(235));
}
/// Flèche de retour (↶ pour Annuler, ↷ en miroir pour Refaire) : pointe vers l'arrière,
/// fait demi-tour par une boucle, comme dans les logiciels de bureautique.
fn draw_back_arrow(p: &egui::Painter, r: egui::Rect, mirror: bool, col: egui::Color32) {
    let st = egui::Stroke::new(2.2, col);
    let (w, h) = (r.width(), r.height());
    let fx = |x: f32| if mirror { r.right() - x * w } else { r.left() + x * w };
    let y = |v: f32| r.top() + v * h;
    let (ytop, ybot, cx, rad) = (0.32, 0.80, 0.58, 0.24);
    let mut pts = vec![egui::pos2(fx(0.14), y(ytop)), egui::pos2(fx(cx), y(ytop))];
    for k in 1..=16 {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * k as f32 / 16.0;
        pts.push(egui::pos2(fx(cx + rad * a.cos() * h / w), y((ytop + ybot) * 0.5 + rad * a.sin())));
    }
    pts.push(egui::pos2(fx(0.30), y(ybot)));
    p.add(egui::Shape::line(pts, st));
    // pointe de flèche pleine
    let tip = egui::pos2(fx(0.02), y(ytop));
    let s = if mirror { -1.0 } else { 1.0 };
    p.add(egui::Shape::convex_polygon(vec![tip, egui::pos2(tip.x + s * 0.22 * w, y(ytop) - 0.17 * h), egui::pos2(tip.x + s * 0.22 * w, y(ytop) + 0.17 * h)], col, egui::Stroke::NONE));
}
/// Jauge de calcul : tube de verre arrondi aspect 3D. Au repos = tube noir, rien d'affiché ;
/// en calcul = mercure rouge (reflet haut, ombre basse) qui avance, pourcentage lisible.
fn thermo_gauge(ui: &mut egui::Ui, frac: f32, active: bool, text: &str, w: f32) { thermo_gauge_c(ui, frac, active, text, w, false) }
/// Idem ; `vert` = mercure VERT (menu Fichier : jauge fixe, pleine).
fn thermo_gauge_c(ui: &mut egui::Ui, frac: f32, active: bool, text: &str, w: f32, vert: bool) { thermo_gauge_k(ui, frac, active, text, w, vert, 1.0) }
/// Idem, toutes les dimensions multipliées par `k` (jauge EXPORTATION de la visionneuse : ×1,5).
fn thermo_gauge_k(ui: &mut egui::Ui, frac: f32, active: bool, text: &str, w: f32, vert: bool, k: f32) {
    use egui::{pos2, vec2, Color32, Rect, Rounding, Stroke};
    let (rect, _) = ui.allocate_exact_size(vec2(w, 22.0 * k), egui::Sense::hover());
    let p = ui.painter();
    let tube = rect.shrink2(vec2(k, k));
    let rt = Rounding::same(tube.height() * 0.5);
    // verre noir + léger volume (ombre interne en haut, reflet en bas)
    p.rect_filled(tube, rt, Color32::from_gray(8));
    p.rect_filled(Rect::from_min_max(pos2(tube.left() + 6.0 * k, tube.bottom() - 5.0 * k), pos2(tube.right() - 6.0 * k, tube.bottom() - 2.0 * k)), Rounding::same(1.5 * k), Color32::from_white_alpha(10));
    if active {
        let inner = tube.shrink(2.5 * k);
        let h = inner.height();
        let fill_r = inner.left() + inner.width() * frac.clamp(0.0, 1.0);
        let merc = Rect::from_min_max(inner.min, pos2(fill_r.max(inner.left() + h), inner.max.y));
        let rm = Rounding::same(h * 0.5);
        // mercure : corps, ombre basse, reflet haut => cylindre (rouge = calcul, vert = fichier)
        let (corps, ombre, milieu, reflet) = if vert {
            (Color32::from_rgb(30, 175, 60), Color32::from_rgb(12, 105, 30), Color32::from_rgb(26, 155, 52), Color32::from_rgba_unmultiplied(170, 255, 180, 170))
        } else {
            (Color32::from_rgb(200, 25, 25), Color32::from_rgb(130, 10, 10), Color32::from_rgb(185, 20, 20), Color32::from_rgba_unmultiplied(255, 170, 160, 170))
        };
        p.rect_filled(merc, rm, corps);
        p.rect_filled(Rect::from_min_max(pos2(merc.left(), merc.top() + h * 0.55), merc.max), rm, ombre);
        p.rect_filled(Rect::from_min_max(pos2(merc.left(), merc.top() + h * 0.40), pos2(merc.right(), merc.top() + h * 0.75)), Rounding::same(h * 0.2), milieu);
        p.rect_filled(Rect::from_min_max(pos2(merc.left() + h * 0.4, merc.top() + 1.5 * k), pos2((merc.right() - h * 0.4).max(merc.left() + h * 0.6), merc.top() + h * 0.32)), Rounding::same(h * 0.16), reflet);
    }
    p.rect_stroke(tube, rt, Stroke::new(1.2 * k, Color32::from_gray(if active { 150 } else { 90 })));
    // reflet du verre sur toute la longueur
    p.line_segment([pos2(tube.left() + 8.0 * k, tube.top() + 2.5 * k), pos2(tube.right() - 8.0 * k, tube.top() + 2.5 * k)], Stroke::new(k, Color32::from_white_alpha(35)));
    if active {
        // texte gras blanc cerclé de noir : lisible sur le rouge comme sur le noir
        let c = tube.center();
        let font = egui::FontId::proportional(12.5 * k);
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
            p.text(c + vec2(dx, dy) * k, egui::Align2::CENTER_CENTER, text, font.clone(), Color32::BLACK);
        }
        p.text(c, egui::Align2::CENTER_CENTER, text, font.clone(), Color32::WHITE);
        p.text(c + vec2(0.5 * k, 0.0), egui::Align2::CENTER_CENTER, text, font, Color32::WHITE);
    }
}
/// Engrenage seul (sans libellé) : ouvre le menu Options.
/// Engrenage Options : BLEU en relief (ombre portée, face éclairée, reflet), carré `taille`.
fn gear_button(ui: &mut egui::Ui, open: bool, taille: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(taille, taille), egui::Sense::click());
    let p = ui.painter();
    if open || resp.hovered() {
        p.rect_filled(rect, 5.0, if open { egui::Color32::from_gray(55) } else { egui::Color32::from_gray(45) });
    }
    let r = rect.shrink(1.5);
    let fond = if open { egui::Color32::from_gray(55) } else if resp.hovered() { egui::Color32::from_gray(45) } else { ui.visuals().panel_fill };
    // rotation permanente : ~1 tour / 6 s, ~12 images/s (la scène 3D, inchangée, est recopiée : voir cache3d)
    let rot = (ui.input(|i| i.time) as f32 * std::f32::consts::TAU / 6.0) % std::f32::consts::TAU;
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    // ombre (décalée bas-droite), corps bleu, reflet (décalé haut-gauche, plus clair, rogné)
    draw_gear_c(p, r.translate(egui::vec2(1.6, 1.8)), egui::Color32::from_rgb(10, 25, 45), egui::Color32::TRANSPARENT, rot);
    draw_gear_c(p, r, egui::Color32::from_rgb(40, 110, 200), fond, rot);
    let h = r.shrink(r.width() * 0.17).translate(egui::vec2(-r.width() * 0.05, -r.width() * 0.05));
    p.circle_stroke(h.center(), h.width() * 0.30, egui::Stroke::new(r.width() * 0.07, egui::Color32::from_rgba_unmultiplied(150, 205, 255, 150)));
    p.circle_filled(r.center(), r.width() * 0.15, fond);
    p.circle_stroke(r.center(), r.width() * 0.15, egui::Stroke::new(1.0, egui::Color32::from_rgb(20, 60, 120)));
    resp
}
fn draw_gear_c(p: &egui::Painter, r: egui::Rect, col: egui::Color32, hole: egui::Color32, rot: f32) {
    let c = r.center();
    let w = r.width().min(r.height());
    let (rb, rt) = (w * 0.34, w * 0.49);
    let n = 8;
    let step = std::f32::consts::TAU / n as f32;
    let pt = |a: f32, rad: f32| egui::pos2(c.x + rad * a.cos(), c.y + rad * a.sin());
    for i in 0..n {
        let a = i as f32 * step + rot;
        let (hb, ht) = (step * 0.30, step * 0.19);
        p.add(egui::Shape::convex_polygon(vec![pt(a - hb, rb - 1.0), pt(a - ht, rt), pt(a + ht, rt), pt(a + hb, rb - 1.0)], col, egui::Stroke::NONE));
    }
    p.circle_filled(c, rb, col);
    if hole != egui::Color32::TRANSPARENT { p.circle_filled(c, w * 0.15, hole); }
}
const JAUNE: egui::Color32 = egui::Color32::from_rgb(235, 204, 84);
/// Bouton jaune du panneau de droite (texte noir).
fn yellow_btn(text: &str, size: f32) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).size(size).strong().color(egui::Color32::BLACK))
        .fill(JAUNE).stroke(egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 232, 150))).rounding(egui::Rounding::same(6.0))
}
/// Bleu des titres, accordé au thème : bleu ciel lisible sur fond sombre, bleu de sélection sur fond clair.
fn title_blue(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode { egui::Color32::from_rgb(110, 185, 245) } else { egui::Color32::from_rgb(15, 75, 150) }
}
/// Titre de menu : MAJUSCULES, texte noir sur bandeau JAUNE (style des boutons jaunes).
fn yellow_title(ui: &mut egui::Ui, text: &str) {
    let w = ui.available_width().max(150.0);
    let (r, _) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 6.0, JAUNE);
    ui.painter().rect_stroke(r, 6.0, egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 232, 150)));
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text.to_uppercase(), egui::FontId::proportional(13.0), egui::Color32::BLACK);
}
/// Titre encadré dans le bleu de sélection (pleine largeur, ou `w`).
fn title_box(ui: &mut egui::Ui, text: &str, size: f32, w: f32) -> egui::Response {
    let blue = title_blue(ui);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, size + 10.0), egui::Sense::hover());
    ui.painter().rect_stroke(r.shrink(0.75), 5.0, egui::Stroke::new(1.5, blue));
    ui.painter().text(egui::pos2(r.left() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(size), blue);
    resp
}
/// Titre de section dépliable, encadré en bleu (triangle ▶/▼) ; état mémorisé. Renvoie « ouvert ».
fn section(ui: &mut egui::Ui, key: &str, text: &str) -> bool {
    let id = ui.make_persistent_id(key);
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, false));
    // test sans clic : PATCH3D_SECTION=<clé> ouvre cette section au démarrage
    if ui.ctx().frame_nr() < 3 && std::env::var("PATCH3D_SECTION").as_deref() == Ok(key) { open = true; ui.data_mut(|d| d.insert_persisted(id, true)); }
    let blue = title_blue(ui);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    if resp.clicked() { open = !open; ui.data_mut(|d| d.insert_persisted(id, open)); }
    let p = ui.painter();
    if resp.hovered() { p.rect_filled(r.shrink(0.75), 5.0, blue.gamma_multiply(0.25)); }
    p.rect_stroke(r.shrink(0.75), 5.0, egui::Stroke::new(1.5, blue));
    let c = egui::pos2(r.left() + 13.0, r.center().y);
    let tri = if open { vec![c + egui::vec2(-5.0, -3.0), c + egui::vec2(5.0, -3.0), c + egui::vec2(0.0, 4.0)] }
              else { vec![c + egui::vec2(-3.0, -5.0), c + egui::vec2(4.0, 0.0), c + egui::vec2(-3.0, 5.0)] };
    p.add(egui::Shape::convex_polygon(tri, blue, egui::Stroke::NONE));
    p.text(egui::pos2(r.left() + 26.0, r.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(14.0), blue);
    open
}
/// Bulle de l'option « Activer GPU » : ce que fait l'option + toutes les mesures de la machine.
fn perf_tooltip(lang: Lang, gpu_name: &str) -> String {
    let p = perf::profile();
    let fr = |x: f64| format!("{x:.2}").replace('.', ",");
    let mo = |x: u64| { let s = x.to_string(); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push('\u{202F}'); } o.push(c); } o };
    let mut t = format!("{}\n{}\n\n", tr(lang, "gpu_tip"), tr(lang, "m_adapt"));
    t += &format!("• {} : {} {} — {} Gvox/s\n", tr(lang, "m_cpu"), p.threads, tr(lang, "m_cores"), fr(p.cpu_gvox));
    if let Some(r) = p.ram_mb { t += &format!("• {} : {} Mo\n", tr(lang, "m_ram"), mo(r)); }
    if let Some(g) = p.gpu_gvox {
        t += &format!("• GPU : {gpu_name}\n   {} Gvox/s (×{} {})\n", fr(g), fr(g / p.cpu_gvox.max(1e-6)), tr(lang, "m_vs_cpu"));
    }
    if let Some(v) = p.vram_mb { t += &format!("• {} : {} Mo\n", tr(lang, "m_vram"), mo(v.max(0) as u64)); }
    let moteur = if perf::use_gpu() { tr(lang, "m_eng_gpu") }
                 else if perf::GPU_CHECKED.load(std::sync::atomic::Ordering::Relaxed) { tr(lang, "m_eng_slow") }
                 else { tr(lang, "m_eng_cpu") };
    t += &format!("\n{} : {moteur}\n", tr(lang, "m_engine"));
    t += &format!("{} : {} {} · {} {}", tr(lang, "m_res"), tr(lang, "simp_env"), perf::env_res(), tr(lang, "lissage"), perf::liss_res());
    t
}

/// Case à cocher : JAUNE (coche noire) si activée, VIDE (bord gris) sinon. Renvoie la réponse (clic).
fn gpu_check(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    let p = ui.painter();
    let b = egui::Rect::from_center_size(egui::pos2(r.left() + 11.0, r.center().y), egui::vec2(16.0, 16.0));
    if on {
        p.rect_filled(b, 3.0, JAUNE);
        p.add(egui::Shape::line(vec![b.left_center() + egui::vec2(3.0, 0.0), b.center_bottom() + egui::vec2(-1.0, -4.0), b.right_top() + egui::vec2(-3.0, 4.0)], egui::Stroke::new(2.2, egui::Color32::BLACK)));
    } else {
        p.rect_stroke(b, 3.0, egui::Stroke::new(1.5, egui::Color32::from_gray(if resp.hovered() { 220 } else { 150 })));
    }
    p.text(egui::pos2(b.right() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), ui.visuals().text_color());
    resp
}
/// Bouton Annuler / Refaire du panneau d'outils : flèche + texte, grisé si indisponible.
/// Case à cocher jaune (bien visible) : carré jaune plein + coche noire quand cochée.
fn case_jaune(ui: &mut egui::Ui, val: &mut bool, texte: &str) -> egui::Response {
    let police = egui::FontId::proportional(13.0);
    let tw = ui.fonts(|f| f.layout_no_wrap(texte.to_string(), police.clone(), egui::Color32::WHITE).size().x);
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(24.0 + tw, 22.0), egui::Sense::click());
    if resp.clicked() { *val = !*val; resp.mark_changed(); }
    let p = ui.painter();
    let b = egui::Rect::from_center_size(egui::pos2(rect.left() + 9.0, rect.center().y), egui::vec2(17.0, 17.0));
    let clair = if resp.hovered() { egui::Color32::from_rgb(250, 222, 120) } else { JAUNE };
    if *val {
        p.rect_filled(b, 3.0, clair);
        p.line_segment([b.left_center() + egui::vec2(3.5, 0.5), b.center_bottom() + egui::vec2(-1.0, -4.0)], egui::Stroke::new(2.4, egui::Color32::BLACK));
        p.line_segment([b.center_bottom() + egui::vec2(-1.0, -4.0), b.right_top() + egui::vec2(-3.5, 4.0)], egui::Stroke::new(2.4, egui::Color32::BLACK));
    } else {
        p.rect_stroke(b, 3.0, egui::Stroke::new(1.8, clair));
    }
    p.text(egui::pos2(rect.left() + 24.0, rect.center().y), egui::Align2::LEFT_CENTER, texte, police, if *val { clair } else { ui.visuals().text_color() });
    resp
}

/// Bouton radio jaune : rond jaune plein et texte jaune quand il est choisi.
fn radio_jaune(ui: &mut egui::Ui, choisi: bool, texte: &str) -> egui::Response {
    let police = egui::FontId::proportional(14.0);
    let tw = ui.fonts(|f| f.layout_no_wrap(texte.to_string(), police.clone(), egui::Color32::WHITE).size().x);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0 + tw, 24.0), egui::Sense::click());
    let p = ui.painter();
    let c = egui::pos2(rect.left() + 10.0, rect.center().y);
    let clair = if resp.hovered() { egui::Color32::from_rgb(250, 222, 120) } else { JAUNE };
    p.circle_stroke(c, 8.0, egui::Stroke::new(2.0, if choisi { clair } else { egui::Color32::from_gray(150) }));
    if choisi { p.circle_filled(c, 5.0, clair); }
    p.text(egui::pos2(rect.left() + 26.0, rect.center().y), egui::Align2::LEFT_CENTER, texte, police, if choisi { clair } else { ui.visuals().text_color() });
    resp
}

fn history_button(ui: &mut egui::Ui, label: &str, enabled: bool, redo: bool, w: f32, h: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let p = ui.painter();
    let bg = if !enabled { egui::Color32::from_gray(45) } else if resp.hovered() { egui::Color32::from_rgb(250, 222, 120) } else { JAUNE };
    p.rect_filled(rect, 6.0, bg);
    p.rect_stroke(rect, 6.0, egui::Stroke::new(1.0, if enabled { egui::Color32::from_rgb(255, 232, 150) } else { egui::Color32::from_gray(70) }));
    // flèche : clignote en rouge (créneau 0,5 s) tant que l'action est disponible
    let col = if enabled {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        if (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0 { egui::Color32::from_rgb(220, 20, 20) } else { egui::Color32::from_rgb(120, 60, 40).gamma_multiply(0.45) }
    } else { egui::Color32::from_gray(95) };
    let txt = if enabled { egui::Color32::BLACK } else { egui::Color32::from_gray(110) };
    let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), egui::vec2(20.0, 18.0));
    draw_back_arrow(p, ir, redo, col);
    p.text(egui::pos2(rect.left() + 32.0, rect.center().y), egui::Align2::LEFT_CENTER, label.to_uppercase(), egui::FontId::proportional(11.5), txt);
    resp
}
fn draw_cross(p: &egui::Painter, r: egui::Rect) {
    let st = egui::Stroke::new(2.4, egui::Color32::from_rgb(220, 70, 70));
    let m = r.width() * 0.22;
    p.line_segment([r.min + egui::vec2(m, m), r.max - egui::vec2(m, m)], st);
    p.line_segment([egui::pos2(r.right() - m, r.top() + m), egui::pos2(r.left() + m, r.bottom() - m)], st);
}
/// Engrenage plein : roue + 8 dents trapézoïdales + moyeu évidé (`hole` = couleur du fond).
fn draw_gear(p: &egui::Painter, r: egui::Rect, hole: egui::Color32) {
    let col = egui::Color32::from_rgb(235, 204, 84); // jaune (comme le bouton de vue actif)
    let c = r.center();
    let w = r.width().min(r.height());
    let (rb, rt) = (w * 0.34, w * 0.49); // rayon de la roue, rayon en bout de dent
    let n = 8;
    let step = std::f32::consts::TAU / n as f32;
    let pt = |a: f32, rad: f32| egui::pos2(c.x + rad * a.cos(), c.y + rad * a.sin());
    for i in 0..n {
        let a = i as f32 * step;
        // dent : large à la base, plus étroite au sommet
        let (hb, ht) = (step * 0.30, step * 0.19);
        p.add(egui::Shape::convex_polygon(vec![pt(a - hb, rb - 1.0), pt(a - ht, rt), pt(a + ht, rt), pt(a + hb, rb - 1.0)], col, egui::Stroke::NONE));
    }
    p.circle_filled(c, rb, col);
    p.circle_filled(c, w * 0.15, hole);
}
fn draw_help(p: &egui::Painter, r: egui::Rect) {
    let c = r.center();
    p.circle_stroke(c, r.width() * 0.4, ic_stroke());
    p.text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(15.0), egui::Color32::from_gray(235));
}
fn draw_cursor(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke();
    let a = r.min + egui::vec2(r.width() * 0.32, r.height() * 0.2);
    let b = r.min + egui::vec2(r.width() * 0.32, r.height() * 0.8);
    let cc = r.min + egui::vec2(r.width() * 0.76, r.height() * 0.56);
    p.line_segment([a, b], st); p.line_segment([a, cc], st); p.line_segment([b, cc], st);
}
/// Logo OBJ-AXE : les trois anneaux de l'outil de rotation (X rouge, Y vert, Z bleu clair),
/// directement sur le fond bleu du bouton, sans poignées.
/// Logo 3D-IMP : imprimante 3D (cadre, portique, buse, plateau, pièce en cours d'impression).
fn draw_imprimante(p: &egui::Painter, r: egui::Rect) {
    let (w, h) = (r.width(), r.height());
    let pt = |x: f32, y: f32| egui::pos2(r.left() + w * x, r.top() + h * y);
    let gris = egui::Stroke::new((w * 0.07).max(1.4), egui::Color32::from_gray(225));
    // cadre : deux montants et la traverse du haut
    p.line_segment([pt(0.12, 0.95), pt(0.12, 0.08)], gris);
    p.line_segment([pt(0.88, 0.95), pt(0.88, 0.08)], gris);
    p.line_segment([pt(0.12, 0.08), pt(0.88, 0.08)], gris);
    // portique (axe X) et chariot avec buse
    p.line_segment([pt(0.12, 0.36), pt(0.88, 0.36)], egui::Stroke::new((w * 0.06).max(1.2), egui::Color32::from_rgb(120, 180, 240)));
    p.rect_filled(egui::Rect::from_min_max(pt(0.40, 0.28), pt(0.60, 0.46)), 1.5, egui::Color32::from_rgb(120, 180, 240));
    p.add(egui::Shape::convex_polygon(vec![pt(0.45, 0.46), pt(0.55, 0.46), pt(0.50, 0.56)], egui::Color32::from_rgb(255, 120, 60), egui::Stroke::NONE));
    // pièce en cours (jaune) sur le plateau
    p.rect_filled(egui::Rect::from_min_max(pt(0.38, 0.66), pt(0.62, 0.84)), 1.5, egui::Color32::from_rgb(240, 200, 60));
    // plateau
    p.line_segment([pt(0.20, 0.86), pt(0.80, 0.86)], egui::Stroke::new((w * 0.09).max(1.8), egui::Color32::from_gray(200)));
}

/// Logo FORME (mise en forme) : cube en perspective, flèche courbe de rotation et double flèche
/// d'agrandissement.
fn draw_forme(p: &egui::Painter, r: egui::Rect) {
    let (w, h) = (r.width(), r.height());
    let pt = |x: f32, y: f32| egui::pos2(r.left() + w * x, r.top() + h * y);
    let trait_ = egui::Stroke::new((w * 0.05).max(1.0), egui::Color32::from_gray(40));
    // cube : face avant + dessus + côté
    let (a, b, c, d) = (pt(0.14, 0.42), pt(0.56, 0.42), pt(0.56, 0.86), pt(0.14, 0.86));
    let (e, f, g) = (pt(0.30, 0.28), pt(0.72, 0.28), pt(0.72, 0.72));
    p.add(egui::Shape::convex_polygon(vec![a, b, c, d], egui::Color32::from_gray(205), trait_));
    p.add(egui::Shape::convex_polygon(vec![a, e, f, b], egui::Color32::from_gray(245), trait_));
    p.add(egui::Shape::convex_polygon(vec![b, f, g, c], egui::Color32::from_gray(150), trait_));
    // flèche courbe de rotation (jaune) en haut à droite
    let jaune = egui::Color32::from_rgb(255, 205, 40);
    let arc: Vec<egui::Pos2> = (0..=12).map(|i| { let t = -2.6 + i as f32 / 12.0 * 2.3; pt(0.62 + 0.30 * t.cos(), 0.30 + 0.26 * t.sin()) }).collect();
    let fin = *arc.last().unwrap();
    p.add(egui::Shape::line(arc, egui::Stroke::new((w * 0.10).max(2.0), jaune)));
    p.add(egui::Shape::convex_polygon(vec![fin + egui::vec2(w * 0.10, -h * 0.02), fin + egui::vec2(-w * 0.06, -h * 0.08), fin + egui::vec2(-w * 0.02, h * 0.10)], jaune, egui::Stroke::NONE));
    // double flèche d'agrandissement (jaune) le long de la diagonale bas-droite
    let (q0, q1) = (pt(0.66, 0.78), pt(0.94, 0.98));
    p.line_segment([q0, q1], egui::Stroke::new((w * 0.10).max(2.0), jaune));
    for (bout, sens) in [(q1, 1.0f32), (q0, -1.0)] {
        let dir = (q1 - q0).normalized() * sens;
        let n = egui::vec2(-dir.y, dir.x);
        p.add(egui::Shape::convex_polygon(vec![bout + dir * w * 0.05, bout - dir * w * 0.09 + n * w * 0.07, bout - dir * w * 0.09 - n * w * 0.07], jaune, egui::Stroke::NONE));
    }
}

fn draw_obj_axe(p: &egui::Painter, r: egui::Rect) {
    let c = r.center();
    let rr = r.width().min(r.height()) * 0.5;
    let (rx, ry) = (rr * 1.05, rr * 1.05);
    let anneau = |a: f32, b: f32, rot: f32, col: egui::Color32| {
        let pts: Vec<egui::Pos2> = (0..=48).map(|i| {
            let t = i as f32 / 48.0 * std::f32::consts::TAU;
            let (x, y) = (a * t.cos(), b * t.sin());
            egui::pos2(c.x + x * rot.cos() - y * rot.sin(), c.y + x * rot.sin() + y * rot.cos())
        }).collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(2.2, col)));
    };
    anneau(rx, ry * 0.34, 0.0, egui::Color32::from_rgb(150, 210, 255));   // Z : anneau horizontal, bleu clair
    anneau(rx * 0.40, ry, 0.42, egui::Color32::from_rgb(80, 235, 80));    // Y
    anneau(rx * 0.40, ry, -0.42, egui::Color32::from_rgb(255, 75, 75));   // X
    p.circle_filled(c, 1.8, egui::Color32::WHITE);
}

/// Diagramme de Venn de deux disques A (gauche) et B (droite), la région du résultat remplie.
/// `op` : None = logo du bouton BOOLÉENS (A orange, B bleu, sans résultat).
/// Sommets d'un cube vu en isométrique dans `r` : (haut, haut-droit, bas-droit, bas, bas-gauche, haut-gauche, centre).
fn cube_iso(r: egui::Rect) -> [egui::Pos2; 7] {
    let (c, h) = (r.center(), r.height() * 0.42);
    let x = h * 0.866;
    [egui::pos2(c.x, c.y - h), egui::pos2(c.x + x, c.y - h * 0.5), egui::pos2(c.x + x, c.y + h * 0.5), egui::pos2(c.x, c.y + h),
     egui::pos2(c.x - x, c.y + h * 0.5), egui::pos2(c.x - x, c.y - h * 0.5), c]
}
/// Logo fil de fer : cube en arêtes claires, faces coupées par leur diagonale (triangles).
fn draw_fil_de_fer(p: &egui::Painter, r: egui::Rect) {
    let [t, tr, br, b, bl, tl, m] = cube_iso(r);
    let st = egui::Stroke::new(1.4, egui::Color32::from_rgb(200, 230, 255));
    for (a, z) in [(t, tr), (tr, br), (br, b), (b, bl), (bl, tl), (tl, t), (m, tl), (m, tr), (m, b)] { p.line_segment([a, z], st); }
    let fin = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(200, 230, 255, 150));
    for (a, z) in [(tl, tr), (tl, b), (m, br)] { p.line_segment([a, z], fin); }
}
/// Logo facettes : cube plein, faces dorées ombrées (dessus clair, gauche moyen, droite sombre).
fn draw_facettes(p: &egui::Painter, r: egui::Rect) {
    let [t, tr, br, b, bl, tl, m] = cube_iso(r);
    let bord = egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 45, 10));
    p.add(egui::Shape::convex_polygon(vec![t, tr, m, tl], egui::Color32::from_rgb(250, 222, 120), bord));
    p.add(egui::Shape::convex_polygon(vec![tl, m, b, bl], egui::Color32::from_rgb(215, 175, 60), bord));
    p.add(egui::Shape::convex_polygon(vec![m, tr, br, b], egui::Color32::from_rgb(150, 115, 30), bord));
}
fn draw_venn(p: &egui::Painter, r: egui::Rect, op: Option<booleen::Op>, plein: egui::Color32, fond: egui::Color32, trait_: egui::Color32) {
    let w = r.width().min(r.height());
    let rad = w * 0.30;
    let (ca, cb) = (r.center() - egui::vec2(w * 0.16, 0.0), r.center() + egui::vec2(w * 0.16, 0.0));
    let st = egui::Stroke::new(1.6, trait_);
    match op {
        None => {
            p.circle_filled(ca, rad, egui::Color32::from_rgb(240, 150, 60).gamma_multiply(0.85));
            p.circle_filled(cb, rad, egui::Color32::from_rgb(110, 180, 255).gamma_multiply(0.85));
        }
        Some(booleen::Op::Union) => { p.circle_filled(ca, rad, plein); p.circle_filled(cb, rad, plein); }
        Some(booleen::Op::Soustraction) => { p.circle_filled(ca, rad, plein); p.circle_filled(cb, rad, fond); }
        Some(booleen::Op::Intersection) => {
            // lentille commune = points de chaque cercle situés dans l'autre (polygone convexe)
            let n = 48;
            let mut pts: Vec<egui::Pos2> = Vec::new();
            for (c, o) in [(ca, cb), (cb, ca)] {
                for i in 0..n {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    let q = c + egui::vec2(a.cos(), a.sin()) * rad;
                    if (q - o).length() <= rad + 0.01 { pts.push(q); }
                }
            }
            let m = r.center();
            pts.sort_by(|a, b| (a.y - m.y).atan2(a.x - m.x).partial_cmp(&(b.y - m.y).atan2(b.x - m.x)).unwrap());
            if pts.len() >= 3 { p.add(egui::Shape::convex_polygon(pts, plein, egui::Stroke::NONE)); }
        }
    }
    p.circle_stroke(ca, rad, st);
    p.circle_stroke(cb, rad, st);
}
/// Ciseaux : deux lames croisées, deux anneaux en bas.
fn draw_ciseaux(p: &egui::Painter, r: egui::Rect, poignees: egui::Color32) {
    // ciseaux ouverts, pointes en haut : lames métalliques effilées qui se croisent au pivot,
    // anneaux de poignée colorés en bas, vis au pivot
    let (w, h) = (r.width(), r.height());
    let pt = |x: f32, y: f32| egui::pos2(r.left() + w * x, r.top() + h * y);
    let acier = egui::Color32::from_rgb(222, 226, 232);
    let ombre = egui::Color32::from_rgb(70, 74, 82);
    let pivot = egui::vec2(0.5, 0.46);
    for cote in [-1.0f32, 1.0] {
        // lame : de la poignée (en bas, côté opposé) vers la pointe (en haut), large au pivot
        let pointe = egui::vec2(0.5 + cote * 0.34, 0.02);
        let attache = egui::vec2(0.5 - cote * 0.12, 0.64);
        let d = (pointe - attache).normalized();
        let n = egui::vec2(-d.y, d.x) * cote;
        let poly = [pointe, pivot + n * 0.13 - d * 0.03, attache + n * 0.06, attache - n * 0.05, pivot - n * 0.02];
        p.add(egui::Shape::convex_polygon(poly.iter().map(|v| pt(v.x, v.y)).collect(), acier, egui::Stroke::new((w * 0.03).max(1.0), ombre)));
        // anneau de poignée (légèrement ovale : deux cercles décalés)
        let c = egui::vec2(0.5 - cote * 0.24, 0.80);
        let ep = egui::Stroke::new((w * 0.09).max(2.0), poignees);
        p.line_segment([pt(attache.x, attache.y), pt(c.x + cote * 0.08, c.y - 0.10)], ep);
        p.circle_stroke(pt(c.x, c.y), w * 0.15, ep);
    }
    // vis du pivot
    p.circle_filled(pt(pivot.x, pivot.y), w * 0.07, ombre);
    p.circle_filled(pt(pivot.x, pivot.y), w * 0.03, acier);
}

fn draw_booleens(p: &egui::Painter, r: egui::Rect) { draw_venn(p, r, None, egui::Color32::BLACK, egui::Color32::BLACK, egui::Color32::from_gray(235)); }
/// Bouton d'opération booléenne du panneau : jaune, diagramme à gauche, libellé noir.
fn bool_op_button(ui: &mut egui::Ui, op: booleen::Op, text: &str, actif: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), if actif { egui::Sense::click() } else { egui::Sense::hover() });
    let p = ui.painter();
    let fill = if !actif { JAUNE.gamma_multiply(0.35) } else if resp.hovered() { egui::Color32::from_rgb(250, 222, 120) } else { JAUNE };
    p.rect_filled(rect, 6.0, fill);
    p.rect_stroke(rect, 6.0, egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 232, 150)));
    let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(40.0, 34.0));
    draw_venn(p, ir, Some(op), egui::Color32::from_gray(30), fill, egui::Color32::BLACK);
    p.text(egui::pos2(rect.left() + 52.0, rect.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(13.5), egui::Color32::BLACK);
    resp
}

fn draw_center(p: &egui::Painter, r: egui::Rect) {
    let c = r.center(); let st = ic_stroke();
    p.circle_stroke(c, r.width() * 0.3, st);
    let m = r.width() * 0.42;
    p.line_segment([egui::pos2(c.x - m, c.y), egui::pos2(c.x + m, c.y)], st);
    p.line_segment([egui::pos2(c.x, c.y - m), egui::pos2(c.x, c.y + m)], st);
}
// ---- Icônes de l'overlay de navigation (trait clair sur fond sombre) ----
fn draw_nav_zone(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke();
    // cadre de sélection (en haut-gauche)
    let sq = egui::Rect::from_min_max(r.min + egui::vec2(1.0, 1.0), egui::pos2(r.center().x + 2.0, r.center().y + 2.0));
    p.rect_stroke(sq, 1.0, st);
    // petite loupe (en bas-droite)
    let c = egui::pos2(r.right() - r.width() * 0.26, r.bottom() - r.height() * 0.26);
    let rad = r.width() * 0.17;
    p.circle_stroke(c, rad, st);
    p.line_segment([egui::pos2(c.x + rad * 0.7, c.y + rad * 0.7), r.max - egui::vec2(1.0, 1.0)], egui::Stroke::new(2.0, egui::Color32::from_gray(235)));
}
fn draw_nav_zoom(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke();
    let c = egui::pos2(r.center().x - r.width() * 0.08, r.center().y - r.height() * 0.08);
    let rad = r.width() * 0.3;
    p.circle_stroke(c, rad, st);
    // signe + dans la loupe
    let m = rad * 0.55;
    p.line_segment([egui::pos2(c.x - m, c.y), egui::pos2(c.x + m, c.y)], st);
    p.line_segment([egui::pos2(c.x, c.y - m), egui::pos2(c.x, c.y + m)], st);
    // manche
    p.line_segment([egui::pos2(c.x + rad * 0.7, c.y + rad * 0.7), r.max - egui::vec2(2.0, 2.0)], egui::Stroke::new(2.4, egui::Color32::from_gray(235)));
}
fn draw_nav_move(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke(); let c = r.center(); let m = r.width() * 0.42;
    p.line_segment([egui::pos2(c.x, c.y - m), egui::pos2(c.x, c.y + m)], st);
    p.line_segment([egui::pos2(c.x - m, c.y), egui::pos2(c.x + m, c.y)], st);
    draw_arrowhead(p, egui::pos2(c.x, c.y - m), egui::vec2(0.0, -1.0), st);
    draw_arrowhead(p, egui::pos2(c.x, c.y + m), egui::vec2(0.0, 1.0), st);
    draw_arrowhead(p, egui::pos2(c.x - m, c.y), egui::vec2(-1.0, 0.0), st);
    draw_arrowhead(p, egui::pos2(c.x + m, c.y), egui::vec2(1.0, 0.0), st);
}
fn draw_nav_rotate(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke(); let c = r.center(); let rad = r.width() * 0.34;
    // anneau d'orbite (ellipse) pour l'effet 3D
    p.add(egui::Shape::ellipse_stroke(c, egui::vec2(rad, rad * 0.42), egui::Stroke::new(1.2, egui::Color32::from_gray(190))));
    // flèche circulaire = rotation
    draw_arc(p, c, rad, -2.5, 1.7, st);
    let a = 1.7f32; let tip = egui::pos2(c.x + rad * a.cos(), c.y + rad * a.sin());
    draw_arrowhead(p, tip, egui::vec2(-a.sin(), a.cos()), st);
}
fn draw_nav_center(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke(); let c = r.center(); let m = r.width() * 0.42;
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let tip = egui::pos2(c.x + sx * m, c.y + sy * m);
        let base = egui::pos2(c.x + sx * m * 0.4, c.y + sy * m * 0.4);
        p.line_segment([base, tip], st);
        draw_arrowhead(p, tip, egui::vec2(sx, sy), st);
    }
}
fn draw_ruler_big(p: &egui::Painter, r: egui::Rect) {
    let st = ic_stroke();
    let rr = egui::Rect::from_min_max(egui::pos2(r.left(), r.center().y - r.height() * 0.22), egui::pos2(r.right(), r.center().y + r.height() * 0.22));
    p.rect_stroke(rr, 1.0, st);
    for k in 1..5 { let x = rr.left() + rr.width() * k as f32 / 5.0; let h = if k % 2 == 0 { rr.height() * 0.6 } else { rr.height() * 0.35 }; p.line_segment([egui::pos2(x, rr.top()), egui::pos2(x, rr.top() + h)], st); }
}
// ---- Icônes des outils (façon MoI : trait sombre sur fond clair) ----
fn tstroke() -> egui::Stroke { egui::Stroke::new(1.7, egui::Color32::from_gray(55)) }
fn ti_line(p: &egui::Painter, r: egui::Rect) {
    p.line_segment([egui::pos2(r.left()+2.0, r.bottom()-2.0), egui::pos2(r.right()-2.0, r.top()+2.0)], tstroke());
}
fn ti_free(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let n = 22; let mut prev = None;
    for i in 0..=n { let t = i as f32 / n as f32;
        let x = r.left()+2.0 + (r.width()-4.0)*t;
        let y = r.center().y - (r.height()*0.33)*((t*std::f32::consts::TAU*1.1).sin());
        let pt = egui::pos2(x, y); if let Some(pp)=prev { p.line_segment([pp, pt], st); } prev=Some(pt); }
}
fn ti_rect(p: &egui::Painter, r: egui::Rect) { p.rect_stroke(r.shrink(2.5), 0.0, tstroke()); }
fn ti_poly(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let c = r.center(); let rad = r.width()*0.42; let mut pts = Vec::new();
    for i in 0..5 { let a = -std::f32::consts::FRAC_PI_2 + i as f32*std::f32::consts::TAU/5.0; pts.push(egui::pos2(c.x+rad*a.cos(), c.y+rad*a.sin())); }
    for i in 0..5 { p.line_segment([pts[i], pts[(i+1)%5]], st); }
}
fn ti_circle(p: &egui::Painter, r: egui::Rect) { p.circle_stroke(r.center(), r.width()*0.42, tstroke()); }
fn ti_arc(p: &egui::Painter, r: egui::Rect) { draw_arc(p, egui::pos2(r.center().x, r.bottom()-3.0), r.width()*0.5, -3.0, -0.14, tstroke()); }
fn ti_ellipse(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let c = r.center(); let (rx, ry) = (r.width()*0.46, r.height()*0.30); let n = 28; let mut prev = None;
    for i in 0..=n { let a = i as f32/n as f32*std::f32::consts::TAU; let pt = egui::pos2(c.x+rx*a.cos(), c.y+ry*a.sin()); if let Some(pp)=prev { p.line_segment([pp,pt], st); } prev=Some(pt); }
}
fn ti_spiral(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let c = r.center(); let n = 60; let mut prev = None;
    for i in 0..=n { let t = i as f32/n as f32; let a = t*std::f32::consts::TAU*2.2; let rad = r.width()*0.08 + t*r.width()*0.34;
        let pt = egui::pos2(c.x+rad*a.cos(), c.y+rad*a.sin()); if let Some(pp)=prev { p.line_segment([pp,pt], st); } prev=Some(pt); }
}
fn ti_plane(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let fill = egui::Color32::from_gray(150);
    let (a,b,c,d) = (egui::pos2(r.left()+5.0, r.bottom()-6.0), egui::pos2(r.left()+r.width()*0.45, r.top()+5.0), egui::pos2(r.right()-5.0, r.top()+r.height()*0.4), egui::pos2(r.left()+r.width()*0.55, r.bottom()-3.0));
    p.add(egui::Shape::convex_polygon(vec![a,b,c,d], fill, st));
}
fn ti_box(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let w = r.width()*0.6; let h = r.height()*0.6; let o = r.width()*0.22;
    let c = r.center();
    let fl = egui::pos2(c.x-w*0.5, c.y-h*0.5+o*0.5);
    let front = egui::Rect::from_min_size(fl, egui::vec2(w, h));
    p.rect_filled(front, 0.0, egui::Color32::from_gray(175)); p.rect_stroke(front, 0.0, st);
    let tr = egui::pos2(front.right(), front.top());
    p.add(egui::Shape::convex_polygon(vec![front.left_top(), front.right_top(), egui::pos2(tr.x+o, tr.y-o), egui::pos2(front.left()+o, front.top()-o)], egui::Color32::from_gray(205), st));
    p.add(egui::Shape::convex_polygon(vec![front.right_top(), front.right_bottom(), egui::pos2(front.right()+o, front.bottom()-o), egui::pos2(tr.x+o, tr.y-o)], egui::Color32::from_gray(140), st));
}
fn ti_sphere(p: &egui::Painter, r: egui::Rect) {
    let c = r.center(); let rad = r.width()*0.42;
    p.circle_filled(c, rad, egui::Color32::from_gray(170));
    p.circle_filled(egui::pos2(c.x-rad*0.3, c.y-rad*0.3), rad*0.5, egui::Color32::from_gray(215));
    p.circle_stroke(c, rad, tstroke());
}
fn ti_cyl(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let c = r.center(); let (rx, ry) = (r.width()*0.32, r.height()*0.13); let hh = r.height()*0.3;
    let body = egui::Rect::from_center_size(c, egui::vec2(rx*2.0, hh*2.0));
    p.rect_filled(body, 0.0, egui::Color32::from_gray(175));
    p.line_segment([body.left_top(), body.left_bottom()], st); p.line_segment([body.right_top(), body.right_bottom()], st);
    let top = egui::Rect::from_center_size(egui::pos2(c.x, c.y-hh), egui::vec2(rx*2.0, ry*2.0));
    p.add(egui::Shape::ellipse_filled(top.center(), egui::vec2(rx, ry), egui::Color32::from_gray(205)));
    p.add(egui::Shape::ellipse_stroke(top.center(), egui::vec2(rx, ry), st));
    draw_arc(p, egui::pos2(c.x, c.y+hh), rx, 0.0, std::f32::consts::PI, st);
}
fn ti_cone(p: &egui::Painter, r: egui::Rect) {
    let st = tstroke(); let c = r.center(); let rx = r.width()*0.34; let ry = r.height()*0.12; let apex = egui::pos2(c.x, r.top()+4.0);
    let (bl, br) = (egui::pos2(c.x-rx, r.bottom()-6.0), egui::pos2(c.x+rx, r.bottom()-6.0));
    p.add(egui::Shape::convex_polygon(vec![apex, bl, br], egui::Color32::from_gray(175), st));
    p.add(egui::Shape::ellipse_filled(egui::pos2(c.x, r.bottom()-6.0), egui::vec2(rx, ry), egui::Color32::from_gray(150)));
    p.add(egui::Shape::ellipse_stroke(egui::pos2(c.x, r.bottom()-6.0), egui::vec2(rx, ry), st));
}
fn ti_text(p: &egui::Painter, r: egui::Rect) {
    p.text(r.center(), egui::Align2::CENTER_CENTER, "A", egui::FontId::proportional(r.height()*0.9), egui::Color32::from_gray(55));
}

/// Cellule outil façon MoI : fond clair, icône dessinée + libellé dessous. Renvoie la réponse.
fn tool_cell(ui: &mut egui::Ui, w: f32, label: &str, draw: impl Fn(&egui::Painter, egui::Rect)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 58.0), egui::Sense::click());
    let r = rect.shrink(2.0);
    let p = ui.painter();
    let bg = if resp.hovered() { egui::Color32::from_rgb(232, 238, 246) } else { egui::Color32::from_rgb(244, 246, 249) };
    p.rect_filled(r, 4.0, bg);
    p.rect_stroke(r, 4.0, egui::Stroke::new(1.0, egui::Color32::from_gray(175)));
    let ir = egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 20.0), egui::vec2(30.0, 30.0));
    draw(p, ir);
    p.text(egui::pos2(r.center().x, r.bottom() - 9.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), egui::Color32::from_gray(40));
    resp
}

fn icon_button(ui: &mut egui::Ui, label: &str, active: bool, draw: impl Fn(&egui::Painter, egui::Rect)) -> egui::Response {
    icon_button_s(ui, label, active, draw, 1.0)
}
/// Bouton à icône, mis à l'échelle `s` (barre du bas à géométrie variable).
fn icon_button_s(ui: &mut egui::Ui, label: &str, active: bool, draw: impl Fn(&egui::Painter, egui::Rect), s: f32) -> egui::Response {
    icon_button_w(ui, label, active, draw, s, 56.0)
}
/// Même bouton, largeur `w` (avant échelle) : pour les libellés longs.
fn icon_button_w(ui: &mut egui::Ui, label: &str, active: bool, draw: impl Fn(&egui::Painter, egui::Rect), s: f32, w: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w * s, 50.0 * s), egui::Sense::click());
    let r = rect.shrink(2.0);
    let p = ui.painter();
    let bg = if active { egui::Color32::from_rgb(58, 110, 150) }
             else if resp.hovered() { egui::Color32::from_gray(82) }
             else { egui::Color32::from_gray(60) };
    p.rect_filled(r, 6.0, bg);
    p.rect_stroke(r, 6.0, egui::Stroke::new(1.0, egui::Color32::from_gray(if active { 150 } else { 95 })));
    let ir = egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 16.0 * s), egui::vec2(24.0 * s, 24.0 * s));
    draw(p, ir);
    p.text(egui::pos2(r.center().x, r.bottom() - 8.0 * s), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional((10.5 * s).max(7.0)), egui::Color32::from_gray(225));
    resp
}

/// Bouton Règles : même taille que icon_button, logo réduit + unité courante en gras dessous.
fn ruler_button(ui: &mut egui::Ui, unit: &str, active: bool, s: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(56.0 * s, 50.0 * s), egui::Sense::click());
    let r = rect.shrink(2.0);
    let p = ui.painter();
    let bg = if active { egui::Color32::from_rgb(58, 110, 150) }
             else if resp.hovered() { egui::Color32::from_gray(82) }
             else { egui::Color32::from_gray(60) };
    p.rect_filled(r, 6.0, bg);
    p.rect_stroke(r, 6.0, egui::Stroke::new(1.0, egui::Color32::from_gray(if active { 150 } else { 95 })));
    // logo règle agrandi en haut, unité courante (texte normal) en dessous (plus de libellé)
    let ir = egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 14.0 * s), egui::vec2(36.0 * s, 24.0 * s));
    draw_ruler_big(p, ir);
    let font = egui::FontId::proportional((13.0 * s).max(9.0));
    p.text(egui::pos2(r.center().x, r.bottom() - 11.0 * s), egui::Align2::CENTER_CENTER, unit, font, egui::Color32::WHITE);
    resp
}

/// Bouton texte façon menu déroulant. Chevron « ^ » = le menu se déroule vers le HAUT.
fn menu_button(ui: &mut egui::Ui, text: &str, open: bool, w: f32, s: f32) -> egui::Response {
    let orange = egui::Color32::from_rgb(232, 168, 66);
    let gbg = egui::Color32::from_gray(64);
    ui.add_sized(egui::vec2(w, 50.0 * s), egui::Button::new(egui::RichText::new(format!("{}  ^", text.to_uppercase())).size(13.0 * s.max(0.6))
            .color(if open { egui::Color32::BLACK } else { egui::Color32::from_gray(235) }).strong()).wrap(false)
        .fill(if open { orange } else { gbg })
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110))))
}
/// Fenêtre d'un menu de la barre (Taille, Snap Grille) : s'ouvre DANS la bande sous les vues
/// (jamais par-dessus l'affichage), calée à gauche de son bouton. Se ferme au clic ailleurs.
fn bar_menu(ctx: &egui::Context, id: egui::Id, btn: &egui::Response, bar_top: f32, add: impl FnOnce(&mut egui::Ui)) {
    bar_menu_at(ctx, id, btn, bar_top, true, 236.0, add)
}
/// `left` = fenêtre à gauche du bouton, sinon à sa droite.
fn bar_menu_at(ctx: &egui::Context, id: egui::Id, btn: &egui::Response, bar_top: f32, left: bool, w: f32, add: impl FnOnce(&mut egui::Ui)) {
    if !ctx.memory(|m| m.is_popup_open(id)) { return; }
    let x = if left { (btn.rect.left() - w - 8.0).max(4.0) } else { btn.rect.right() + 8.0 };
    // appuyée sur le bas de la fenêtre ; si elle est plus haute que la barre, elle déborde vers le haut
    let _ = bar_top;
    let pos = egui::pos2(x, ctx.screen_rect().bottom() - 4.0);
    let r = egui::Area::new(id.with("area")).order(egui::Order::Foreground).fixed_pos(pos).pivot(egui::Align2::LEFT_BOTTOM).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| { ui.set_width(w - 16.0); add(ui); });
    });
    if r.response.clicked_elsewhere() && !btn.clicked() && !btn.secondary_clicked() { ctx.memory_mut(|m| m.close_popup()); }
}
/// Menu « Enregistrer sous » : avertissements (sélection / scène) puis grille des formats ; renvoie le
/// format choisi. `n_sel` = objets sélectionnés (0 = aucun : toute la scène part), `n_tot` = objets de la scène.
fn menu_sauver(ui: &mut egui::Ui, lang: Lang, n_sel: usize, n_tot: usize) -> Option<&'static str> {
    let mut choix = None;
    yellow_title(ui, tr(lang, "enreg_sous"));
    if n_sel >= 2 || (n_sel == 0 && n_tot >= 2) {
        ui.label(egui::RichText::new(tr(lang, "sauve_3mf")).size(11.5).color(egui::Color32::from_rgb(150, 200, 255)));
    }
    if n_sel > 0 && n_tot > n_sel {
        // plusieurs objets sur le plateau (ex. après une découpe) : avertissement bien visible
        egui::Frame::none().fill(egui::Color32::from_rgb(150, 30, 25)).rounding(egui::Rounding::same(5.0)).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(format!("⚠ {}", tr(lang, if n_sel == 1 { "sauve_avert1" } else { "sauve_avert" }).replace("{n}", &n_tot.to_string()).replace("{s}", &n_sel.to_string())))
                .size(12.0).strong().color(egui::Color32::WHITE)).wrap(true));
        });
    } else {
        let t = if n_sel > 0 { format!("{} : {n_sel}", tr(lang, "sauve_sel")) } else { tr(lang, "sauve_tout").to_string() };
        ui.label(egui::RichText::new(t).size(11.5).color(egui::Color32::from_rgb(250, 200, 90)));
    }
    for rangee in FORMATS_SAUVE.chunks(4) {
        ui.horizontal(|ui| {
            for &(ext, nom) in rangee {
                if ui.add_sized(egui::vec2(73.0, 26.0), egui::Button::new(egui::RichText::new(nom).size(11.5))).on_hover_text(format!(".{ext}")).clicked() {
                    choix = Some(ext); ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    }
    choix
}
/// Menu de la barre ouvert JUSTE AU-DESSUS de son bouton (aligné à gauche du bouton).
/// Renvoie le rectangle du menu (pour savoir si la souris est dessus).
fn bar_menu_above(ctx: &egui::Context, id: egui::Id, btn: &egui::Response, w: f32, add: impl FnOnce(&mut egui::Ui)) -> Option<egui::Rect> {
    if !ctx.memory(|m| m.is_popup_open(id)) { return None; }
    let x = btn.rect.left().min(ctx.screen_rect().right() - w - 4.0).max(4.0);
    let r = egui::Area::new(id.with("area")).order(egui::Order::Foreground).fixed_pos(egui::pos2(x, btn.rect.top() - 6.0)).pivot(egui::Align2::LEFT_BOTTOM).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| { ui.set_width(w - 16.0); add(ui); });
    });
    if r.response.clicked_elsewhere() && !btn.clicked() && !btn.secondary_clicked() { ctx.memory_mut(|m| m.close_popup()); }
    Some(r.response.rect)
}
/// Barre pleine (boîte fine) entre deux points du plan Z=0 : maillage watertight orienté
/// selon le segment p0->p1. `w` = largeur, `t` = épaisseur (en Z).
fn bar_mesh(p0: Vec3, p1: Vec3, w: f32, t: f32) -> Vec<f32> {
    let dir = (p1 - p0); let len = dir.length();
    if len < 1e-4 { return Vec::new(); }
    let ax = dir / len;                       // le long de la barre
    let ay = Vec3::new(-ax.y, ax.x, 0.0).normalize_or_zero(); // largeur (dans le plan)
    let az = Vec3::Z;                         // épaisseur
    let (hw, ht) = (w * 0.5, t * 0.5);
    // 8 coins
    let c = |u: f32, v: f32, s: f32| p0 + ax * u + ay * v + az * s;
    let v000 = c(0.0, -hw, -ht); let v100 = c(len, -hw, -ht); let v110 = c(len, hw, -ht); let v010 = c(0.0, hw, -ht);
    let v001 = c(0.0, -hw, ht);  let v101 = c(len, -hw, ht);  let v111 = c(len, hw, ht);  let v011 = c(0.0, hw, ht);
    let mut out = Vec::new();
    let mut quad = |a: Vec3, b: Vec3, d: Vec3, e: Vec3| {
        let n = (b - a).cross(d - a).normalize_or_zero();
        for p in [a, b, d, a, d, e] { out.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]); }
    };
    quad(v000, v010, v110, v100); // bas
    quad(v001, v101, v111, v011); // haut
    quad(v000, v100, v101, v001); // -y
    quad(v010, v011, v111, v110); // +y
    quad(v000, v001, v011, v010); // début
    quad(v100, v110, v111, v101); // fin
    out
}

/// Nettoie une primitive OCCT (retire les triangles « sliver » aux pôles/apex) pour
/// qu'elle soit irréprochable : 0 trou / 0 erreur de face dès la création.
fn clean_prim(m: &mut stl::Mesh) {
    m.verts = defects::repair(&m.verts);
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for c in m.verts.chunks_exact(6) { let p = Vec3::new(c[0], c[1], c[2]); lo = lo.min(p); hi = hi.max(p); }
    if m.verts.is_empty() { lo = Vec3::ZERO; hi = Vec3::ZERO; }
    m.min = lo; m.max = hi;
}

/// Rotation `m3` autour de `c` cuite dans les sommets (positions + normales) ; l'objet garde
/// son point le plus bas à la même hauteur (posé sur le plateau => reste posé).
fn rotate_obj(o: &mut Obj, c: Vec3, m3: glam::Mat3) {
    let z0 = o.min.z;
    for ch in o.verts.chunks_exact_mut(6) {
        let p = m3 * (Vec3::new(ch[0], ch[1], ch[2]) - c) + c;
        let n = m3 * Vec3::new(ch[3], ch[4], ch[5]);
        ch[0] = p.x; ch[1] = p.y; ch[2] = p.z; ch[3] = n.x; ch[4] = n.y; ch[5] = n.z;
    }
    o.refresh();
    let dz = z0 - o.min.z;
    if dz.abs() > 1e-6 { o.translate(Vec3::new(0.0, 0.0, dz)); }
}

/// Animation GRAVITÉ : l'objet se redresse vers sa face stable en s'élevant un peu, puis tombe
/// lentement sur le plan XY et rebondit faiblement avant de s'immobiliser (z min = 0, X/Y inchangés).
struct GravAnim { obj: usize, depart: Vec<f32>, rot: glam::Quat, c: Vec3, z0: f32, haut: f32, t0: std::time::Instant,
                  // GRAVITÉ : trajectoire de la chute simulée (rotation, position du centre de gravité) à 60 i/s
                  traj: Option<Vec<(glam::Quat, Vec3)>> }
const GRAV_MONTEE: f32 = 0.45; // s : l'objet monte à sa hauteur de lâcher avant la chute simulée
const GRAV_REDRESSE: f32 = 0.8;   // s : bascule vers la face stable + montée
const GRAV_CHUTE: f32 = 0.93;     // s : durée de la première chute (lente, bien visible)
const GRAV_REBOND: f32 = 0.3;     // coefficient de restitution (rebond faible)
const GRAV_NB_REBONDS: i32 = 2;
impl GravAnim {
    fn new(obj: usize, o: &Obj) -> Result<GravAnim, String> {
        // vraie chute physique (rebonds, roulis, bascules) depuis l'orientation actuelle
        let taille = (o.max - o.min).length().max(1e-3);
        let haut = o.min.z.max(taille * 0.4);
        if let Some(ch) = gravite::simuler(&o.verts, haut) {
            return Ok(GravAnim { obj, depart: o.verts.clone(), rot: glam::Quat::IDENTITY, c: ch.cg0, z0: o.min.z, haut, t0: std::time::Instant::now(), traj: Some(ch.images) });
        }
        let (m, cg) = gravite::repos(&o.verts).ok_or("objet sans volume : impossible de le poser")?;
        let m3 = glam::Mat3::from_cols(m.x_axis.as_vec3(), m.y_axis.as_vec3(), m.z_axis.as_vec3());
        let rot = glam::Quat::from_mat3(&m3).normalize();
        let taille = (o.max - o.min).length().max(1e-3);
        // hauteur de lâcher : au moins ~40 % de la taille de l'objet, pour que la chute se voie
        let haut = o.min.z.max(taille * 0.4);
        Ok(GravAnim { obj, depart: o.verts.clone(), rot, c: cg.as_vec3(), z0: o.min.z, haut, t0: std::time::Instant::now(), traj: None })
    }
    /// Même animation, mais posée sur une face choisie (normale extérieure `n`, bouton SURFACE).
    fn vers(obj: usize, o: &Obj, n: Vec3) -> GravAnim {
        let rot = glam::Quat::from_rotation_arc(n.normalize(), Vec3::NEG_Z);
        let cg = gravite::centre_gravite(&o.verts).as_vec3();
        let taille = (o.max - o.min).length().max(1e-3);
        let haut = o.min.z.max(taille * 0.4);
        GravAnim { obj, depart: o.verts.clone(), rot, c: cg, z0: o.min.z, haut, t0: std::time::Instant::now(), traj: None }
    }
    fn duree() -> f32 {
        let mut d = GRAV_REDRESSE + GRAV_CHUTE;
        for k in 1..=GRAV_NB_REBONDS { d += 2.0 * GRAV_CHUTE * GRAV_REBOND.powi(k); }
        d
    }
    /// (fraction de rotation, hauteur du point le plus bas) au temps t
    fn etat(&self, t: f32) -> (f32, f32) {
        if t < GRAV_REDRESSE {
            let u = t / GRAV_REDRESSE; let s = u * u * (3.0 - 2.0 * u);
            return (s, self.z0 + (self.haut - self.z0) * s);
        }
        let g = 2.0 * self.haut / (GRAV_CHUTE * GRAV_CHUTE);
        let mut t = t - GRAV_REDRESSE;
        if t < GRAV_CHUTE { return (1.0, self.haut - 0.5 * g * t * t); }
        t -= GRAV_CHUTE;
        let mut v = g * GRAV_CHUTE;
        for _ in 0..GRAV_NB_REBONDS {
            v *= GRAV_REBOND;
            let d = 2.0 * v / g;
            if t < d { return (1.0, (v * t - 0.5 * g * t * t).max(0.0)); }
            t -= d;
        }
        (1.0, 0.0)
    }
    /// Applique l'image au temps courant ; renvoie true quand l'objet est posé (fin).
    fn appliquer(&self, o: &mut Obj) -> bool {
        let t = self.t0.elapsed().as_secs_f32();
        if let Some(tr) = &self.traj {
            // montée douce jusqu'au point de lâcher, puis la trajectoire simulée à vitesse réelle
            let (q, x, fin) = if t < GRAV_MONTEE {
                let u = t / GRAV_MONTEE; let s = u * u * (3.0 - 2.0 * u);
                (glam::Quat::IDENTITY, self.c.lerp(tr[0].1, s), false)
            } else {
                let i = ((t - GRAV_MONTEE) * 60.0) as usize;
                let fin = i + 1 >= tr.len();
                let (q, x) = tr[i.min(tr.len() - 1)];
                (q, x, fin)
            };
            o.verts.clear();
            o.verts.reserve(self.depart.len());
            let mut zmin = f32::INFINITY;
            for ch in self.depart.chunks_exact(6) {
                let p = q * (Vec3::new(ch[0], ch[1], ch[2]) - self.c) + x;
                let n = q * Vec3::new(ch[3], ch[4], ch[5]);
                zmin = zmin.min(p.z);
                o.verts.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
            }
            // jamais sous le plateau ; posé exactement à z = 0 à la fin
            let dz = if fin || zmin < 0.0 { -zmin } else { 0.0 };
            if dz != 0.0 { for ch in o.verts.chunks_exact_mut(6) { ch[2] += dz; } }
            o.refresh();
            return fin;
        }
        let fin = t >= Self::duree();
        let (s, zb) = if fin { (1.0, 0.0) } else { self.etat(t) };
        let q = glam::Quat::IDENTITY.slerp(self.rot, s);
        o.verts.clear();
        o.verts.reserve(self.depart.len());
        let mut zmin = f32::INFINITY;
        for ch in self.depart.chunks_exact(6) {
            let p = q * (Vec3::new(ch[0], ch[1], ch[2]) - self.c) + self.c;
            let n = q * Vec3::new(ch[3], ch[4], ch[5]);
            zmin = zmin.min(p.z);
            o.verts.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
        }
        let dz = zb - zmin;
        for ch in o.verts.chunks_exact_mut(6) { ch[2] += dz; }
        o.refresh();
        fin
    }
}

/// Logo SORTIR : porte entrouverte et flèche qui en sort.
fn draw_sortir(p: &egui::Painter, r: egui::Rect, col: egui::Color32) {
    let (w, h) = (r.width(), r.height());
    let pt = |x: f32, y: f32| egui::pos2(r.left() + w * x, r.top() + h * y);
    let st = egui::Stroke::new(2.2, col);
    // cadre de porte (ouvert à droite)
    p.line_segment([pt(0.55, 0.08), pt(0.10, 0.08)], st);
    p.line_segment([pt(0.10, 0.08), pt(0.10, 0.92)], st);
    p.line_segment([pt(0.10, 0.92), pt(0.55, 0.92)], st);
    p.line_segment([pt(0.55, 0.08), pt(0.55, 0.30)], st);
    p.line_segment([pt(0.55, 0.70), pt(0.55, 0.92)], st);
    // flèche vers la droite
    p.line_segment([pt(0.30, 0.50), pt(0.80, 0.50)], egui::Stroke::new(3.0, col));
    p.add(egui::Shape::convex_polygon(vec![pt(0.98, 0.50), pt(0.74, 0.30), pt(0.74, 0.70)], col, egui::Stroke::NONE));
}

/// Logo SURFACE : deux surfaces parallèles en perspective, flèche vers le bas entre les deux.
/// Aire en mm² avec séparateur des milliers (2 173,4).
fn fmt_aire(a: f32) -> String {
    let e = a.round() as i64;
    let s = e.to_string();
    let mut o = String::new();
    for (k, c) in s.chars().enumerate() { if k > 0 && (s.len() - k) % 3 == 0 { o.push(' '); } o.push(c); }
    if a < 100.0 { format!("{a:.1}") } else { o }
}

fn draw_surface(p: &egui::Painter, r: egui::Rect, col: egui::Color32) {
    let st = egui::Stroke::new(2.0, col);
    let plaque = |yh: f32, yb: f32| vec![egui::pos2(r.left() + r.width() * 0.22, yh), egui::pos2(r.right() - r.width() * 0.02, yh),
                                          egui::pos2(r.right() - r.width() * 0.22, yb), egui::pos2(r.left() + r.width() * 0.02, yb)];
    let h = r.height();
    p.add(egui::Shape::convex_polygon(plaque(r.top() + h * 0.02, r.top() + h * 0.24), col.gamma_multiply(0.30), st));
    p.add(egui::Shape::convex_polygon(plaque(r.bottom() - h * 0.24, r.bottom() - h * 0.02), col.gamma_multiply(0.30), st));
    let x = r.center().x;
    let (haut, pointe) = (r.top() + h * 0.30, r.bottom() - h * 0.30);
    p.line_segment([egui::pos2(x, haut), egui::pos2(x, pointe - h * 0.12)], egui::Stroke::new(3.0, col));
    let w = r.width() * 0.17;
    p.add(egui::Shape::convex_polygon(vec![egui::pos2(x - w, pointe - h * 0.15), egui::pos2(x + w, pointe - h * 0.15), egui::pos2(x, pointe)], col, egui::Stroke::NONE));
}

/// Logo GRAVITÉ : flèche vers le bas posée sur un plan horizontal (vu en perspective).
fn draw_gravite(p: &egui::Painter, r: egui::Rect, col: egui::Color32) {
    let st = egui::Stroke::new(2.2, col);
    // plan horizontal (parallélogramme) en bas
    let (y0, y1) = (r.bottom() - r.height() * 0.30, r.bottom() - r.height() * 0.06);
    let plan = vec![egui::pos2(r.left() + r.width() * 0.22, y0), egui::pos2(r.right() - r.width() * 0.02, y0),
                    egui::pos2(r.right() - r.width() * 0.22, y1), egui::pos2(r.left() + r.width() * 0.02, y1)];
    p.add(egui::Shape::convex_polygon(plan, col.gamma_multiply(0.35), st));
    // flèche
    let x = r.center().x;
    let (top, tip) = (r.top() + r.height() * 0.04, y0 + (y1 - y0) * 0.45);
    p.line_segment([egui::pos2(x, top), egui::pos2(x, tip - r.height() * 0.16)], egui::Stroke::new(3.2, col));
    let w = r.width() * 0.20;
    p.add(egui::Shape::convex_polygon(vec![egui::pos2(x - w, tip - r.height() * 0.22), egui::pos2(x + w, tip - r.height() * 0.22), egui::pos2(x, tip)], col, egui::Stroke::NONE));
}

fn place_at_origin(m: &mut stl::Mesh) {
    let off = m.min;
    for c in m.verts.chunks_exact_mut(6) { c[0] -= off.x; c[1] -= off.y; c[2] -= off.z; }
    m.max -= off; m.min = Vec3::ZERO;
}

/// Affichage grille / axes (icône en bas à gauche des vues) : 0 grille+axes, 1 grille seule,
/// 2 axes seuls, 3 rien. Global (lu par draw_view, y compris pour les photos).
static GRILLE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
fn grille_mode() -> u8 { GRILLE_MODE.load(std::sync::atomic::Ordering::Relaxed) }
fn show_axes() -> bool { matches!(grille_mode(), 0 | 2) }
/// Mode visionneuse (sans les outils) : la grille reste, les axes X / Y / Z ne sont pas tracés.
static EXPORT_TEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false); // clic simulé (PATCH3D_OPEN=export) déjà fait
static VISIONNEUSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn axes_visibles() -> bool { show_axes() && !VISIONNEUSE.load(std::sync::atomic::Ordering::Relaxed) }

/// Logo « grille 3D » : plan quadrillé en perspective + axes X rouge, Y vert, Z bleu.
/// `grille` / `axes` : parties allumées (les autres en gris atténué).
fn draw_grille3d(p: &egui::Painter, r: egui::Rect, grille: bool, axes: bool) {
    let c = r.center() + egui::vec2(0.0, r.height() * 0.12);
    let (ux, uy) = (egui::vec2(r.width() * 0.42, r.height() * 0.13), egui::vec2(-r.width() * 0.30, r.height() * 0.20));
    let pt = |a: f32, b: f32| c + ux * a + uy * b;
    let gc = if grille { egui::Color32::from_gray(225) } else { egui::Color32::from_gray(85) };
    for i in 0..=4 {
        let t = -1.0 + i as f32 * 0.5;
        p.line_segment([pt(-1.0, t), pt(1.0, t)], egui::Stroke::new(1.0, gc));
        p.line_segment([pt(t, -1.0), pt(t, 1.0)], egui::Stroke::new(1.0, gc));
    }
    let (rx, gx, bx) = if axes { (egui::Color32::from_rgb(255, 80, 80), egui::Color32::from_rgb(90, 230, 90), egui::Color32::from_rgb(110, 170, 255)) }
                       else { (egui::Color32::from_gray(90), egui::Color32::from_gray(90), egui::Color32::from_gray(90)) };
    let o = pt(-1.0, 1.0);
    p.line_segment([o, pt(1.25, 1.0)], egui::Stroke::new(2.2, rx));
    p.line_segment([o, pt(-1.0, -1.3)], egui::Stroke::new(2.2, gx));
    p.line_segment([o, o + egui::vec2(0.0, -r.height() * 0.62)], egui::Stroke::new(2.2, bx));
}

/// Direction de la lumière de la scène (projecteur), partagée par tous les rendus (écran, photo).
static LUMIERE: std::sync::Mutex<Option<Vec3>> = std::sync::Mutex::new(None);
fn lumiere_courante() -> Vec3 {
    let mut g = LUMIERE.lock().unwrap();
    *g.get_or_insert_with(lumiere::defaut)
}
fn lumiere_regler(d: Vec3) { *LUMIERE.lock().unwrap() = Some(d); }

#[allow(clippy::too_many_arguments)]
/// Objets A et B du panneau BOOLÉENS, teintés dans la vue (vide hors du panneau et pour les photos).
/// Résultat d'une découpe : (objet, morceau côté flèche, autre morceau).
static COUPE_RES: std::sync::Mutex<Vec<(usize, Vec<f32>, Vec<f32>)>> = std::sync::Mutex::new(Vec::new());
/// Opération du panneau FORME, appliquée à la sélection (objet seul ou groupe).
#[derive(Clone, Copy)]
enum FormeOp { Pos(Vec3), Poser, Rot(usize, f32), Miroir(usize), Echelle(Vec3), Aligner(usize, u8), Repartir(usize), Lineaire(u32, f32, usize), Circulaire(u32, f32) }

/// Sélection multiple (Ctrl+clic, comme 3D Builder) : objets surlignés en plus de `sel`.
static SEL_PLUS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
/// Aperçu TEXTURES : (objet remplacé à l'affichage, VAO de l'aperçu, nombre de sommets).
static APERCU_TX: std::sync::Mutex<Option<(usize, glow::NativeVertexArray, i32, bool)>> = std::sync::Mutex::new(None);
/// Pièce surlignée d'un objet fait de plusieurs pièces : (objet, VAO de la pièce, nombre de sommets).
static PIECE_SEL: std::sync::Mutex<Option<(usize, glow::NativeVertexArray, i32)>> = std::sync::Mutex::new(None);

/// Pièces séparées d'un objet (triangles reliés par des sommets communs) : pièce de chaque triangle, nombre de pièces.
fn pieces(verts: &[f32]) -> (Vec<u32>, usize) {
    let nt = verts.len() / 18;
    let mut id: fasthash::HashMap<(u32, u32, u32), u32> = fasthash::HashMap::default();
    let mut parent: Vec<u32> = Vec::new();
    fn racine(p: &mut [u32], mut x: u32) -> u32 { while p[x as usize] != x { p[x as usize] = p[p[x as usize] as usize]; x = p[x as usize]; } x }
    let mut tri_v = Vec::with_capacity(nt);
    for t in 0..nt {
        let mut v = [0u32; 3];
        for k in 0..3 {
            let o = t * 18 + k * 6;
            let key = (verts[o].to_bits(), verts[o + 1].to_bits(), verts[o + 2].to_bits());
            let n = parent.len() as u32;
            v[k] = *id.entry(key).or_insert_with(|| { parent.push(n); n });
        }
        for k in 1..3 { let (a, b) = (racine(&mut parent, v[0]), racine(&mut parent, v[k])); if a != b { parent[b as usize] = a; } }
        tri_v.push(v[0]);
    }
    let mut num: fasthash::HashMap<u32, u32> = fasthash::HashMap::default();
    let comp: Vec<u32> = tri_v.iter().map(|&v| { let r = racine(&mut parent, v); let n = num.len() as u32; *num.entry(r).or_insert(n) }).collect();
    let n = num.len();
    (comp, n)
}

/// Triangle d'un objet touché par le rayon du curseur (le plus proche).
fn tri_vise(mvp: &Mat4, cursor: (f64, f64), rect: (f32, f32, f32, f32), o: &Obj) -> Option<usize> {
    let (orig, dir) = ray_from(mvp, cursor, rect);
    let mut best: Option<(usize, f32)> = None;
    for (k, t) in o.tris.iter().enumerate() { if let Some(d) = ray_tri(orig, dir, t) { if best.map_or(true, |b| d < b.1) { best = Some((k, d)); } } }
    best.map(|b| b.0)
}

/// Sommets (et couleurs) des triangles d'un objet dont la pièce vaut (`garder` = true) ou ne vaut pas `c`.
fn tris_piece(o: &Obj, comp: &[u32], c: u32, garder: bool) -> (Vec<f32>, Vec<f32>) {
    let cols = o.has_colors();
    let (mut v, mut col) = (Vec::new(), Vec::new());
    for (t, &k) in comp.iter().enumerate() {
        if (k == c) != garder { continue; }
        v.extend_from_slice(&o.verts[t * 18..t * 18 + 18]);
        if cols { col.extend_from_slice(&o.cols[t * 9..t * 9 + 9]); }
    }
    (v, col)
}
unsafe fn draw_view(gl: &glow::Context, program: glow::NativeProgram, scene: &[Obj], sel: Option<usize>,
                    u: &Uniforms, grid: &grid::Grid, rule: &ruler::Ruler, mvp: &Mat4, eye: Vec3,
                    show_ruler: bool, show_grid: bool, reflect: f32, dim: f32, gray: f32, tex_highlight: bool, alpha: f32, xf: Option<(usize, Mat4)>) {
    let a = mvp.to_cols_array();
    gl.use_program(Some(program));
    gl.uniform_matrix_4_f32_slice(u.mvp.as_ref(), false, &a);
    gl.uniform_matrix_4_f32_slice(u.model.as_ref(), false, &Mat4::IDENTITY.to_cols_array());
    gl.uniform_3_f32(u.eye.as_ref(), eye.x, eye.y, eye.z);
    let l = lumiere_courante();
    gl.uniform_3_f32(u.light.as_ref(), l.x, l.y, l.z);
    gl.uniform_1_f32(u.reflect.as_ref(), reflect);
    gl.uniform_1_f32(u.dim.as_ref(), dim);
    gl.uniform_1_f32(u.gray.as_ref(), gray);
    gl.uniform_1_f32(u.alpha.as_ref(), alpha);
    let piece_sel = *PIECE_SEL.lock().unwrap();
    for (i, o) in scene.iter().enumerate() {
        if o.nverts == 0 { continue; }
        // pièce choisie : seule elle est surlignée (l'objet, lui, s'affiche comme sélectionné sans orange)
        let piece_ici = piece_sel.filter(|p| p.0 == i);
        let choisi = sel == Some(i) || (sel.is_some() && SEL_PLUS.lock().unwrap().contains(&i));
        if tex_highlight {
            // Mode Texturé : l'objet sélectionné est en couleur, les autres restent sombres.
            let dark = sel.is_some() && !choisi;
            gl.uniform_1_f32(u.gray.as_ref(), if dark { 1.0 } else { 0.0 });
            gl.uniform_1_f32(u.dim.as_ref(), if dark { 0.38 } else { 1.0 });
            gl.uniform_1_f32(u.sel.as_ref(), 0.0);
        } else {
            gl.uniform_1_f32(u.sel.as_ref(), if choisi && piece_ici.is_none() { 1.0 } else { 0.0 });
        }
        gl.uniform_1_f32(u.usecol.as_ref(), if o.has_colors() { 1.0 } else { 0.0 });
        gl.uniform_4_f32(u.tint.as_ref(), 0.0, 0.0, 0.0, 0.0);
        // aperçu de rotation (outil de rotation) : matrice de modèle sur l'objet concerné seulement
        let m = xf.filter(|x| x.0 == i).map_or(Mat4::IDENTITY, |x| x.1);
        gl.uniform_matrix_4_f32_slice(u.model.as_ref(), false, &m.to_cols_array());
        match *APERCU_TX.lock().unwrap() {
            Some((k, vao, n, cols)) if k == i => {
                // couleurs du mode Texture (masques) avec l'éclairage normal : uUseCol = 2
                gl.uniform_1_f32(u.usecol.as_ref(), if cols { 2.0 } else { 0.0 });
                gl.uniform_1_f32(u.sel.as_ref(), 0.0);
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, n);
            }
            _ => { gl.bind_vertex_array(Some(o.vao)); gl.draw_arrays(glow::TRIANGLES, 0, o.nverts); }
        }
        if let Some((_, vao, n)) = piece_ici {
            // la pièce choisie, en orange de sélection, juste devant l'objet
            gl.uniform_1_f32(u.sel.as_ref(), 1.0); gl.uniform_1_f32(u.usecol.as_ref(), 0.0);
            gl.uniform_1_f32(u.gray.as_ref(), 0.0); gl.uniform_1_f32(u.dim.as_ref(), 1.0);
            gl.enable(glow::POLYGON_OFFSET_FILL); gl.polygon_offset(-1.0, -2.0);
            gl.bind_vertex_array(Some(vao));
            gl.draw_arrays(glow::TRIANGLES, 0, n);
            gl.disable(glow::POLYGON_OFFSET_FILL);
        }
    }
    gl.uniform_matrix_4_f32_slice(u.model.as_ref(), false, &Mat4::IDENTITY.to_cols_array());
    grid.draw(gl, &a, show_grid, axes_visibles());
    if show_ruler { rule.draw(gl, &a); }
}
fn view_mvp(k: ViewKind, orbit: &Orbit, ortho: &[Ortho; 3], base: Vec3, aspect: f32) -> (Mat4, Vec3) {
    if k == ViewKind::ThreeD { orbit.mvp(aspect) } else { ortho[view::oidx(k)].mvp(k, base, aspect) }
}

/// Pas « rond » (1 / 2 / 5 × 10ⁿ) juste au-dessus de la cible, en mm.
fn nice_step(target: f32) -> f32 {
    if !(target > 0.0) { return 10.0; }
    let p = 10f32.powf(target.log10().floor());
    for m in [1.0, 2.0, 5.0] { if m * p >= target { return m * p; } }
    10.0 * p
}

/// Recalcule les défauts de l'objet actif (ou scène vide) et met à jour les traits rouge/jaune.
unsafe fn recompute_defects(gl: &glow::Context, scene: &[Obj], sel: Option<usize>,
                            bnd: &mut defects::LineSet, err: &mut defects::LineSet) -> defects::Defects {
    let idx = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 { Some(0) } else { None });
    let d = match idx { Some(i) => defects::analyze(&scene[i].verts), None => defects::Defects::default() };
    bnd.update(gl, &d.boundary);
    err.update(gl, &d.errors);
    d
}

/// Pas, nombre de lignes de grille et de graduations de règle pour la scène : le pas suit la taille
/// des objets (demi-diagonale), l'étendue couvre aussi un objet éloigné de l'origine (grille et
/// règles partent de l'origine).
fn dims_grille(unit: Unit, scene: &[Obj], r_vide: f32) -> (f32, i32, i32) {
    let (lo, hi) = scene_bounds(scene);
    let (taille, portee) = if hi == lo { (r_vide, r_vide) }
                           else { (((hi - lo) * 0.5).length().max(1e-3), lo.abs().max(hi.abs()).max_element()) };
    let step = unit.pas_grille((taille * 0.2).max(portee / 150.0));
    let lines = (((portee + taille) * 1.25 / step).ceil() as i32).clamp(8, 400);
    let majors = (((portee + taille) / step).ceil() as i32).clamp(6, 200);
    (step, lines, majors)
}

/// (Re)construit grille + règle aux dimensions données (voir dims_grille).
unsafe fn build_grid_ruler(gl: &glow::Context, (step, lines, majors): (f32, i32, i32)) -> (grid::Grid, ruler::Ruler, i32) {
    (grid::Grid::new(gl, step, lines), ruler::Ruler::new(gl, step, majors), majors)
}

/// Ouverture / sauvegarde : sélecteur + lecture/écriture dans un fil séparé (la fenêtre
/// reste réactive ; sinon le bureau la déclare « ne répond pas »).
/// Ouverture : liste d'objets (un 3MF peut en contenir plusieurs, séparés), chacun (sommets, couleurs).
/// Opened : (fichier, objets, ajout à la scène, défauts du dernier objet déjà analysés dans le fil).
enum FileOut { Opened(std::path::PathBuf, Vec<(Vec<f32>, Option<Vec<f32>>)>, bool, Option<defects::Defects>), Saved(std::path::PathBuf), Cancelled, Failed(String) }

/// Texte à incruster dans une copie d'écran : (x, y) en pixels de l'image, ancrage
/// 0 = centré, 1 = aligné à droite (haut), 2 = aligné à gauche (haut) ; fond optionnel.
struct ShotText { x: f32, y: f32, text: String, col: [u8; 3], size: f32, anchor: u8, clip: [f32; 4] }
struct ShotBox { x0: f32, y0: f32, x1: f32, y1: f32 }

fn blend(rgb: &mut [u8], w: u32, h: u32, x: i32, y: i32, col: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 || a <= 0.0 { return; }
    let i = ((y as u32 * w + x as u32) * 3) as usize;
    let a = a.min(1.0);
    for c in 0..3 { rgb[i + c] = (rgb[i + c] as f32 * (1.0 - a) + col[c] as f32 * a).round() as u8; }
}
/// Dessine les textes (chiffres des règles, axes, cotes, noms des vues) avec la police de l'interface,
/// cerclés de noir comme à l'écran.
fn stamp_texts(rgb: &mut [u8], w: u32, h: u32, font_bytes: &[u8], boxes: &[ShotBox], texts: &[ShotText]) {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
    for b in boxes {
        for y in b.y0.max(0.0) as i32..b.y1.min(h as f32) as i32 { for x in b.x0.max(0.0) as i32..b.x1.min(w as f32) as i32 {
            blend(rgb, w, h, x, y, [0, 0, 0], 0.55);
        }}
    }
    let Ok(font) = FontRef::try_from_slice(font_bytes) else { return };
    for t in texts {
        let sf = font.as_scaled(PxScale::from(t.size));
        // mise en page simple sur une ligne
        let mut glyphs = Vec::new();
        let mut caret = 0.0f32;
        let mut prev = None;
        for ch in t.text.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev { caret += sf.kern(p, id); }
            glyphs.push((id, caret));
            caret += sf.h_advance(id);
            prev = Some(id);
        }
        let (asc, desc) = (sf.ascent(), sf.descent());
        let x0 = match t.anchor { 0 => t.x - caret * 0.5, 1 => t.x - caret, _ => t.x };
        let base = if t.anchor == 0 { t.y + (asc + desc) * 0.5 } else { t.y + asc };
        let o = (t.size / 12.0).max(1.0).round() as i32;
        for (pass, (dx, dy)) in [(-o, 0), (o, 0), (0, -o), (0, o), (0, 0)].into_iter().enumerate() {
            let col = if pass < 4 { [0, 0, 0] } else { t.col };
            let alpha = if pass < 4 { 0.9 } else { 1.0 };
            for &(id, gx) in &glyphs {
                let g = id.with_scale_and_position(sf.scale(), ab_glyph::point(x0 + gx, base));
                if let Some(og) = font.outline_glyph(g) {
                    let bb = og.px_bounds();
                    og.draw(|px, py, cov| {
                        let (gx, gy) = (bb.min.x as i32 + px as i32 + dx, bb.min.y as i32 + py as i32 + dy);
                        // coupé à sa vue (comme à l'écran)
                        if (gx as f32) < t.clip[0] || (gy as f32) < t.clip[1] || (gx as f32) >= t.clip[2] || (gy as f32) >= t.clip[3] { return; }
                        blend(rgb, w, h, gx, gy, col, cov * alpha);
                    });
                }
            }
        }
    }
}

/// Incruste le logo 3D-Patch en haut à gauche d'une copie d'écran RGB (≈13 % de la hauteur).
fn stamp_logo(rgb: &mut [u8], w: u32, h: u32) {
    let Ok(logo) = image::load_from_memory(include_bytes!("../assets/logo.png")) else { return };
    let side = ((h as f32 * 0.13).round() as u32).clamp(64, 512);
    let logo = logo.resize(side, side, image::imageops::FilterType::Lanczos3).to_rgba8();
    let m = (h as f32 * 0.02).round() as u32; // marge
    for (x, y, px) in logo.enumerate_pixels() {
        let (dx, dy) = (x + m, y + m);
        if dx >= w || dy >= h { continue; }
        let a = px[3] as f32 / 255.0 * 0.92;
        let i = ((dy * w + dx) * 3) as usize;
        for c in 0..3 { rgb[i + c] = (rgb[i + c] as f32 * (1.0 - a) + px[c] as f32 * a).round() as u8; }
    }
}

/// Calcul lourd lancé en arrière-plan (la fenêtre reste fluide, la jauge avance).
#[derive(Clone, Copy, PartialEq)]
enum JobKind { Lissage, Repair, Simp, Relief, Wire, Bool, Coupe, Creux, Texte, Texture }
type JobOut = Result<(Vec<f32>, String), String>;
struct Job { obj: usize, kind: JobKind, t0: std::time::Instant, handle: std::thread::JoinHandle<JobOut> }
fn spawn_job(obj: usize, kind: JobKind, f: impl FnOnce() -> JobOut + Send + 'static) -> Job {
    progress::reset();
    Job { obj, kind, t0: std::time::Instant::now(), handle: std::thread::spawn(f) }
}
fn job_key(k: JobKind) -> &'static str {
    match k { JobKind::Lissage => "lissage", JobKind::Repair => "repair", JobKind::Simp => "simplif", JobKind::Relief => "relief_j", JobKind::Wire => "wireframe", JobKind::Bool => "bool_j", JobKind::Coupe => "coupe_btn", JobKind::Creux => "creux_btn", JobKind::Texte => "texte_btn", JobKind::Texture => "tx_titre" }
}

fn main() {
    langue_regler(Lang::initiale()); // messages des modules dès le chargement du premier fichier
    // Sans argument : on démarre sur une primitive (aucun fichier requis → lancement fiable
    // par raccourci/double-clic, quel que soit le dossier courant).
    let path = std::env::args().nth(1).unwrap_or_else(|| "prim:logo".to_string()); // démarrage : le logo 3D en couleur
    // visionneuse lancée avec un fichier seul (double-clic) : la fenêtre s'ouvre d'abord, le fichier se charge
    // ensuite en arrière-plan (jauge CHARGEMENT) ; avec des options (--check…), chargement immédiat comme avant
    let charge_differe = !path.starts_with("prim:") && path != "logo" && path != "demo" && std::env::args().count() == 2;
    let mut cur_mesh = if charge_differe { stl::Mesh { verts: Vec::new(), min: Vec3::splat(-50.0), max: Vec3::splat(50.0) } } else { mesh_from_arg(&path) };
    if path == "prim:logo" || path == "logo" {
        // mise en scène demandée (capture 2026-10-16) : logo tourné face à −Y (vers la caméra), posé
        // coin à l'origine comme un objet importé (x 0→60, y 0→35, z 0→60), X vers la droite à l'écran
        for c in cur_mesh.verts.chunks_exact_mut(6) { c[0] = -c[0]; c[1] = -c[1]; c[3] = -c[3]; c[4] = -c[4]; }
        let (lo, hi) = (cur_mesh.min, cur_mesh.max);
        cur_mesh.min = Vec3::new(-hi.x, -hi.y, lo.z); cur_mesh.max = Vec3::new(-lo.x, -lo.y, hi.z);
        place_at_origin(&mut cur_mesh);
    }
    println!("Objet — {} triangles", cur_mesh.verts.len() / 18);

    // Test headless de la Réparation A.I : viewer <src> --envtest [sortie.stl]
    let argv: Vec<String> = std::env::args().collect();
    // Vérification headless : analyse de l'objet (triangles + défauts), sans fenêtre.
    // Test export : écrit l'objet dans <out> (format selon extension) puis quitte.
    if let Some(i) = argv.iter().position(|a| a == "--save") {
        if let Some(out) = argv.get(i+1) {
            let pth = std::path::Path::new(out);
            let ext = pth.extension().and_then(|e| e.to_str()).unwrap_or("stl").to_ascii_lowercase();
            let res = match ext.as_str() { "obj" => stl::write_obj(pth, &cur_mesh), "ply" => stl::write_ply(pth, &cur_mesh), "3mf" => threemf::write(pth, &cur_mesh.verts),
                "3dm" => rhino::export(pth, &cur_mesh.verts, None, if argv.iter().any(|a| a == "--rhino8") { 8 } else { 5 }).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                "skp" => sketchup::export(pth, &cur_mesh.verts, None).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                e if assimp::export_id(e).is_some() => assimp::export(pth, e, &cur_mesh.verts, None).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                _ => write_stl(pth, &cur_mesh) };
            match res { Ok(()) => println!("SAVE ok: {out} ({} tri)", cur_mesh.verts.len()/18), Err(e) => println!("SAVE err: {e}") }
        }
        return;
    }
    // Test relief : applique un motif et analyse le résultat.
    if let Some(i) = argv.iter().position(|a| a == "--relieftest") {
        let pr = relief::Params { pattern: relief::Pattern::Knurling, mapping: relief::Mapping::Triplanar, height_mm: 1.0, scale_mm: 8.0, invert: false, mask_normal: None };
        let (nv, before, after) = relief::apply(&cur_mesh.verts, &pr);
        let d = defects::analyze(&nv);
        println!("RELIEF: {before} -> {after} tri | {} trou(s) | {} erreur(s)", d.holes, d.face_errors);
        let _ = i;
        return;
    }
    // test : --assoc on|off|etat (associations « visionneuse 3D »)
    if let Some(i) = argv.iter().position(|a| a == "--assoc") {
        match argv.get(i + 1).map(|s| s.as_str()) {
            Some("on") => println!("{:?}", assoc::enable()),
            Some("off") => println!("{:?}", assoc::disable()),
            _ => println!("actif : {}", assoc::is_enabled()),
        }
        return;
    }
    if argv.iter().any(|a| a == "--check") {
        if argv.iter().any(|a| a == "--lisse") {
            let t0 = std::time::Instant::now();
            let (v, n) = defects::remove_internal(&cur_mesh.verts, 384);
            cur_mesh.verts = v;
            println!("LISSAGE INTERNE : {n} triangle(s) internes supprimés — {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0);
        }
        if let Some(r) = argv.iter().position(|a| a == "--env").and_then(|i| argv.get(i + 1)).and_then(|s| s.parse::<usize>().ok()) {
            let t0 = std::time::Instant::now();
            cur_mesh.verts = remesh::envelope(&cur_mesh.verts, r);
            println!("ENVELOPPE {r} : {} tri — {:.0} ms", cur_mesh.verts.len() / 18, t0.elapsed().as_secs_f32() * 1000.0);
        }
        if let Some(n) = argv.iter().position(|a| a == "--simp").and_then(|i| argv.get(i + 1)).and_then(|s| s.parse::<usize>().ok()) {
            let t0 = std::time::Instant::now();
            cur_mesh.verts = simplify::resample(&cur_mesh.verts, n);
            println!("SIMPLIFICATION -> {n} : {} tri — {:.0} ms", cur_mesh.verts.len() / 18, t0.elapsed().as_secs_f32() * 1000.0);
        }
        if argv.iter().any(|a| a == "--rep") {
            let mut prev = (usize::MAX, usize::MAX);
            for _ in 0..25 { cur_mesh.verts = defects::repair(&cur_mesh.verts);
                let d = defects::analyze(&cur_mesh.verts); let c=(d.holes,d.face_errors);
                if c==(0,0)||c==prev {break;} prev=c; }
        }
        if let Some(o) = argv.iter().position(|a| a == "--out").and_then(|i| argv.get(i + 1)) {
            let _ = write_stl(std::path::Path::new(o), &cur_mesh);
        }
        let d = defects::analyze(&cur_mesh.verts);
        let ext = cur_mesh.max - cur_mesh.min;
        println!("CHECK: {} tri | {} trou(s) | {} erreur(s) face | bbox {:.2}x{:.2}x{:.2}",
            cur_mesh.verts.len()/18, d.holes, d.face_errors, ext.x, ext.y, ext.z);
        return;
    }
    if let Some(i) = argv.iter().position(|a| a == "--envtest") {
        // perce quelques trous pour tester le bouchage
        let mut v = cur_mesh.verts.clone();
        let holed = (v.len() / 18 / 20).max(3) * 18; // retire ~5% des triangles
        v.truncate(v.len() - holed.min(v.len()));
        let before = defects::analyze(&v);
        println!("AVANT : {} tri, {} face(s) erreur, {} trou(s)", v.len()/18, before.face_errors, before.holes);
        if argv.iter().any(|a| a == "--autoloop") {
            let mut w = v.clone(); let mut prev = (usize::MAX, usize::MAX);
            for p in 1..=25 {
                w = defects::repair(&w);
                let d = defects::analyze(&w); let cur = (d.holes, d.face_errors);
                println!("  auto passe {p}: {} trou(s), {} erreur(s)", d.holes, d.face_errors);
                if cur == (0,0) || cur == prev { break; } prev = cur;
            }
            return;
        }
        let t0 = std::time::Instant::now();
        let env = if argv.iter().any(|a| a == "--raw") { remesh::envelope(&v, 200) } else { defects::repair(&remesh::envelope(&v, 140)) };
        let dt = t0.elapsed();
        let after = defects::analyze(&env);
        println!("APRÈS : {} tri, {} face(s) erreur, {} trou(s) — {:.0} ms", env.len()/18, after.face_errors, after.holes, dt.as_secs_f32()*1000.0);
        if let Some(out) = argv.get(i + 1) {
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for c in env.chunks_exact(6) { let p = Vec3::new(c[0],c[1],c[2]); lo = lo.min(p); hi = hi.max(p); }
            let _ = write_stl(std::path::Path::new(out), &stl::Mesh { verts: env, min: lo, max: hi });
            println!("écrit : {out}");
        }
        return;
    }

    let event_loop = EventLoop::new().unwrap();
    // icône de fenêtre (barre des tâches / Alt-Tab) = logo 3D-Patch embarqué
    let win_icon = image::load_from_memory(include_bytes!("../assets/logo.png")).ok().and_then(|im| {
        let im = im.resize(128, 128, image::imageops::FilterType::Lanczos3).to_rgba8();
        let (w, h) = im.dimensions();
        winit::window::Icon::from_rgba(im.into_raw(), w, h).ok()
    });
    // MODE VISIONNEUSE : lancé avec un fichier (double-clic sur un objet 3D) -> l'objet seul, sans
    // outils ni règles ; la souris tourne autour ; double-clic gauche = toute l'interface.
    let viewer_start = !path.starts_with("prim:") && path != "logo" && path != "demo" && std::env::var_os("PATCH3D_EDIT").is_none(); // test : PATCH3D_EDIT = fichier ouvert directement en édition
    let titre = if viewer_start {
        format!("3D-Patch — {}", std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned()))
    } else { "3D-Patch".to_string() };
    // taille au démarrage : 2560 × 1440 (pixels réels) si l'écran le permet ; écran pile à cette taille :
    // fenêtre agrandie (la barre de titre la ferait déborder) ; écran plus petit : taille habituelle
    let ecran = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().max_by_key(|m| { let s = m.size(); s.width as u64 * s.height as u64 }))
        .map(|m| m.size());
    let mut win_builder = WindowBuilder::new().with_title(titre).with_inner_size(LogicalSize::new(1024.0, 768.0)).with_window_icon(win_icon);
    match ecran {
        Some(e) if e.width > 2560 && e.height >= 1440 + 120 => win_builder = win_builder.with_inner_size(winit::dpi::PhysicalSize::new(2560u32, 1440u32)),
        Some(e) if e.width >= 2560 && e.height >= 1440 => win_builder = win_builder.with_maximized(true),
        _ => {}
    }
    let template = ConfigTemplateBuilder::new().with_depth_size(24);
    let (window, gl_config) = DisplayBuilder::new()
        .with_window_builder(Some(win_builder))
        .build(&event_loop, template, |cfgs| cfgs.reduce(|a, b| if b.num_samples() > a.num_samples() { b } else { a }).unwrap())
        .unwrap();
    let window = window.unwrap();
    let raw = window.raw_window_handle();
    let gl_display = gl_config.display();
    let ctx_attrs = ContextAttributesBuilder::new().build(Some(raw));
    let not_current = unsafe { gl_display.create_context(&gl_config, &ctx_attrs).expect("contexte GL") };
    let surf_attrs = window.build_surface_attributes(Default::default());
    let surface = unsafe { gl_display.create_window_surface(&gl_config, &surf_attrs).expect("surface GL") };
    let gl_context = not_current.make_current(&surface).unwrap();
    let _ = surface.set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));
    let gl = Arc::new(unsafe { glow::Context::from_loader_function(|s| gl_display.get_proc_address(&CString::new(s).unwrap()) as *const _) });
    // GPU de calcul : présent seulement si pilote matériel + OpenGL ≥ 4.3 ; désactivé par défaut,
    // choix mémorisé (fichier « gpu » du dossier de config).
    perf::measure_cpu();
    { let p = perf::profile(); println!("Processeur : {} cœurs, {:.2} Gvox/s, mémoire disponible {:?} Mo", p.threads, p.cpu_gvox, p.ram_mb); }
    let gpu = unsafe { gpu::Gpu::detect(&gl) };
    let mut gpu_on = gpu.is_some() && paths::flag("gpu");
    perf::GPU_CHECKED.store(gpu_on, std::sync::atomic::Ordering::Relaxed);
    match &gpu { Some((g, _)) => println!("GPU de calcul disponible : {}", g.name), None => println!("GPU de calcul indisponible (option masquée)") }
    // Test sans clic : --gputest compare Lissage interne et Enveloppe, processeur puis GPU.
    if argv.iter().any(|a| a == "--gputest") {
        if let Some((g, rx)) = &gpu {
            let v = cur_mesh.verts.clone();
            for on in [false, true] {
                perf::GPU_CHECKED.store(on, std::sync::atomic::Ordering::Relaxed);
                let v2 = v.clone();
                let h = std::thread::spawn(move || {
                    let t0 = std::time::Instant::now();
                    let (li, n) = defects::remove_internal(&v2, 384);
                    let t1 = t0.elapsed().as_secs_f32();
                    let t0 = std::time::Instant::now();
                    let env = remesh::envelope(&v2, 300);
                    let t2 = t0.elapsed().as_secs_f32();
                    let d = defects::analyze(&env);
                    eprintln!("  enveloppe : {} trou(s), {} erreur(s)", d.holes, d.face_errors);
                    (li.len() / 18, n, t1, env.len() / 18, t2)
                });
                while !h.is_finished() { unsafe { g.serve(&gl, rx); } std::thread::sleep(std::time::Duration::from_millis(2)); }
                let (lt, n, t1, et, t2) = h.join().unwrap();
                println!("  finesse retenue : enveloppe {} / lissage {} (GPU utilisé : {})", perf::env_res(), perf::liss_res(), perf::use_gpu());
                if on { println!("----- bulle -----\n{}\n-----------------", perf_tooltip(Lang::Fr, &g.name)); }
                println!("{} : lissage {lt} tri ({n} retirés) {t1:.2} s | enveloppe {et} tri {t2:.2} s", if on { "GPU" } else { "CPU" });
            }
        }
        return;
    }

    let mut r = cur_mesh.radius();
    let mut base = cur_mesh.center();
    let mut orbit = Orbit { yaw: CAM_YAW, pitch: CAM_PITCH, dist: r * 3.2, target: base };
    // logo de démarrage : mise en scène choisie par l'utilisateur (photo de référence) — logo posé
    // coin à l'origine, vue de trois quarts en plongée montrant toute la grille et les axes gradués
    if path == "prim:logo" || path == "logo" {
        // caméra côté −Y, un peu à droite, légère plongée (capture de référence de l'utilisateur)
        orbit = Orbit { yaw: CAM_YAW, pitch: CAM_PITCH, dist: r * 3.0, target: base };
    }
    let mut ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];

    let program;
    unsafe {
        gl.enable(glow::DEPTH_TEST);
        program = gl.create_program().unwrap();
        for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let s = gl.create_shader(ty).unwrap();
            gl.shader_source(s, src); gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) { panic!("shader: {}", gl.get_shader_info_log(s)); }
            gl.attach_shader(program, s);
        }
        gl.link_program(program);
    }
    // image 3D gardée entre deux images (voir cache3d) ; créée tant que le framebuffer de la fenêtre est lié
    let mut cache = unsafe { cache3d::Cache3d::new(&gl) };
    let plaque = unsafe { plaque::Plaque::new(&gl) };
    let mut evt_recu = true; // taille / échelle / thème changés depuis la dernière image : on redessine toute la scène
    let u = Uniforms {
        light: unsafe { gl.get_uniform_location(program, "uLight") },
        mvp: unsafe { gl.get_uniform_location(program, "uMVP") },
        model: unsafe { gl.get_uniform_location(program, "uModel") },
        eye: unsafe { gl.get_uniform_location(program, "uEye") },
        reflect: unsafe { gl.get_uniform_location(program, "uReflect") },
        dim: unsafe { gl.get_uniform_location(program, "uDim") },
        sel: unsafe { gl.get_uniform_location(program, "uSel") },
        gray: unsafe { gl.get_uniform_location(program, "uGray") },
        alpha: unsafe { gl.get_uniform_location(program, "uAlpha") },
        usecol: unsafe { gl.get_uniform_location(program, "uUseCol") },
        tint: unsafe { gl.get_uniform_location(program, "uTint") },
    };
    // Scène multi-objets : chaque objet a sa géométrie en coordonnées monde + son VAO/VBO.
    let mut scene: Vec<Obj> = vec![unsafe {
        if path == "prim:logo" || path == "logo" { Obj::with_colors(&gl, std::mem::take(&mut cur_mesh.verts), logo3d().1) }
        else { Obj::with_colors(&gl, std::mem::take(&mut cur_mesh.verts), CLI_COLS.lock().unwrap().take().unwrap_or_default()) }
    }];
    // Logo d'accueil : seulement regardable (tourner/zoomer/déplacer la vue). Le premier appui sur
    // n'importe quel bouton de l'interface l'efface entièrement (historique compris) AVANT l'action
    // du bouton : aucune opération ne peut s'appliquer au logo.
    let mut splash = (path == "prim:logo" || path == "logo") && std::env::var_os("PATCH3D_NOSPLASH").is_none();
    let mut sel: Option<usize> = if scene[0].nverts > 0 && !splash && !viewer_start { Some(0) } else { None };
    let mut unit = Unit::initiale();
    let mut grille_dims = dims_grille(unit, &scene, r);
    let mut grid_step = grille_dims.0;
    let (mut grid, mut rule, mut ruler_majors) = unsafe { build_grid_ruler(&gl, grille_dims) };


    // Mode « photo » : viewer <src> --shot <png> (vue 3D seule).
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--two") {
        // scène de test : 2 objets, le premier sélectionné (pour vérifier le mode Texturé)
        if let Ok(mut m) = occt::primitive(occt::Prim::Sphere { r: 14.0 }, occt::DEFLEXION) {
            for c in m.verts.chunks_exact_mut(6) { c[0] += 55.0; }
            scene.push(unsafe { Obj::new(&gl, m.verts) });
        }
        sel = Some(0);
    }
    if args.iter().any(|a| a == "--envelope") {
        // remplace l'objet par son enveloppe A.I (en mémoire, normales lisses conservées)
        let env = remesh::envelope(&scene[0].verts, 200);
        if env.len() >= 18 { unsafe { scene[0].delete(&gl); } scene[0] = unsafe { Obj::new(&gl, env) }; sel = Some(0); }
    }
    if let Some(i) = args.iter().position(|a| a == "--wire") {
        // transforme l'objet en treillis imprimable (arêtes -> barreaux Ø mm)
        let d: f32 = args.get(i+1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let mres: usize = args.iter().position(|a| a == "--meshres").and_then(|j| args.get(j+1)).and_then(|s| s.parse().ok()).unwrap_or(50);
        let dec = remesh::decimate(&scene[0].verts, mres);
        println!("Décimation : {} -> {} triangles (res {mres})", scene[0].verts.len()/18, dec.len()/18);
        let mut src = if dec.len() >= 18 { dec } else { scene[0].verts.clone() };
        let found: f32 = args.iter().position(|a| a == "--found").and_then(|j| args.get(j+1)).and_then(|s| s.parse().ok()).unwrap_or(1.5);
        remesh::floor_foundation(&mut src, found);
        match remesh::wireframe_solid(&src, d*0.5, 6) {
            Ok(w) => { println!("Fil de fer : {} triangles (Ø {d} mm)", w.len()/18);
                       unsafe { scene[0].delete(&gl); } scene[0] = unsafe { Obj::new(&gl, w) }; sel = Some(0); }
            Err(e) => { eprintln!("fil de fer impossible : {e}"); }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "shot.png".into());
        let (w, h) = (1100i32, 800i32);
        unsafe {
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            let color = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, w, h);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(color));
            let depth = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, w, h);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            if std::env::var("VIEWER_QUADSHOT").is_ok() {
                gl.disable(glow::SCISSOR_TEST);
                gl.viewport(0, 0, w, h);
                gl.clear_color(0.20, 0.22, 0.26, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                gl.enable(glow::SCISSOR_TEST);
                for k in view::ALL_VIEWS {
                    if let Some(sr) = view::screen_rect(ViewMode::Quad, k, w as f32, h as f32, 40.0, 0.0) {
                        let (gx, gy, gw, gh) = view::gl_rect(sr, h as f32);
                        if gw <= 2 || gh <= 2 { continue; }
                        gl.viewport(gx, gy, gw, gh);
                        gl.scissor(gx + 1, gy + 1, gw - 2, gh - 2);
                        gl.clear_color(0.10, 0.12, 0.15, 1.0);
                        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                        let (mvp, eye) = view_mvp(k, &orbit, &ortho, base, gw as f32 / gh as f32);
                        draw_view(&gl, program, &scene, sel, &u, &grid, &rule, &mvp, eye, true, true, 0.0, 1.0, 0.0, false, 1.0, None);
                    }
                }
                gl.disable(glow::SCISSOR_TEST);
            } else {
                gl.viewport(0, 0, w, h);
                gl.clear_color(0.10, 0.12, 0.15, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                // test : PATCH3D_CAM="yaw pitch zoom" (radians, radians, facteur de distance)
                if let Ok(c) = std::env::var("PATCH3D_CAM") {
                    let v: Vec<f32> = c.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 3 { orbit.yaw = v[0]; orbit.pitch = v[1]; orbit.dist *= v[2]; }
                }
                let (mvp, eye) = orbit.mvp(w as f32 / h as f32);
                // --mode defects|textured|transparent|wireframe (par défaut: defects)
                let m = args.iter().position(|a| a == "--mode").and_then(|i| args.get(i+1)).map(|s| s.as_str()).unwrap_or("defects");
                let (refl, dimv, grayv, show_def, wire, alphav) = match m {
                    "textured"    => (0.0, 1.0, 0.0, false, false, 1.0),
                    "transparent" => (0.0, 1.0, 0.0, false, false, 0.30),
                    "wireframe"   => (0.0, 1.0, 0.0, false, true,  1.0),
                    _             => (0.0, 0.6, 1.0, true,  false, 1.0),
                };
                if alphav < 1.0 { gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA); gl.depth_mask(false); }
                if wire { gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE); }
                draw_view(&gl, program, &scene, sel, &u, &grid, &rule, &mvp, eye, true, true, refl, dimv, grayv, m=="textured", alphav, None);
                if alphav < 1.0 { gl.disable(glow::BLEND); gl.depth_mask(true); }
                if wire { gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL); }
                if show_def {
                    let a = mvp.to_cols_array();
                    let d = defects::analyze(&scene[0].verts);
                    let mut bl = defects::LineSet::new(&gl); bl.update(&gl, &d.boundary);
                    let mut el = defects::LineSet::new(&gl); el.update(&gl, &d.errors);
                    gl.disable(glow::DEPTH_TEST);
                    el.draw(&gl, &a, [1.0, 0.85, 0.0, 1.0], 3.0);
                    bl.draw(&gl, &a, [1.0, 0.15, 0.15, 1.0], 4.0);
                    gl.enable(glow::DEPTH_TEST);
                }
            }
            let mut buf = vec![0u8; (w * h * 4) as usize];
            gl.read_pixels(0, 0, w, h, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(&mut buf));
            let row = (w * 4) as usize;
            let mut flip = vec![0u8; buf.len()];
            for y in 0..h as usize { let s = (h as usize - 1 - y) * row; flip[y*row..y*row+row].copy_from_slice(&buf[s..s+row]); }
            image::save_buffer(&out, &flip, w as u32, h as u32, image::ColorType::Rgba8).unwrap();
        }
        println!("image écrite : {out}");
        return;
    }

    let egui_ctx = egui::Context::default();
    let mut egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &window, None, None);
    let mut egui_painter = egui_glow::Painter::new(gl.clone(), "", None).expect("egui painter");

    // Photo de l'auteur (menu Options ▸ À propos), embarquée dans le binaire.
    let author_tex: Option<egui::TextureHandle> = {
        let bytes = include_bytes!("../assets/author.jpg");
        image::load_from_memory(bytes).ok().map(|img| {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img);
            egui_ctx.load_texture("author_photo", ci, egui::TextureOptions::LINEAR)
        })
    };

    let mut size = window.inner_size();
    let mut mode = ViewMode::Single(ViewKind::ThreeD); // par défaut : vue 3D en visualisation
    // test sans clic : PATCH3D_MODE=quad ouvre directement en 4 vues
    if std::env::var("PATCH3D_MODE").as_deref() == Ok("quad") { mode = ViewMode::Quad; }
    let mut viewer = viewer_start;
    // « 3D-Patch en visionneuse 3D » : état lu en arrière-plan, bascule exécutée en arrière-plan
    let assoc_etat = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    { let a = assoc_etat.clone(); std::thread::spawn(move || a.store(assoc::is_enabled(), std::sync::atomic::Ordering::Relaxed)); }
    let mut assoc_job: Option<std::thread::JoinHandle<Result<String, String>>> = None;
    let mut show_ruler = !viewer; // règles actives dès le démarrage (masquées en visionneuse)
    let mut last_left: Option<(std::time::Instant, (f64, f64))> = None; // double-clic gauche (visionneuse)
    let mut job: Option<Job> = None;
    let mut file_job: Option<std::thread::JoinHandle<FileOut>> = None;
    let mut chargement_initial = charge_differe; // fichier de la visionneuse en cours de chargement (jauge CHARGEMENT)
    // jauge CHARGEMENT des formats sans avancement : durée typique estimée d'après la taille (≈ 50 Mo/s, mesuré)
    let debut_appli = std::time::Instant::now();
    let taille_chargement = {
        let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        // conception SolidCraft paramétrique : petit fichier mais long calcul par SolidCraft (congés…)
        let taille = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f32;
        if matches!(ext.as_str(), "solidcraft" | "solvecraft") { 15.0 }
        // CAO à convertir (couture + maillage OpenCASCADE) : ≈ 1 Mo/s mesuré
        else if matches!(ext.as_str(), "3dm" | "step" | "stp" | "iges" | "igs") { (taille / 1.0e6).max(0.3) }
        else { (taille / 50.0e6).max(0.3) }
    };
    if charge_differe {
        let p = std::path::PathBuf::from(&path);
        file_job = Some(std::thread::spawn(move || match charger_objets(&p) {
            Ok(objs) => { paths::remember(&p); let d = analyse_ouverture(&objs); FileOut::Opened(p, objs, false, d) }
            Err(e) => FileOut::Failed(format!("{} — {e}", trc("err_import"))),
        }));
    }
    let mut bar_row: Option<(f32, f32)> = None; // (haut, hauteur) réels de la rangée de boutons de la barre
    let mut bar_wr1: f32 = 270.0; // largeur NATURELLE (échelle 1) mesurée du groupe droit de la barre
    let mut job_done: Option<std::time::Instant> = None; // jauge à 100 % un court instant
    let mut simp_env = true; // Simplification : passer d'abord par une enveloppe unique (supprime les jours entre pièces)
    let mut simp_target: usize = 0; // Simplification : nombre de triangles visé
    let mut simp_seen: usize = usize::MAX; // dernier nombre réel affiché (resynchronise la cible)
    let mut lang = Lang::initiale();
    let mut dark = std::env::var_os("PATCH3D_CLAIR").is_none(); // test sans clic : PATCH3D_CLAIR=1 = thème clair
    let mut taille_molette: Option<std::time::Instant> = None; // dernier cran de molette sur TAILLE
    let mut prochain_repaint: Option<std::time::Instant> = None; // réveil programmé (repaint différé d'egui)
    let mut selection: Option<Vec3> = None;
    let mut face_normal: Option<Vec3> = None; // normale de la dernière face cliquée (relief ciblé)
    let mut cursor = (0.0f64, 0.0f64);
    let mut drag: Option<Drag> = None;
    let mut last: Option<(f64, f64)> = None;
    let mut last_ground: Option<Vec3> = None; // pour le déplacement d'objet dans le plan Z=0
    let mut last_mid: Option<std::time::Instant> = None;
    // test sans clic : PATCH3D_GRILLE=0..3 (grille+axes, grille, axes, rien)
    if let Some(n) = std::env::var("PATCH3D_GRILLE").ok().and_then(|t| t.parse::<u8>().ok()) { GRILLE_MODE.store(n % 4, std::sync::atomic::Ordering::Relaxed); }
    let mut show_grid = matches!(grille_mode(), 0 | 1);
    let mut snap_size = 1.0f32;
    let snap_direct = false;
    let mut disp = DisplayMode::Textured; // à l'ouverture : objet toujours affiché en texturé
    // test : PATCH3D_OPEN=fil démarre en fil de fer (bouton de la visionneuse)
    if std::env::var("PATCH3D_OPEN").as_deref() == Ok("fil") { disp = DisplayMode::Wireframe; }
    let mut bnd_lines = unsafe { defects::LineSet::new(&gl) };
    let mut err_lines = unsafe { defects::LineSet::new(&gl) };
    let mut defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
    // Historique scène pour Annuler / Refaire : liste de scènes (chaque scène = liste des sommets par objet).
    let snapshot = |scene: &Vec<Obj>| -> Vec<(Vec<f32>, Vec<f32>)> { scene.iter().map(|o| (o.verts.clone(), if o.has_colors() { o.cols.clone() } else { Vec::new() })).collect() };
    let mut history: Vec<Vec<(Vec<f32>, Vec<f32>)>> = vec![snapshot(&scene)];
    let mut hist_idx = 0usize;
    // Primitive à créer depuis le panneau d'outils (côté en mm choisi dans l'UI).
    let mut new_prim: Option<occt::Prim> = None;
    let mut prim_size = 30.0f32;
    let mut tool_tab = 1u8; // 0 = Courbes, 1 = Solides (panneau façon MoI)
    let mut wire_nozzle = 0.4f32;       // largeur de buse (mm) = mini imprimable
    let mut wire_thick = 0.4f32;        // épaisseur réelle du fil (mm), >= buse
    let mut wire_found = 1.5f32;        // hauteur de fondation : fils collés au sol (mm)
    let mut wire_sides = 6u32;          // facettes des barreaux (roundeur)
    let mut wire_mesh_res = 50u32;      // résolution de décimation du maillage source
    let mut wire_msg: Option<String> = None;
    // CREUSER : épaisseur de paroi, trou d'évacuation (Ø), retour visible dans le panneau
    let (mut creux_ep, mut creux_trou, mut creux_diam) = (2.0f32, false, 3.0f32); // trou décoché par défaut
    let mut creux_msg: Option<String> = None;
    // MODE TEXTURE (atelier.rs, moteur matiere) : état et panneau ; aperçu GL affiché à la place de l'objet
    let mut tex = atelier::Atelier::new();
    tex.actif = matches!(std::env::var("PATCH3D_OPEN").as_deref(), Ok("texture") | Ok("pinceau")); // test sans clic (pinceau = armé)
    if std::env::var("PATCH3D_OPEN").as_deref() == Ok("pinceau") { tex.armer_pinceau(); }
    let mut tx_vue: Option<(usize, Obj)> = None;
    let mut tx_curseur: Option<(egui::Pos2, f32)> = None; // cercle du pinceau à l'écran (points)
    // marqueur temporaire du trou d'évacuation (fil de fer rouge) : (objet, son coin min, segments) ;
    // disparaît quand l'objet est désélectionné, déplacé ou modifié
    let mut trou_marque: Option<(usize, Vec3, Vec<[Vec3; 2]>)> = None; // retour visible (succès / erreur) dans le panneau
    let mut status_msg: Option<(String, bool, std::time::Instant)> = None; // toast (texte, erreur?, horodatage)
    // test sans clic : PATCH3D_TOAST="texte" affiche le bandeau d'info au démarrage
    if let Ok(t) = std::env::var("PATCH3D_TOAST") { status_msg = Some((t, false, std::time::Instant::now())); }
    // Outil de tracé actif (Courbes) : on dessine sur le plan Z=0 par cliquer-glisser.
    let mut draw_tool: Option<DrawShape> = None;
    let mut draw_start: Option<Vec3> = None;
    let mut draw_cur: Option<Vec3> = None;
    let mut sketch_view: Option<ViewKind> = None;
    // Navigation par overlay (au survol) : outil actif + rectangle de zoom "Zone".
    let mut nav_tool: Option<NavTool> = None;
    let mut zone: Option<(ViewKind, (f64, f64), (f64, f64))> = None; // (vue, départ px, courant px)
    // Relief / texture (façon BumpMesh).
    let mut relief_pattern = relief::Pattern::Knurling;
    let mut relief_mapping = relief::Mapping::Triplanar;
    let mut relief_height = 1.0f32;
    let mut relief_scale = 10.0f32;
    let mut relief_invert = false;
    let mut relief_face_only = true; // graver seulement la face cliquée
    let mut tex_cache: Vec<egui::TextureHandle> = Vec::new();
    let mut resize_lock = true; // chaîne de proportions
    // Outil de rotation (double clic droit sur l'objet) + vue où se fait le geste
    let mut gizmo: Option<gizmo::Gizmo> = None;
    let mut rot_view = ViewKind::ThreeD;
    let mut last_right: Option<(std::time::Instant, (f64, f64))> = None; // double clic droit
    // PIÈCES d'un objet : pièce choisie au clic gauche (objet, pièce, pièce de chaque triangle, nombre, signature),
    // son surlignage, le menu du clic droit (position, objet, pièce, nombre) et la demande de dégroupage
    let mut piece: Option<(usize, u32, Vec<u32>, usize, (Vec3, Vec3, usize))> = None;
    let mut piece_vue: Option<(Obj, (Vec3, Vec3, usize))> = None;
    let mut menu_piece: Option<(egui::Pos2, usize, u32, usize, Vec<usize>)> = None; // + objets à grouper (Maj+clic)
    let mut degroup_req: Option<(usize, u32, bool)> = None;
    let mut grouper_req: Option<Vec<usize>> = None;
    let mut clic_droit: Option<(std::time::Instant, (f64, f64))> = None; // appui droit (menu si relâché sans glisser)
    // test sans clic : PATCH3D_GRAV=1 appuie sur GRAVITÉ (outil de rotation ouvert sur l'objet 0)
    let mut grav_test = std::env::var_os("PATCH3D_GRAV").is_some();
    let mut grav_anim: Option<GravAnim> = None;
    // SURFACE : (objet, faces planes de pose affichées en rouge), face sous la souris
    let mut surf: Option<(usize, Vec<gravite::FacePose>)> = None;
    let mut surf_survol: Option<usize> = None;
    // ANALYSE DES SURFACES (bouton SURFACE d'OBJ-AXE) : calcul en arrière-plan (objet, départ, fil) ;
    // ligne de la liste survolée (la face s'allume sur l'objet comme au survol de la vue)
    let mut surf_analyse: Option<(usize, std::time::Instant, std::thread::JoinHandle<Vec<gravite::FacePose>>)> = None;
    let mut surf_liste: Option<usize> = None;
    // Panneau BOOLÉENS : remplace le panneau d'outils ; A = objet gardé, B = objet outil
    // PRÉPARATION À L'IMPRESSION (3D-IMP) : panneau qui occupe toute la colonne de droite
    let mut panneau_imp = std::env::var("PATCH3D_OPEN").as_deref() == Ok("impression");
    let (mut imp_angle, mut imp_buse, mut imp_mat) = (45.0f32, 0.4f32, 0usize);
    let (mut imp_voir_s, mut imp_voir_f) = (true, true);
    let mut imp_tr: usize = usize::MAX;             // trancheur (TRANCHEURS.len() = autre programme)
    let mut imp_autre: Option<std::path::PathBuf> = None;
    let mut imp_installes: Option<Vec<Option<impression::Lancement>>> = None;
    let mut imp_detect: Option<std::thread::JoinHandle<Vec<Option<impression::Lancement>>>> = None;
    let mut imp_choix_prog: Option<std::thread::JoinHandle<Option<std::path::PathBuf>>> = None;
    let mut imp_an: (String, Option<std::sync::Arc<impression::Analyse>>) = (String::new(), None);
    let mut imp_job: Option<(String, std::thread::JoinHandle<impression::Analyse>)> = None;
    let mut imp_msg: Option<(String, bool)> = None;
    // MISE EN FORME : panneau d'outils (remplace la colonne de droite, comme BOOLÉENS)
    let mut panneau_forme = std::env::var("PATCH3D_OPEN").as_deref() == Ok("forme");
    let mut forme_pos_t: Option<std::time::Instant> = None; // dernier déplacement par la position (glisser = 1 seule étape d'annulation)
    let (mut f_angle, mut f_axe_rot, mut f_ech, mut f_ech3) = (45.0f32, 2usize, 100.0f32, [100.0f32; 3]);
    let (mut f_n, mut f_pas, mut f_dir, mut f_cn, mut f_cr) = (3u32, 30.0f32, 0usize, 6u32, 40.0f32);
    let mut panneau_bool = matches!(std::env::var("PATCH3D_OPEN").as_deref(), Ok("booleens") | Ok("coupe") | Ok("texte") | Ok("image"));
    // sélection multiple (Ctrl+clic, comme 3D Builder), dans l'ordre des clics ; contient `sel`
    let mut multi: Vec<usize> = Vec::new();
    let mut mods = winit::keyboard::ModifiersState::empty();
    let mut bool_garder_b = false;
    // découpe (ciseaux) : armé = le prochain glisser dans une vue trace la ligne de coupe
    let mut coupe_arme = std::env::var("PATCH3D_OPEN").as_deref() == Ok("coupe");
    // TEXTE & RELIEF : sous-panneau de BOOLÉENS (toute la colonne de droite)
    let mut panneau_texte = matches!(std::env::var("PATCH3D_OPEN").as_deref(), Ok("texte") | Ok("image"));
    // INSÈRE IMAGE : même sous-panneau et même pose que le texte, contours tirés d'une image vectorisée
    let mut txt_image = std::env::var("PATCH3D_OPEN").as_deref() == Ok("image");
    // test sans clic : PATCH3D_IMAGE=fichier charge l'image au démarrage
    let mut img: Option<vectorise::Image> = std::env::var("PATCH3D_IMAGE").ok().and_then(|p| vectorise::Image::charger(std::path::Path::new(&p)).ok());
    let mut img_prm = vectorise::Params::default();
    let mut img_largeur = 40.0f32;
    let mut img_choix: Option<std::thread::JoinHandle<Option<std::path::PathBuf>>> = None;
    let mut img_tex: Option<(u64, egui::TextureHandle)> = None; // aperçu N&B (génération de la vectorisation)
    let mut img_vu = String::new(); // réglages de l'image au dernier calcul du tracé
    let mut txt_saisie = String::from("3D-Patch");
    let (mut txt_just, mut txt_taille, mut txt_relief, mut txt_suivre) = (1u8, 10.0f32, true, false);
    let mut txt_police: Option<usize> = None;
    let mut txt_filtre = String::new();
    let mut txt_cadre: Option<CadreTexte> = None;
    // tracé du cadre : 0 rien, 1 armé (clic 1 = départ), 2 X, 3 Y, 4 Z (profondeur)
    let mut txt_echelle: Option<(f32, f32)> = None; // poignée de coin tenue : (taille de départ, distance au centre de départ)
    // poignée de profondeur tenue : (cadre de départ, longueur de flèche de départ, normale du plan de glisse face à la vue)
    let mut txt_prof_glisse: Option<(CadreTexte, f32, Vec3)> = None;
    let mut txt_k = ViewKind::ThreeD;
    let mut outil_ann: Option<(usize, OutilAnn)> = None; // (étape d'historique de l'opération, outil à rétablir)
    let (mut txt_lance, mut coupe_lance): (Option<CadreTexte>, Option<booleen::Cadre>) = (None, None);
    let mut txt_police_ui: Option<usize> = None; // police chargée dans l'interface (champ de saisie)
    let mut txt_deplace: Option<Vec3> = None;     // cadre de texte déplacé : décalage (centre − point saisi)
    let mut txt_sel = false;                      // cadre de texte sélectionné (contour jaune ; Suppr l'efface) // fenêtre où se trouve la souris pendant le tracé
    let mut txt_rot: Option<(f32, f32)> = None; // rotation par poignée : (angle de départ, angle du curseur au départ)
    let mut txt_apercus: std::collections::HashMap<(usize, String), (Vec<[[f32; 2]; 3]>, f32, f32)> = std::collections::HashMap::new();
    let mut txt_contours: (String, Vec<Vec<[f64; 2]>>) = (String::new(), Vec::new());
    let mut coupe: Option<booleen::Cadre> = None;
    let mut coupe_tir: Option<ViewKind> = None; // tracé en cours dans cette vue
    let (mut export_apres_rep, mut ouvrir_export) = (false, false);
    // jauge EXPORTATION de la visionneuse : écriture du fichier lancée depuis l'icône Export, fin (100 % un instant)
    let (mut export_fichier, mut export_fin): (bool, Option<std::time::Instant>) = (false, None); // icône Export de la visionneuse : réparer d'abord, puis le menu
    let mut coupe_etape = 0u8; // 1 = clic 1 fait, le trait L suit la souris ; 2 = clic 2 fait, l se déplie
    let mut coupe_garder: u8 = 0; // 0 = les deux morceaux, 1 = côté flèche, 2 = autre côté
    let mut coupe_ecart = true;
    let mut coupe_section: (String, Vec<[Vec3; 2]>) = (String::new(), Vec::new()); // ligne de coupe mémorisée
    let mut coupe_sel = false; // le cadre est sélectionné comme un objet (Suppr l'enlève)
    // lignes d'intersection des objets sélectionnés (panneau Booléens), calculées en arrière-plan
    let mut inter_seg: (String, Vec<[Vec3; 2]>) = (String::new(), Vec::new());
    let mut inter_job: Option<(String, std::thread::JoinHandle<Vec<[Vec3; 2]>>)> = None;
    let mut presse_papiers: Vec<(Vec<f32>, Vec<f32>)> = Vec::new(); // Ctrl+C / Ctrl+X : (sommets, couleurs) des objets copiés
    let mut mode_avant_coupe: Option<ViewMode> = None; // vue à rétablir à la fermeture du panneau Booléens / fin de découpe
    let mut fin_outil = false; // booléen / découpe terminé : retour à la boîte à outils normale
    let mut panneau_prec = false;
    // une seule fonction ouverte à la fois : OBJ-AXE, BOOLÉENS, FORME, 3D-IMP, éclairage, outils de tracé
    let mut modes_prec = [false; 7]; // état du panneau Booléens à l'image précédente (ouverture = passage en 4 vues)
    let mut coupe_glisse: Option<(ViewKind, Vec3)> = None; // cadre déplacé à la souris
    let mut coupe_ecart_job: Option<(Vec3, u8)> = None; // écart et choix au lancement du calcul
    let mut bool_suppr: Vec<usize> = Vec::new(); // objets à retirer de la scène quand le calcul aboutit
    let mut bool_test = std::env::var("PATCH3D_REQ").ok().filter(|r| r.starts_with("bool_")); // test sans clic
    // PROJECTEUR : mode de placement de la lumière + vue où l'on glisse
    let mut light_mode = std::env::var_os("PATCH3D_LUMIERE").is_some(); // test sans clic : mode ouvert
    if let Ok(t) = std::env::var("PATCH3D_LUMIERE") {
        let v: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 2 { let (a, e) = (v[0].to_radians(), v[1].to_radians()); lumiere_regler(Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())); }
    }
    let mut light_drag: Option<ViewKind> = None;
    let dome = |scene: &Vec<Obj>| -> (Vec3, f32) { let (lo, hi) = scene_bounds(scene); ((lo + hi) * 0.5, (((hi - lo) * 0.5).length() * 1.3).max(1.0)) };
    // test sans clic : PATCH3D_ROT=z (outil ouvert, anneau survolé) ou z:35 (rotation en cours de 35°)
    // ou z:35:cuire (rotation appliquée à l'objet)
    let mut rot_test: Option<(usize, f32, bool)> = std::env::var("PATCH3D_ROT").ok().map(|t| {
        let mut it = t.split(':');
        let ax = match it.next().unwrap_or("z") { "x" | "X" => 0, "y" | "Y" => 1, _ => 2 };
        let deg = it.next().and_then(|d| d.parse::<f32>().ok()).unwrap_or(f32::NAN);
        (ax, deg, it.next().is_some())
    });
    println!("Défauts : {} face(s) en erreur, {} trou(s)", defects.face_errors, defects.holes);
    // Préférences mémorisées (tout sauf l'écran d'accueil, toujours montré au lancement).
    // Les variables PATCH3D_* de test restent prioritaires.
    {
        let pf = paths::prefs();
        let b = |k: &str| pf.get(k).map(|v| v == "1");
        let f = |k: &str| pf.get(k).and_then(|v| v.parse::<f32>().ok()).filter(|x| x.is_finite());
        let u = |k: &str| pf.get(k).and_then(|v| v.parse::<u32>().ok());
        if std::env::var_os("PATCH3D_CLAIR").is_none() { if let Some(v) = b("sombre") { dark = v; } }
        if std::env::var_os("PATCH3D_GRILLE").is_none() {
            if let Some(n) = u("grille") { GRILLE_MODE.store((n % 4) as u8, std::sync::atomic::Ordering::Relaxed); show_grid = matches!(n % 4, 0 | 1); }
        }
        if !viewer {
            if let Some(v) = b("regles") { show_ruler = v; }
            if let Some(m) = pf.get("vue").and_then(|v| vue_de_code(v)) { mode = m; }
        }
        if let Some(v) = b("simp_env") { simp_env = v; }
        if let Some(v) = b("garder_proportions") { resize_lock = v; }
        if let Some(v) = f("fil_buse") { wire_nozzle = v.clamp(0.05, 10.0); }
        if let Some(v) = f("fil_epaisseur") { wire_thick = v.clamp(0.1, 10.0); }
        if let Some(v) = f("fil_fondation") { wire_found = v.clamp(0.0, 50.0); }
        if let Some(v) = u("fil_facettes") { wire_sides = v.clamp(3, 64); }
        if let Some(v) = u("fil_resolution") { wire_mesh_res = v.clamp(6, 200); }
        if let Some(p) = pf.get("relief_motif").and_then(|k| relief::PATTERNS.into_iter().find(|p| p.key() == k)) { relief_pattern = p; }
        if let Some(m) = pf.get("relief_projection").and_then(|k| match k.as_str() { "planz" => Some(relief::Mapping::PlanarZ), "cyl" => Some(relief::Mapping::Cylindrical), "tri" => Some(relief::Mapping::Triplanar), _ => None }) { relief_mapping = m; }
        if let Some(v) = f("relief_hauteur") { relief_height = v; }
        if let Some(v) = f("relief_echelle") { relief_scale = v; }
        if let Some(v) = b("relief_inverser") { relief_invert = v; }
        if let Some(v) = b("relief_face_seule") { relief_face_only = v; }
        if let Some(v) = f("taille_snap") { snap_size = v; }
        if let Some(v) = f("imp_angle") { imp_angle = v.clamp(10.0, 89.0); }
        if let Some(v) = f("imp_buse") { imp_buse = v.clamp(0.1, 2.0); }
        if let Some(v) = u("imp_matiere") { imp_mat = (v as usize).min(impression::MATIERES.len() - 1); }
        if let Some(v) = u("imp_trancheur") { imp_tr = (v as usize).min(impression::TRANCHEURS.len()); }
        if let Some(v) = pf.get("imp_autre").filter(|v| !v.is_empty()) { imp_autre = Some(std::path::PathBuf::from(v)); }
        if let Some(v) = f("taille_primitive") { prim_size = v; }
        if let Some(l) = pf.get("lumiere").map(|t| t.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|l| l.len() == 3) {
            let d = Vec3::new(l[0], l[1], l[2]);
            if d.is_finite() && d.length() > 1e-3 { lumiere_regler(d.normalize()); }
        }
    }
    let mut prefs_ecrites = String::new();
    window.request_redraw();

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(winit::event_loop::ControlFlow::Wait);
        // rafraîchissement DIFFÉRÉ demandé par egui (animations : engrenage, clignotements…) :
        // on dort jusqu'à l'échéance au lieu de redessiner en boucle
        if let Some(t) = prochain_repaint {
            if std::time::Instant::now() >= t { prochain_repaint = None; window.request_redraw(); }
            else { elwt.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(t)); }
        }
        if let Event::WindowEvent { event, .. } = event {
            // l'image 3D gardée n'est plus valable qu'après un changement de taille, d'échelle, de thème ou une
            // réapparition (tout le reste est dans sa clé) : un simple mouvement de souris ne relance plus le
            // rendu complet de la scène (coûteux sur un gros maillage)
            if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ThemeChanged(_) | WindowEvent::Occluded(false)) { evt_recu = true; }
            let resp = egui_state.on_window_event(&window, &event);
            // egui-winit répond « repaint » même à RedrawRequested : le suivre relancerait une image sans fin
            if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }
            let egui_grab = resp.consumed;
            // clic gauche relâché au-dessus de l'interface (panneau, étiquette…) : les poignées sont lâchées quand
            // même, sinon le cadre resterait accroché à la souris
            if egui_grab && matches!(event, WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. }) {
                coupe_glisse = None; txt_rot = None; txt_deplace = None; txt_echelle = None; txt_prof_glisse = None; light_drag = None;
            }
            let (wf, hf) = (size.width as f32, size.height as f32);
            let sf = window.scale_factor() as f32;
            // visionneuse : ni barre ni panneau -> la vue occupe toute la fenêtre
            let bar_px = if viewer { 0.0 } else { BAR_H * sf };
            // mode Texture : colonne élargie + bandeau en haut (les vues 3D commencent dessous)
            let panel_w = if tex.actif && !viewer { PANEL_TEX_W } else { PANEL_W };
            let side_px = if viewer { 0.0 } else { panel_w * sf };
            let rect_of = |m: ViewMode, k: ViewKind| view::screen_rect(m, k, wf, hf, bar_px, side_px);
            let aspect_of = |m: ViewMode, k: ViewKind| rect_of(m, k).map(|(_, _, w, h)| w / h.max(1.0)).unwrap_or(1.0);

            match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::ModifiersChanged(m) => mods = m.state(),
                WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                    match event.logical_key {
                        // Échap : ferme d'abord l'outil de rotation (sinon quitte)
                        Key::Named(NamedKey::Escape) if panneau_texte && txt_sel => { txt_sel = false; window.request_redraw(); }
                        Key::Named(NamedKey::Escape) if surf.is_some() || surf_analyse.is_some() => { surf = None; surf_survol = None; surf_analyse = None; window.request_redraw(); }
                        Key::Named(NamedKey::Escape) => if coupe_arme || coupe.is_some() { coupe_arme = false; coupe = None; coupe_tir = None; coupe_etape = 0; window.request_redraw(); }
                            else if light_mode { light_mode = false; window.request_redraw(); }
                            else if gizmo.is_some() { gizmo = None; window.request_redraw(); } else { elwt.exit() },
                        Key::Character(ref c) if c.eq_ignore_ascii_case("f") => {
                            let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                            orbit = Orbit { yaw: CAM_YAW, pitch: CAM_PITCH, dist: r * 3.2, target: base };
                            ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];
                            window.request_redraw();
                        }
                        // Ctrl+A : tout sélectionner (comme 3D Builder)
                        Key::Character(ref c) if c.eq_ignore_ascii_case("a") && mods.control_key() && !splash && !viewer => {
                            multi = (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect();
                            sel = multi.last().copied(); selection = None; gizmo = None;
                            window.request_redraw();
                        }
                        // Ctrl+C / Ctrl+X : copier / couper les objets sélectionnés ; Ctrl+V : coller à côté de la scène
                        Key::Character(ref c) if mods.control_key() && !splash && !viewer && job.is_none() && !egui_ctx.wants_keyboard_input()
                            && ["c", "x", "v"].iter().any(|k| c.eq_ignore_ascii_case(k)) => {
                            let k = c.to_ascii_lowercase();
                            let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else { sel.filter(|&i| i < scene.len()).into_iter().collect() };
                            if (k == "c" || k == "x") && !choix.is_empty() {
                                presse_papiers = choix.iter().map(|&i| (scene[i].verts.clone(), if scene[i].has_colors() { scene[i].cols.clone() } else { Vec::new() })).collect();
                                let n = presse_papiers.len();
                                if k == "x" {
                                    let mut tous = choix.clone(); tous.sort_unstable(); tous.dedup();
                                    for &i in tous.iter().rev() { unsafe { scene[i].delete(&gl); } scene.remove(i); }
                                    sel = None; multi.clear(); selection = None; gizmo = None;
                                    defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                    history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                    if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                }
                                status_msg = Some((format!("{} : {n}", tr(lang, if k == "x" { "pp_coupe" } else { "pp_copie" })), false, std::time::Instant::now()));
                            } else if k == "v" && !presse_papiers.is_empty() {
                                // le groupe collé garde ses positions relatives et se pose à droite de la scène
                                let (mut glo, mut ghi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                                for (v, _) in &presse_papiers { for p in v.chunks_exact(6) { let q = Vec3::new(p[0], p[1], p[2]); glo = glo.min(q); ghi = ghi.max(q); } }
                                let pleine = scene.iter().any(|o| o.nverts > 0);
                                let (slo, shi) = scene_bounds(&scene);
                                let d = if pleine { Vec3::new(shi.x + ((shi - slo).length() * 0.06).max(4.0) - glo.x, 0.0, 0.0) } else { Vec3::ZERO };
                                scene.retain(|o| o.nverts > 0);
                                let debut = scene.len();
                                for (v, cols) in &presse_papiers {
                                    let mut o = unsafe { Obj::with_colors(&gl, v.clone(), cols.clone()) };
                                    if d != Vec3::ZERO { o.translate(d); unsafe { o.upload(&gl); } }
                                    scene.push(o);
                                }
                                multi = if scene.len() - debut >= 2 { (debut..scene.len()).collect() } else { Vec::new() };
                                sel = Some(scene.len() - 1); selection = None; gizmo = None;
                                let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                                orbit.target = base; orbit.dist = orbit.dist.max(r * 3.2);
                                ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];
                                defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                status_msg = Some((format!("{} : {}", tr(lang, "pp_colle"), presse_papiers.len()), false, std::time::Instant::now()));
                            }
                            window.request_redraw();
                        }
                        Key::Named(NamedKey::Delete) if txt_sel && panneau_texte => { txt_cadre = None; txt_sel = false; window.request_redraw(); }
                        Key::Named(NamedKey::Delete) if coupe_sel => { coupe = None; coupe_sel = false; window.request_redraw(); }
                        Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace) if !egui_ctx.wants_keyboard_input() && !splash && !viewer => {
                            if let Some(i) = sel.filter(|&i| i < scene.len()) {
                                piece = None; menu_piece = None;
                                // sélection multiple : tous les objets choisis sont supprimés
                                let mut tous: Vec<usize> = if multi.len() >= 2 { std::mem::take(&mut multi) } else { vec![i] };
                                tous.sort_unstable(); tous.dedup();
                                for &k in tous.iter().rev() { if k >= scene.len() { continue; } unsafe { scene[k].delete(&gl); } scene.remove(k); }
                                sel = None; selection = None; gizmo = None;
                                defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                window.request_redraw();
                            }
                        }
                        _ => {}
                    }
                }
                WindowEvent::Resized(s) => {
                    size = s;
                    if let (Some(w), Some(h)) = (NonZeroU32::new(s.width), NonZeroU32::new(s.height)) { surface.resize(&gl_context, w, h); }
                    unsafe { gl.viewport(0, 0, s.width as i32, s.height as i32); }
                    window.request_redraw();
                }
                WindowEvent::MouseInput { state, button, .. } if !egui_grab => {
                    let pressed = state == ElementState::Pressed;
                    if !pressed && button == MouseButton::Left && tex.peinture { tex.peinture = false; window.request_redraw(); }
                    if pressed {
                        if let Some(k) = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32) {
                            let rect = rect_of(mode, k).unwrap();
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            // Outil de rotation : clic gauche sur un anneau / une poignée = début de rotation
                            let cur_pts = egui::pos2(cursor.0 as f32 / sf, cursor.1 as f32 / sf);
                            let rot_hit = if button == MouseButton::Left && surf.is_none() { gizmo.as_ref().and_then(|g| g.hit(&mvp, cur_pts, rect, sf)) } else { None };
                            let (dc, dr) = dome(&scene);
                            let (ro, rd) = ray_from(&mvp, cursor, rect);
                            let sur_dome = light_mode && button == MouseButton::Left && {
                                let pres = lumiere::position_ecran(&mvp, dc, dr, lumiere_courante(), rect, sf).map_or(false, |q| (q - cur_pts).length() < 22.0);
                                let oc = ro - dc; let b = oc.dot(rd);
                                pres || b * b - (oc.length_squared() - dr * dr) >= 0.0
                            };
                            if sur_dome {
                                lumiere_regler((lumiere::point_sphere(ro, rd, dc, dr) - dc).normalize_or(lumiere::defaut()));
                                light_drag = Some(k); drag = None;
                            } else if let (Some(ax), Some(g)) = (rot_hit, gizmo.as_mut()) {
                                let (o, d) = ray_from(&mvp, cursor, rect);
                                let a = g.cursor_angle(ax, o, d, &mvp, cur_pts, rect, sf);
                                g.start(ax, a); g.hover = Some(ax);
                                rot_view = k; drag = None;
                            } else {
                            match button {
                                // MODE TEXTURE : pinceau des zones lisses (Maj = l'outil inverse)
                                MouseButton::Left if tex.actif && tex.outil.is_some() && nav_tool.is_none() && !viewer => {
                                    let cible = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 { Some(0) } else { None });
                                    match cible.and_then(|i| atelier::Atelier::viser(ro, rd, &scene[i].tris).map(|h| (i, h))) {
                                        Some((i, (t, p))) => {
                                            // SURFACE : un clic prend toute la surface (marches franchies) ; sinon pinceau rond
                                            if tex.outil == Some(atelier::Outil::Surface) { tex.surface(&scene[i].verts, t, mods.shift_key()); }
                                            else { tex.peindre(&scene[i].tris, &scene[i].verts, p, rd, mods.shift_key()); tex.peinture = true; }
                                            drag = None;
                                        }
                                        None => drag = Some(if k == ViewKind::ThreeD { Drag::Orbit(k) } else { Drag::Pan(k) }),
                                    }
                                }
                                MouseButton::Left if viewer => {
                                    let dbl = last_left.map_or(false, |(t, p)| t.elapsed().as_millis() < 400 && (p.0 - cursor.0).abs() < 6.0 && (p.1 - cursor.1).abs() < 6.0);
                                    if dbl {
                                        viewer = false; show_ruler = true; drag = None; last_left = None;
                                    } else {
                                        last_left = Some((std::time::Instant::now(), cursor));
                                        drag = Some(if k == ViewKind::ThreeD { Drag::Orbit(k) } else { Drag::Pan(k) });
                                    }
                                }
                                MouseButton::Left if panneau_texte && !splash && txt_cadre.is_some() && nav_tool.is_none() => {
                                    // TEXTE (façon 3D Builder) : flèche = profondeur et orientation, coin = taille, roue = rotation, intérieur = glisser sur la
                                    // surface ; ailleurs sur un objet = le texte s'y pose (orienté sur la face touchée)
                                    if let Some(c) = txt_cadre {
                                        let centre = c.pt(0.0, 0.0, 0.0);
                                        let sfc = window.scale_factor() as f32;
                                        let cur = egui::pos2(cursor.0 as f32 / sfc, cursor.1 as f32 / sfc);
                                        let sur_prof = poignee_prof(&c, &mvp, rect, sfc, txt_relief).map_or(false, |(_, tip)| (tip - cur).length() < 10.0);
                                        if sur_prof {
                                            // flèche bleue : sa longueur = profondeur, sa direction = orientation du texte ; elle glisse
                                            // dans le plan face à la vue passant par sa pointe
                                            txt_sel = true; txt_k = k;
                                            if let Some(l) = long_fleche(&c, &mvp, rect, 1.0) { txt_prof_glisse = Some((c, l, ray_from(&mvp, cursor, rect).1.normalize())); }
                                        } else if coin_sous(&c, k, &mvp, rect, cursor, sfc) {
                                            // taille de départ : police du texte, ou largeur du dessin (INSÈRE IMAGE)
                                            txt_sel = true; txt_k = k;
                                            if let Some(p) = plan_h(&mvp, cursor, rect, c.n, c.h) { txt_echelle = Some((if txt_image { img_largeur } else { txt_taille }, (p - centre).length().max(1e-3))); }
                                        } else if poignee_sous(&c, k, &mvp, rect, cursor, sfc) {
                                            txt_sel = true; txt_k = k;
                                            if let Some(p) = plan_h(&mvp, cursor, rect, c.n, c.h) { let d = p - centre; txt_rot = Some((c.angle, d.dot(c.v).atan2(d.dot(c.u)))); }
                                        } else if dans_cadre(&c, &mvp, rect, cursor) {
                                            txt_sel = true; txt_k = k;
                                            // vue 3D : sur l'objet ; vue à plat : dans le plan de la VUE, même si le cadre y est vu par la tranche
                                            let pv = if k == ViewKind::ThreeD { plan_h(&mvp, cursor, rect, c.n, c.h) } else {
                                                let (rr, uu) = Ortho::right_up(k); let nv = rr.cross(uu);
                                                plan_h(&mvp, cursor, rect, nv, c.centre.dot(nv))
                                            };
                                            txt_deplace = pv.map(|q| c.centre - q);
                                        } else if let Some((_, hit, nrm)) = pick_scene(&mvp, cursor, rect, &scene) {
                                            txt_cadre = Some(cadre_sur(hit, nrm, Some(&c))); txt_sel = true; txt_k = k;
                                        } else { txt_sel = false; }
                                    }
                                    drag = None;
                                }
                                MouseButton::Left if surf.is_some() && !splash => {
                                    // SURFACE : clic sur une face rouge = l'objet se pose dessus (avec la chute)
                                    if let (Some(fi), Some((i, faces))) = (surf_survol.or_else(|| face_sous(&mvp, cursor, rect, &scene, surf.as_ref().unwrap())), surf.as_ref()) {
                                        let (i, n) = (*i, faces[fi].n);
                                        if i < scene.len() { grav_anim = Some(GravAnim::vers(i, &scene[i], n)); selection = None; }
                                        surf = None; surf_survol = None;
                                    }
                                    drag = None;
                                }
                                MouseButton::Left if coupe_arme && !splash => {
                                    // ciseaux, par clics : 1 = départ du trait, 2 = arrivée (L fixé), 3 = l déplié, fini
                                    match coupe_etape {
                                        0 => {
                                            let e = axe_coupe(k);
                                            if let Some(p) = plane_point(&mvp, cursor, rect, e) {
                                                coupe = Some(booleen::Cadre::new(p, p, e)); coupe_tir = Some(k); coupe_etape = 1;
                                            }
                                        }
                                        1 => if coupe.map_or(false, |c| c.reperes().map_or(false, |(_, _, l)| l >= 0.5)) { coupe_etape = 2; },
                                        _ => {
                                            // l non déplié : cadre perpendiculaire sur toute la hauteur
                                            if let Some(c) = coupe.as_mut() { if c.l < 0.5 { c.l = 0.0; c.incl = 90.0; } }
                                            coupe_etape = 0; coupe_tir = None; coupe_arme = false;
                                        }
                                    }
                                    drag = None;
                                }
                                MouseButton::Left if nav_tool.is_some() => {
                                    // Un outil de navigation de l'overlay est actif : le clic gauche navigue.
                                    match nav_tool.unwrap() {
                                        NavTool::Rotate => drag = Some(if k == ViewKind::ThreeD { Drag::Orbit(k) } else { Drag::Pan(k) }),
                                        NavTool::Pan    => drag = Some(Drag::Pan(k)),
                                        NavTool::Zoom   => drag = Some(Drag::Zoom(k)),
                                        NavTool::Zone   => { zone = Some((k, cursor, cursor)); drag = None; }
                                    }
                                }
                                MouseButton::Left => {
                                    txt_sel = false;
                                    if draw_tool.is_some() && (k == ViewKind::Top || k == ViewKind::ThreeD) {
                                        // Démarre le tracé sur la grille (plan Z=0).
                                        if let Some(p) = ground_point(&mvp, cursor, rect) {
                                            draw_start = Some(p); draw_cur = Some(p); sketch_view = Some(k);
                                            sel = None; selection = None; drag = None;
                                        }
                                    } else {
                                        // Clic gauche = sélectionner l'objet sous le curseur ; s'il y en a un,
                                        // le clic maintenu le déplace (mode Move). Sinon désélection.
                                        // logo d'accueil : ni sélection ni déplacement (clic gauche = rien)
                                        let ajout = mods.control_key() || mods.shift_key();
                                        // le cadre de coupe se sélectionne comme un objet (il passe devant : il est dessiné par-dessus)
                                        let sur_cadre = !splash && coupe.map_or(false, |c| coupe_touche(&c, &scene, &mvp, cursor, rect));
                                        coupe_sel = false;
                                        match if splash || sur_cadre { None } else { pick_scene(&mvp, cursor, rect, &scene) } {
                                            // Ctrl+clic (ou Maj+clic) : ajoute / retire l'objet de la sélection (comme 3D Builder)
                                            Some((i, hit, _)) if ajout => {
                                                gizmo = None;
                                                if multi.len() < 2 { multi = sel.filter(|&s| s < scene.len()).into_iter().collect(); }
                                                if let Some(p) = multi.iter().position(|&k| k == i) {
                                                    multi.remove(p);
                                                    sel = multi.last().copied(); selection = None; drag = None;
                                                } else {
                                                    multi.push(i);
                                                    sel = Some(i); selection = Some(hit);
                                                    drag = Some(Drag::Move(k));
                                                    last_ground = ground_point(&mvp, cursor, rect);
                                                }
                                                if multi.len() < 2 { multi.clear(); }
                                            }
                                            None if ajout => { drag = None; }
                                            Some((i, hit, nrm)) => {
                                                // clic simple sur un objet de la sélection multiple : on la garde, dans le
                                                // même ordre (pour déplacer le groupe sans changer l'outil de SOUSTRAIRE)
                                                if !multi.contains(&i) { multi.clear(); }
                                                if gizmo.as_ref().map_or(false, |g| g.obj != i) { gizmo = None; }
                                                sel = Some(i); selection = Some(hit); face_normal = Some(nrm);
                                                // objet fait de plusieurs pièces : la pièce touchée est choisie (clic droit = Dégrouper)
                                                piece = None; menu_piece = None;
                                                let sig = (scene[i].min, scene[i].max, scene[i].verts.len());
                                                let (comp, n) = pieces(&scene[i].verts);
                                                if n >= 2 { if let Some(t) = tri_vise(&mvp, cursor, rect, &scene[i]) {
                                                    status_msg = Some((tr(lang, "piece_sel").replace("{k}", &(comp[t] + 1).to_string()).replace("{n}", &n.to_string()), false, std::time::Instant::now()));
                                                    piece = Some((i, comp[t], comp, n, sig));
                                                } }
                                                drag = Some(Drag::Move(k));
                                                last_ground = ground_point(&mvp, cursor, rect);
                                            }
                                            None if panneau_texte => { drag = None; } // l'objet à graver reste sélectionné
                                            None => { sel = None; multi.clear(); selection = None; face_normal = None; drag = None; gizmo = None; piece = None; menu_piece = None; }
                                        }
                                        if sur_cadre {
                                            // cadre choisi : on peut le faire glisser dans son plan de tracé
                                            coupe_sel = true; drag = None;
                                            let e = coupe.unwrap().e;
                                            coupe_glisse = plane_point(&mvp, cursor, rect, e).map(|p| (k, p));
                                        }
                                    }
                                }
                                MouseButton::Right => {
                                    // double clic droit : sur l'objet = outil de rotation ; dans le vide = le fermer
                                    let dbl = last_right.map_or(false, |(t, p)| t.elapsed().as_millis() < 450 && (p.0 - cursor.0).abs() < 8.0 && (p.1 - cursor.1).abs() < 8.0);
                                    if dbl && !splash {
                                        match pick_scene(&mvp, cursor, rect, &scene) {
                                            Some((i, _, _)) => { gizmo = Some(gizmo::Gizmo::new(i, scene[i].min, scene[i].max)); sel = Some(i); light_mode = false; }
                                            None => gizmo = None,
                                        }
                                        last_right = None; drag = None; clic_droit = None; menu_piece = None;
                                    } else {
                                        last_right = Some((std::time::Instant::now(), cursor));
                                        clic_droit = Some((std::time::Instant::now(), cursor));
                                        drag = Some(if k == ViewKind::ThreeD { Drag::Orbit(k) } else { Drag::Pan(k) });
                                    }
                                }
                                MouseButton::Middle => {
                                    let dbl = last_mid.map_or(false, |t| t.elapsed().as_millis() < 400);
                                    if dbl {
                                        if let Some((_, hit, _)) = pick_scene(&mvp, cursor, rect, &scene) {
                                            if k == ViewKind::ThreeD { orbit.target = hit; }
                                            else { let (rt, up) = Ortho::right_up(k); let o = &mut ortho[view::oidx(k)]; o.pan = Vec2::new((hit - base).dot(rt), (hit - base).dot(up)); }
                                        }
                                        last_mid = None;
                                    } else { last_mid = Some(std::time::Instant::now()); drag = Some(Drag::Pan(k)); }
                                }
                                _ => {}
                            }
                            }
                            window.request_redraw();
                        }
                    } else {
                        // clic droit simple (sans glisser) sur un objet à plusieurs pièces : menu Dégrouper
                        if button == MouseButton::Right {
                            if let Some((t0, p0)) = clic_droit.take() {
                                let immobile = (p0.0 - cursor.0).abs() < 5.0 && (p0.1 - cursor.1).abs() < 5.0;
                                if immobile && t0.elapsed().as_millis() < 400 && !splash && !viewer {
                                    if let Some(k) = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32) {
                                        let rect = rect_of(mode, k).unwrap();
                                        let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                                        // MODE TEXTURE : clic droit = annuler la dernière action sur les zones lisses (dans l'ordre)
                                        if tex.actif {
                                            tex.annuler_exclusion();
                                        } else if let Some((i, _, _)) = pick_scene(&mvp, cursor, rect, &scene) {
                                            let pos = egui::pos2(cursor.0 as f32 / sf, cursor.1 as f32 / sf);
                                            // sélection multiple (Maj / Ctrl+clic) contenant l'objet : menu Grouper
                                            let groupe: Vec<usize> = if multi.len() >= 2 && multi.contains(&i) { multi.clone() } else { Vec::new() };
                                            let (comp, n) = pieces(&scene[i].verts);
                                            let t = tri_vise(&mvp, cursor, rect, &scene[i]);
                                            if n >= 2 && groupe.is_empty() { if let Some(t) = t {
                                                let sig = (scene[i].min, scene[i].max, scene[i].verts.len());
                                                sel = Some(i); multi.clear();
                                                menu_piece = Some((pos, i, comp[t], n, Vec::new()));
                                                piece = Some((i, comp[t], comp, n, sig));
                                            } } else if !groupe.is_empty() {
                                                menu_piece = Some((pos, i, 0, 1, groupe));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if button == MouseButton::Left { light_drag = None; }
                        if button == MouseButton::Left { coupe_glisse = None; txt_rot = None; txt_deplace = None; txt_echelle = None; txt_prof_glisse = None; }
                        // fin de rotation : la rotation est cuite dans les sommets (l'objet garde son appui en Z)
                        if button == MouseButton::Left {
                            if let Some(g) = gizmo.as_mut().filter(|g| g.drag.is_some()) {
                                if let (Some(m3), true) = (g.rot3(), g.obj < scene.len()) {
                                    let ang = g.angle();
                                    if ang.abs() > 1e-5 {
                                        rotate_obj(&mut scene[g.obj], g.center, m3);
                                        unsafe { scene[g.obj].upload(&gl); }
                                        selection = None;
                                        defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                        history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                        if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                    }
                                }
                                g.drag = None;
                                window.request_redraw();
                            }
                        }
                        if matches!(drag, Some(Drag::Move(_))) {
                            // fin de déplacement : recalcul des défauts de l'objet actif + instantané historique
                            defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                            history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                            if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                        }
                        // Fin d'un zoom "Zone" : on cadre la vue sur le rectangle tracé.
                        if let Some((zk, s, c)) = zone.take() {
                            if let Some((rx, ry, rw, rh)) = rect_of(mode, zk) {
                                let (bw, bh) = ((s.0 - c.0).abs() as f32, (s.1 - c.1).abs() as f32);
                                if bw > 6.0 && bh > 6.0 {
                                    let ratio = (bw / rw).max(bh / rh).clamp(0.02, 1.0);
                                    let mid = (((s.0 + c.0) * 0.5) as f32, ((s.1 + c.1) * 0.5) as f32);
                                    if zk == ViewKind::ThreeD {
                                        let (mvp, _) = view_mvp(zk, &orbit, &ortho, base, aspect_of(mode, zk));
                                        if let Some(p) = ground_point(&mvp, (mid.0 as f64, mid.1 as f64), (rx, ry, rw, rh)) { orbit.target = p; }
                                        orbit.dist = (orbit.dist * ratio).clamp(0.05, 50000.0);
                                    } else {
                                        let o = &mut ortho[view::oidx(zk)];
                                        let wpp = 2.0 * o.scale / rh.max(1.0);
                                        let (cx, cy) = (rx + rw * 0.5, ry + rh * 0.5);
                                        o.pan.x += (mid.0 - cx) * wpp;
                                        o.pan.y -= (mid.1 - cy) * wpp;
                                        o.scale = (o.scale * ratio).clamp(0.05, 50000.0);
                                    }
                                }
                            }
                            window.request_redraw();
                        }
                        // Fin de tracé (Courbes) : crée la forme dessinée comme nouvel objet.
                        if let (Some(shape), Some(p0), Some(p1)) = (draw_tool, draw_start, draw_cur) {
                            let snap = |p: Vec3| if snap_direct { p } else {
                                Vec3::new((p.x / grid_step).round() * grid_step, (p.y / grid_step).round() * grid_step, 0.0) };
                            let (a, b) = (snap(p0), snap(p1));
                            let thick = (grid_step * 0.5).clamp(0.5, 4.0); // épaisseur de la « tôle » tracée
                            let verts: Vec<f32> = match shape {
                                DrawShape::Line => bar_mesh(a, b, thick, thick),
                                DrawShape::Rect => {
                                    let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
                                    if dx > 1e-3 && dy > 1e-3 {
                                        match occt::primitive(occt::Prim::Box { dx: dx as f64, dy: dy as f64, dz: thick as f64 }, occt::DEFLEXION) {
                                            Ok(mut m) => { let off = Vec3::new(a.x.min(b.x), a.y.min(b.y), 0.0);
                                                for c in m.verts.chunks_exact_mut(6) { c[0]+=off.x; c[1]+=off.y; c[2]+=off.z; } m.verts }
                                            Err(_) => Vec::new() }
                                    } else { Vec::new() }
                                }
                                DrawShape::Circle => {
                                    let rad = (b - a).length();
                                    if rad > 1e-3 {
                                        match occt::primitive(occt::Prim::Cylinder { r: rad as f64, h: thick as f64 }, occt::DEFLEXION) {
                                            Ok(mut m) => { for c in m.verts.chunks_exact_mut(6) { c[0]+=a.x; c[1]+=a.y; } m.verts }
                                            Err(_) => Vec::new() }
                                    } else { Vec::new() }
                                }
                            };
                            if verts.len() >= 18 {
                                scene.retain(|o| o.nverts > 0);
                                scene.push(unsafe { Obj::new(&gl, verts) });
                                sel = Some(scene.len() - 1); selection = None;
                                defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                            }
                            draw_start = None; draw_cur = None; sketch_view = None;
                        }
                        drag = None; last = None; last_ground = None;
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    cursor = (position.x, position.y);
                    // MODE TEXTURE : coup de pinceau en glissant, cercle du pinceau sous le curseur
                    if tex.actif && tex.outil.is_some() {
                        let sf = window.scale_factor() as f32;
                        let mut c = None;
                        if let Some(k) = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32) {
                            let rect = rect_of(mode, k).unwrap();
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            let (ro, rd) = ray_from(&mvp, cursor, rect);
                            if let Some(i) = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 { Some(0) } else { None }) {
                                if let Some((_, p)) = atelier::Atelier::viser(ro, rd, &scene[i].tris) {
                                    if tex.peinture { tex.peindre(&scene[i].tris, &scene[i].verts, p, rd, mods.shift_key()); }
                                    if let Some(rp) = tex.rayon_pinceau() {
                                        // rayon à l'écran : projection d'un point du cercle perpendiculaire au regard
                                        let side = rd.cross(Vec3::Z).normalize_or(Vec3::X);
                                        if let (Some(a), Some(b)) = (project(&mvp, p, rect, sf), project(&mvp, p + side * rp, rect, sf)) { c = Some((a, (b - a).length())); }
                                    }
                                }
                            }
                        }
                        if c != tx_curseur || tex.peinture { tx_curseur = c; window.request_redraw(); }
                    } else { tx_curseur = None; }
                    // Projecteur : glisser sur le dôme
                    if let Some(k) = light_drag {
                        if let Some(rect) = rect_of(mode, k) {
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            let (dc, dr) = dome(&scene);
                            let (ro, rd) = ray_from(&mvp, cursor, rect);
                            lumiere_regler((lumiere::point_sphere(ro, rd, dc, dr) - dc).normalize_or(lumiere::defaut()));
                            window.request_redraw();
                        }
                    }
                    // Outil de rotation : rotation en cours, ou survol des anneaux
                    if let Some(g) = gizmo.as_mut() {
                        let cur_pts = egui::pos2(cursor.0 as f32 / sf, cursor.1 as f32 / sf);
                        if let Some(ax) = g.drag.as_ref().map(|d| d.axis) {
                            if let Some(rect) = rect_of(mode, rot_view) {
                                let (mvp, _) = view_mvp(rot_view, &orbit, &ortho, base, aspect_of(mode, rot_view));
                                let (o, d) = ray_from(&mvp, cursor, rect);
                                let a = g.cursor_angle(ax, o, d, &mvp, cur_pts, rect, sf);
                                g.update(a);
                                window.request_redraw();
                            }
                        } else if drag.is_none() {
                            let h = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32).and_then(|k| {
                                let rect = rect_of(mode, k)?;
                                let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                                g.hit(&mvp, cur_pts, rect, sf)
                            });
                            if h != g.hover { g.hover = h; window.request_redraw(); }
                        }
                    }
                    // SURFACE : face rouge sous la souris (surlignée)
                    if let Some(sf_) = surf.as_ref() {
                        let h = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32).and_then(|k| {
                            let rect = rect_of(mode, k)?;
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            face_sous(&mvp, cursor, rect, &scene, sf_)
                        });
                        if h != surf_survol { surf_survol = h; window.request_redraw(); }
                    }
                    // TEXTE : déplacement sur la surface, rotation par les roues, taille par les coins, profondeur / orientation par la flèche
                    if panneau_texte && (txt_rot.is_some() || txt_deplace.is_some() || txt_echelle.is_some() || txt_prof_glisse.is_some()) {
                        // la souris est lue dans la fenêtre où elle se trouve (on peut changer de vue en cours de tracé)
                        let kc = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32).unwrap_or(txt_k);
                        if let (Some(rect), Some(c)) = (rect_of(mode, kc), txt_cadre.as_mut()) {
                            let (mvp, _) = view_mvp(kc, &orbit, &ortho, base, aspect_of(mode, kc));
                            let mut prof_axe = false;
                            // flèche bleue : pointe = point visé dans le plan face à la vue ; direction = normale du texte
                            // (aimantée à 0° / 45° de la normale de départ), longueur = profondeur ; côté = relief / gravé
                            if let Some((c0, l0, dv)) = txt_prof_glisse {
                                let o = c0.pt(0.0, 0.0, 0.0);
                                let sg0 = if txt_relief { 1.0 } else { -1.0 };
                                let tip0 = o + c0.n * (sg0 * l0);
                                if let Some(p) = plan_h(&mvp, cursor, rect, dv, tip0.dot(dv)) {
                                    let w = p - o;
                                    let l = w.length();
                                    if l > 1e-3 {
                                        let cote = if w.dot(c0.n) >= 0.0 { 1.0 } else { -1.0 };
                                        let mut n = w / l * cote; // toujours vers l'extérieur
                                        let th = n.dot(c0.n).clamp(-1.0, 1.0).acos().to_degrees();
                                        let cran = aimante_angle(th);
                                        if (cran - th).abs() > 1e-4 {
                                            let ax = c0.n.cross(n).normalize_or_zero();
                                            n = if ax == Vec3::ZERO || cran == 0.0 { c0.n } else { glam::Mat3::from_axis_angle(ax, cran.to_radians()) * c0.n };
                                        }
                                        let q = glam::Quat::from_rotation_arc(c0.n, n);
                                        c.u = q * c0.u; c.v = q * c0.v; c.n = n; c.centre = o; c.h = o.dot(n);
                                        txt_relief = cote > 0.0;
                                        c.prof = (c0.prof + (l - l0)).max(0.1);
                                    }
                                }
                                prof_axe = true;
                            }
                            // déplacement au contact de l'objet : le cadre glisse sur la surface sous la souris ;
                            // sur une autre face (vue 3D), il se réoriente sur cette face
                            let mut colle = false;
                            // vue à plat : déplacement dans le plan de la vue (directions des flèches bleues)
                            if txt_rot.is_none() && kc != ViewKind::ThreeD {
                                if let Some(off) = txt_deplace {
                                    let (rr, uu) = Ortho::right_up(kc); let nv = rr.cross(uu);
                                    if let Some(q) = plan_h(&mvp, cursor, rect, nv, c.centre.dot(nv)) {
                                        c.centre = q + off;
                                        // vue qui regarde la face du cadre : il reste collé à la surface ; sinon (haut/bas
                                        // dans une vue de côté) la hauteur suit la souris
                                        c.h = if nv.dot(c.n).abs() > 0.9 { surface_sous(&scene, c.centre, c.n).map(|x| x.1).unwrap_or(c.centre.dot(c.n)) } else { c.centre.dot(c.n) };
                                    }
                                    colle = true;
                                }
                            }
                            if txt_rot.is_none() && kc == ViewKind::ThreeD {
                                if let (Some(off), Some((_, hit, nrm))) = (txt_deplace, pick_scene(&mvp, cursor, rect, &scene)) {
                                    let nrm = nrm.normalize_or(c.n);
                                    if kc == ViewKind::ThreeD && nrm.dot(c.n) < 0.95 {
                                        let haut = if nrm.z.abs() < 0.9 { Vec3::Z } else { Vec3::Y };
                                        c.n = nrm; c.u = haut.cross(nrm).normalize_or(Vec3::X); c.v = c.n.cross(c.u);
                                        c.centre = hit; txt_deplace = Some(Vec3::ZERO);
                                    } else {
                                        let o2 = off - c.n * off.dot(c.n);
                                        c.centre = hit + o2;
                                    }
                                    c.h = hit.dot(c.n);
                                    colle = true;
                                }
                            }
                            if prof_axe || colle {} else if let Some(p) = plan_h(&mvp, cursor, rect, c.n, c.h) {
                                match () {
                                    _ if txt_echelle.is_some() => if let Some((t0, d0)) = txt_echelle {
                                        let t = (t0 * (p - c.pt(0.0, 0.0, 0.0)).length() / d0).clamp(0.5, 2000.0);
                                        if txt_image { img_largeur = t; } else { txt_taille = t; }
                                    },
                                    _ => if let Some((a0, c0)) = txt_rot {
                                        let q = p - c.pt(0.0, 0.0, 0.0);
                                        let a = a0 + (q.dot(c.v).atan2(q.dot(c.u)) - c0);
                                        // aimant de la roue : à moins de 5° d'un multiple de 45°, l'angle s'y colle
                                        c.angle = aimante_angle(a.to_degrees().rem_euclid(360.0)).to_radians();
                                    }
                                         else if let Some(off) = txt_deplace { c.centre = p + off; },
                                }
                            }
                            window.request_redraw();
                        }
                    }
                    // Cadre de coupe déplacé à la souris (dans son plan de tracé).
                    if let (Some((k, last_p)), Some(c)) = (coupe_glisse, coupe.as_mut()) {
                        if let Some(rect) = rect_of(mode, k) {
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            if let Some(p) = plane_point(&mvp, cursor, rect, c.e) { let d = p - last_p; c.p0 += d; c.p1 += d; coupe_glisse = Some((k, p)); }
                            window.request_redraw();
                        }
                    }
                    // Découpe en cours : le trait L suit la souris dans sa vue (étape 1)…
                    if let (Some(k), Some(c), 1) = (coupe_tir, coupe.as_mut(), coupe_etape) {
                        if let Some(rect) = rect_of(mode, k) {
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            if let Some(p) = plane_point(&mvp, cursor, rect, c.e) { c.p1 = aimante_45(c.p0, p, c.e); }
                            window.request_redraw();
                        }
                    }
                    // … puis l se déplie en pivotant autour de L (étape 2), dans la vue sous la souris : point visé
                    // dans le plan perpendiculaire au trait passant par son milieu (vue où ce plan est de face ou de biais)
                    if let (Some(c), 2) = (coupe.as_mut(), coupe_etape) {
                        let kc = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32);
                        if let (Some(k), Some((u, _, l)), Some(np)) = (kc, c.reperes(), c.perp_plan()) {
                            if let Some(rect) = rect_of(mode, k) {
                                let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                                let (o, d) = ray_from(&mvp, cursor, rect);
                                let mil = c.origine() + u * (l * 0.5);
                                if d.dot(u).abs() > 0.15 {
                                    let v = o + d * ((mil - o).dot(u) / d.dot(u)) - mil;
                                    let v = v - u * v.dot(u);
                                    c.l = v.length();
                                    c.incl = aimante_angle(v.dot(c.axe()).atan2(v.dot(np)).to_degrees());
                                }
                                window.request_redraw();
                            }
                        }
                    }
                    // Mise à jour du tracé en cours (aperçu rectangle/cercle/ligne).
                    if draw_start.is_some() {
                        if let Some(k) = sketch_view {
                            if let Some(rect) = rect_of(mode, k) {
                                let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                                if let Some(p) = ground_point(&mvp, cursor, rect) { draw_cur = Some(p); }
                                window.request_redraw();
                            }
                        }
                    }
                    // Mise à jour du rectangle de zoom "Zone" en cours.
                    if let Some((zk, s, _)) = zone {
                        zone = Some((zk, s, cursor));
                        window.request_redraw();
                    }
                    if let Some(dr) = drag {
                        let k = match dr { Drag::Orbit(k) | Drag::Pan(k) | Drag::Move(k) | Drag::Zoom(k) => k };
                        if let Some(rect) = rect_of(mode, k) {
                            let vh = rect.3;
                            let (dx, dy) = if let Some((lx, ly)) = last { ((position.x - lx) as f32, (position.y - ly) as f32) } else { (0.0, 0.0) };
                            match dr {
                                Drag::Orbit(_) => {
                                    orbit.yaw -= dx * 0.01;
                                    orbit.pitch = (orbit.pitch + dy * 0.01).clamp(-1.54, 1.54);
                                }
                                Drag::Pan(kk) => {
                                    if kk == ViewKind::ThreeD {
                                        let fwd = (orbit.target - orbit.eye()).normalize();
                                        let right = fwd.cross(Vec3::Z).normalize_or_zero();
                                        let up = right.cross(fwd).normalize_or_zero();
                                        let s = orbit.dist * 0.0015;
                                        orbit.target += (-right * dx + up * dy) * s;
                                    } else {
                                        let o = &mut ortho[view::oidx(kk)];
                                        let wpp = 2.0 * o.scale / vh.max(1.0);
                                        o.pan.x -= dx * wpp; o.pan.y += dy * wpp;
                                    }
                                }
                                Drag::Move(kk) => {
                                    if let Some(idx) = sel {
                                        let (mvp, _) = view_mvp(kk, &orbit, &ortho, base, aspect_of(mode, kk));
                                        let d = if kk == ViewKind::ThreeD {
                                            match (last_ground, ground_point(&mvp, cursor, rect)) {
                                                (Some(a), Some(b)) => { last_ground = Some(b); b - a }
                                                (_, g) => { last_ground = g; Vec3::ZERO }
                                            }
                                        } else {
                                            let (right, up) = Ortho::right_up(kk);
                                            let wpp = 2.0 * ortho[view::oidx(kk)].scale / vh.max(1.0);
                                            right * (dx * wpp) - up * (dy * wpp)
                                        };
                                        if d != Vec3::ZERO {
                                            // sélection multiple : tout le groupe suit
                                            let groupe: Vec<usize> = if multi.contains(&idx) { multi.clone() } else { vec![idx] };
                                            for &g in groupe.iter() {
                                                if g >= scene.len() { continue; }
                                                scene[g].translate(d);
                                                unsafe { scene[g].upload(&gl); }
                                            }
                                            if let Some(s) = selection.as_mut() { *s += d; }
                                        }
                                    }
                                }
                                Drag::Zoom(kk) => {
                                    // Glisser vers le bas = dézoomer, vers le haut = zoomer.
                                    let f = (1.0 + dy * 0.006).clamp(0.5, 2.0);
                                    if kk == ViewKind::ThreeD { orbit.dist = (orbit.dist * f).clamp(0.05, 50000.0); }
                                    else { let o = &mut ortho[view::oidx(kk)]; o.scale = (o.scale * f).clamp(0.05, 50000.0); }
                                }
                            }
                            window.request_redraw();
                        }
                        last = Some((position.x, position.y));
                    }
                }
                WindowEvent::MouseWheel { delta, .. } if !egui_grab => {
                    let d = match delta { MouseScrollDelta::LineDelta(_, y) => y, MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.02 };
                    if let Some(k) = view::view_at(mode, wf, hf, bar_px, side_px, cursor.0 as f32, cursor.1 as f32) {
                        if k == ViewKind::ThreeD { orbit.dist = (orbit.dist * (1.0 - d * 0.1)).clamp(0.05, 50000.0); }
                        else { let o = &mut ortho[view::oidx(k)]; o.scale = (o.scale * (1.0 - d * 0.1)).clamp(0.05, 50000.0); }
                        window.request_redraw();
                    }
                }
                WindowEvent::RedrawRequested => {
                    langue_regler(lang);
                    VISIONNEUSE.store(viewer, std::sync::atomic::Ordering::Relaxed);
                    if let Some((g, rx)) = &gpu { unsafe { g.serve(&gl, rx); } }
                    let mut cur_mode = mode;
                    // outil de rotation : objet disparu => fermé ; sinon recalé sur l'objet (hors geste)
                    // grille et règles toujours à la taille de la scène (import, redimensionnement, déplacement,
                    // annulation, changement d'unité…)
                    let dims = dims_grille(unit, &scene, r);
                    if dims != grille_dims {
                        unsafe { grid.delete(&gl); rule.delete(&gl); }
                        let (g, ru, mj) = unsafe { build_grid_ruler(&gl, dims) };
                        grid = g; rule = ru; ruler_majors = mj; grille_dims = dims;
                    }
                    grid_step = grille_dims.0;
                    if gizmo.as_ref().map_or(false, |g| g.obj >= scene.len() || scene[g.obj].nverts == 0) { gizmo = None; }
                    if let Some(g) = gizmo.as_mut() { if g.drag.is_none() { g.fit(scene[g.obj].min, scene[g.obj].max); } }
                    if let Some((ax, deg, cuire)) = rot_test {
                        if egui_ctx.frame_nr() >= 3 && !scene.is_empty() {
                            let i = sel.filter(|&i| i < scene.len()).unwrap_or(0);
                            let mut g = gizmo::Gizmo::new(i, scene[i].min, scene[i].max);
                            g.hover = Some(ax);
                            if deg.is_finite() { g.start(ax, 0.0); g.update(deg.to_radians().clamp(-3.1, 3.1)); if deg.abs() > 178.0 { if let Some(d) = g.drag.as_mut() { d.angle = deg.to_radians(); } } }
                            if cuire {
                                if let Some(m3) = g.rot3() { rotate_obj(&mut scene[i], g.center, m3); unsafe { scene[i].upload(&gl); } }
                                g.drag = None; g.fit(scene[i].min, scene[i].max);
                                let e = scene[i].max - scene[i].min;
                                println!("ROT cuite : boîte {:.2} x {:.2} x {:.2}, z min {:.3}", e.x, e.y, e.z, scene[i].min.z);
                            }
                            gizmo = Some(g); rot_test = None;
                        }
                    }
                    // tout ce qui compose la scène 3D : si rien n'a bougé, on réaffiche l'image gardée.
                    // Recalculée en fin d'image : si l'interface a changé la scène après le dessin, on redessine.
                    // sélection multiple : objets disparus oubliés ; une sélection simple ailleurs l'annule
                    multi.retain(|&i| i < scene.len() && scene[i].nverts > 0);
                    if multi.len() < 2 || !sel.map_or(false, |s| multi.contains(&s)) { multi.clear(); }
                    // clé commune (tout sauf les caméras) + caméra de chaque vue : une vue n'est redessinée que si
                    // la clé commune ou SA caméra change
                    macro_rules! cle_commune { () => { format!("{} {} {} {} {} {} {} {:?} {wf} {hf} {bar_px} {side_px} {:?} {:?} {:?} {multi:?} {coupe:?} {coupe_sel} {coupe_etape} {:?} {surf_survol:?} {surf_liste:?} {panneau_imp} {imp_voir_s} {imp_voir_f} {}",
                        cache3d::generation(), code_de_vue(mode), disp as u8, dark, show_ruler, show_grid, axes_visibles(),
                        sel, base, lumiere_courante(),
                        gizmo.as_ref().and_then(|g| g.matrix().map(|m| (g.obj, m))), surf.as_ref().map(|s| (s.0, s.1.len())), imp_an.0) } }
                    macro_rules! cles_vues { () => { view::ALL_VIEWS.map(|k| if k == ViewKind::ThreeD { format!("{} {} {} {:?}", orbit.yaw, orbit.pitch, orbit.dist, orbit.target) }
                                                                           else { let o = &ortho[view::oidx(k)]; format!("{:?} {}", o.pan, o.scale) }) } }
                    macro_rules! cle_scene { () => { format!("{} {:?}", cle_commune!(), cles_vues!()) } }
                    let cle_dessinee = cle_scene!();
                    // outils et panneaux ouverts (ce qui change les tracés par-dessus les vues sans toucher à la 3D) :
                    // tout changement pendant l'image, au début ou dans l'interface, redemande une image tout de suite
                    macro_rules! cle_outils { () => { format!("{:?} {:?}", (panneau_bool, panneau_texte, txt_image, panneau_forme, panneau_imp, light_mode, draw_tool.is_some(), tex.actif), (
                        gizmo.is_some(), coupe_arme, coupe_etape, viewer, code_de_vue(mode), txt_cadre.is_some(), txt_sel, coupe.is_some())) } }
                    let outils_avant = cle_outils!();
                    *SEL_PLUS.lock().unwrap() = multi.clone();
                    // fin d'un booléen / d'une découpe, ou panneau refermé : outils désarmés, cadre retiré,
                    // boîte à outils normale et vue d'avant la découpe
                    if fin_outil { panneau_bool = false; fin_outil = false; }
                    // une seule fonction ouverte : celle qui vient de s'ouvrir ferme toutes les autres
                    {
                        let ouverts = [gizmo.is_some(), panneau_bool, panneau_forme, panneau_imp, light_mode, draw_tool.is_some(), tex.actif];
                        if let Some(k) = (0..7).find(|&k| ouverts[k] && !modes_prec[k]) {
                            if k != 0 { gizmo = None; surf = None; surf_survol = None; }
                            if k != 1 { panneau_bool = false; }
                            if k != 2 { panneau_forme = false; }
                            if k != 3 { panneau_imp = false; }
                            if k != 4 { light_mode = false; }
                            if k != 5 { draw_tool = None; }
                            if k != 6 { tex.fermer(); }
                        }
                        modes_prec = [gizmo.is_some(), panneau_bool, panneau_forme, panneau_imp, light_mode, draw_tool.is_some(), tex.actif];
                    }
                    // ouverture du panneau Booléens : passage en 4 vues ; à sa sortie, retour à la vue 3D standard
                    if panneau_bool && !panneau_prec && mode != ViewMode::Quad { mode_avant_coupe = Some(mode); mode = ViewMode::Quad; }
                    if !panneau_bool && panneau_prec { mode_avant_coupe = None; mode = ViewMode::Single(ViewKind::ThreeD); }
                    panneau_prec = panneau_bool;
                    if !panneau_bool { panneau_texte = false; }
                    if !panneau_texte { txt_rot = None; txt_echelle = None; txt_prof_glisse = None; }
                    if !panneau_bool {
                        coupe_arme = false; coupe = None; coupe_tir = None; coupe_etape = 0; coupe_glisse = None;
                        if let Some(m) = mode_avant_coupe.take() { mode = m; }
                    }
                    if coupe.is_none() { coupe_sel = false; }
                    // disposition des vues à jour pour toute l'image (dessin 3D, cache, interface) : sinon les vues
                    // seraient dessinées dans l'ancienne disposition et gardées en cache sous la clé de la nouvelle
                    cur_mode = mode;
                    // marqueur du trou : seulement tant que l'objet creusé reste sélectionné et inchangé
                    if trou_marque.as_ref().map_or(false, |(i, m, _)| sel != Some(*i) || *i >= scene.len() || scene[*i].min != *m) { trou_marque = None; }
                    // PRÉPARATION À L'IMPRESSION : trancheurs installés (une fois), analyse de la pièce
                    // (relancée quand la pièce, la sélection ou les réglages changent), choix d'un programme
                    if panneau_imp {
                        if imp_installes.is_none() && imp_detect.is_none() { imp_detect = Some(std::thread::spawn(impression::detecter)); }
                        if imp_detect.as_ref().map_or(false, |h| h.is_finished()) {
                            let l = imp_detect.take().unwrap().join().unwrap_or_default();
                            // trancheur par défaut : le premier installé
                            if imp_tr == usize::MAX { imp_tr = l.iter().position(|x| x.is_some()).unwrap_or(0); }
                            imp_installes = Some(l);
                        }
                        if imp_job.as_ref().map_or(false, |j| j.1.is_finished()) {
                            let (cle, h) = imp_job.take().unwrap();
                            imp_an = (cle, h.join().ok().map(std::sync::Arc::new));
                            window.request_redraw();
                        }
                        let choix_imp: Vec<usize> = if multi.len() >= 2 { multi.clone() } else if let Some(i) = sel.filter(|&i| i < scene.len() && scene[i].nverts > 0) { vec![i] }
                                                    else { (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect() };
                        let cle = format!("{choix_imp:?} {} {imp_angle} {imp_buse}", cache3d::generation());
                        if imp_an.0 != cle && imp_job.as_ref().map_or(true, |j| j.0 != cle) && imp_job.is_none() && !choix_imp.is_empty() {
                            let tris: Vec<[Vec3; 3]> = choix_imp.iter().flat_map(|&i| scene[i].tris.iter().copied()).collect();
                            let soupe: Vec<f32> = choix_imp.iter().flat_map(|&i| scene[i].verts.iter().copied()).collect();
                            let (a, pm) = (imp_angle, imp_buse * 2.0);
                            imp_job = Some((cle, std::thread::spawn(move || impression::analyser(&tris, &soupe, a, pm))));
                        }
                        if imp_job.is_some() || imp_detect.is_some() { egui_ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
                        if imp_choix_prog.as_ref().map_or(false, |h| h.is_finished()) {
                            if let Some(p) = imp_choix_prog.take().unwrap().join().ok().flatten() { imp_autre = Some(p); imp_tr = impression::TRANCHEURS.len(); }
                        }
                    }
                    // intersections des objets choisis (Booléens) : recalculées quand la sélection ou la
                    // géométrie change ; un seul calcul à la fois, le dernier résultat reste affiché
                    if inter_job.as_ref().map_or(false, |j| j.1.is_finished()) {
                        let (cle, h) = inter_job.take().unwrap();
                        inter_seg = (cle, h.join().unwrap_or_default());
                        window.request_redraw();
                    }
                    let cle_inter = if panneau_bool && multi.len() >= 2 { format!("{multi:?} {}", cache3d::generation()) } else { String::new() };
                    if cle_inter.is_empty() { if !inter_seg.1.is_empty() { inter_seg = (String::new(), Vec::new()); } }
                    else if inter_seg.0 != cle_inter && inter_job.as_ref().map_or(true, |j| j.0 != cle_inter) {
                        if inter_job.is_none() {
                            let objs: Vec<Vec<[Vec3; 3]>> = multi.iter().filter(|&&i| i < scene.len()).map(|&i| scene[i].tris.clone()).collect();
                            inter_job = Some((cle_inter.clone(), std::thread::spawn(move || booleen::intersections(&objs))));
                        }
                    }
                    if inter_job.is_some() { egui_ctx.request_repaint_after(std::time::Duration::from_millis(40)); }
                    // TEXTE : à l'ouverture, le texte se pose de lui-même sur la face visée au centre de la vue
                    // (vue 3D de préférence), sinon sur le dessus de l'objet
                    if panneau_texte && txt_cadre.is_none() && scene.iter().any(|o| o.nverts > 0) {
                        let vue = [ViewKind::ThreeD, ViewKind::Top, ViewKind::Front, ViewKind::Right].into_iter().find_map(|k| rect_of(mode, k).map(|r| (k, r)));
                        let touche = vue.and_then(|(k, r)| {
                            let (mvp, _) = view_mvp(k, &orbit, &ortho, base, aspect_of(mode, k));
                            pick_scene(&mvp, ((r.0 + r.2 * 0.5) as f64, (r.1 + r.3 * 0.5) as f64), r, &scene)
                        });
                        txt_cadre = Some(match touche {
                            Some((_, hit, nrm)) => cadre_sur(hit, nrm, None),
                            None => {
                                let (lo, hi) = sel.filter(|&i| i < scene.len()).map(|i| (scene[i].min, scene[i].max)).unwrap_or_else(|| scene_bounds(&scene));
                                cadre_sur(Vec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, hi.z), Vec3::Z, None)
                            }
                        });
                        txt_sel = true;
                    }
                    // TEXTE : contours de l'aperçu (police, texte, taille, justification) ; le cadre épouse le texte
                    // INSÈRE IMAGE : image choisie (sélecteur en arrière-plan), vectorisation à jour, cadre ajusté au dessin
                    if img_choix.as_ref().map_or(false, |h| h.is_finished()) {
                        if let Some(p) = img_choix.take().unwrap().join().ok().flatten() {
                            match vectorise::Image::charger(&p) {
                                Ok(i) => { img = Some(i); img_prm.pleines.clear(); }
                                Err(e) => status_msg = Some((format!("{} : {e}", tr(lang, "img_illisible")), true, std::time::Instant::now())),
                            }
                        }
                    }
                    if img_choix.is_some() { egui_ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
                    if panneau_texte && txt_image {
                        if let Some(im) = img.as_mut() { im.maj(&mut img_prm); }
                        img_vu = format!("{:?} {img_largeur} {}", img_prm, img.as_ref().map_or(0, |i| i.generation));
                        if let Some(c) = txt_cadre.as_mut() {
                            let cle = format!("image {} {}", img.as_ref().map_or(0, |i| i.generation), img_largeur);
                            if txt_contours.0 != cle {
                                let (p, w, h) = img.as_ref().map(|i| i.contours_mm(img_largeur)).unwrap_or_default();
                                if !p.is_empty() { c.lx = w.max(0.5); c.ly = h.max(0.5); }
                                txt_contours = (cle, p);
                            }
                        }
                    } else if panneau_texte {
                        if let (Some(c), Some(liste)) = (txt_cadre.as_mut(), texte::polices()) {
                            if let Some(pol) = txt_police.and_then(|i| liste.get(i)) {
                                let cle = format!("{} {} {} {}", pol.nom, txt_saisie, txt_taille, txt_just);
                                if txt_contours.0 != cle {
                                    let r = texte::contours(pol, &txt_saisie, txt_taille, txt_just, 0.0);
                                    if let Some((_, w, h)) = &r { c.lx = w.max(0.5); c.ly = h.max(0.5); }
                                    txt_contours = (cle, r.map(|r| r.0).unwrap_or_default());
                                }
                            }
                        }
                    }
                    // ligne de coupe recalculée seulement si le cadre ou la géométrie a changé
                    if let Some(c) = coupe {
                        let cle = format!("{c:?} {}", cache3d::generation());
                        if coupe_section.0 != cle { coupe_section = (cle, section_cadre(&c, &scene)); }
                    } else if !coupe_section.1.is_empty() { coupe_section = (String::new(), Vec::new()); }
                    // outil de rotation lu au moment du dessin, comme la clé du cache (un outil ouvert plus haut l'a peut-être fermé)
                    let rot_xf = gizmo.as_ref().and_then(|g| g.matrix().map(|m| (g.obj, m)));
                    unsafe { if let Some(a_faire) = cache.debut(&gl, size.width as i32, size.height as i32, cle_commune!(), cles_vues!(), evt_recu) {
                        // egui laisse l'état GL modifié (depth test off, blend on, scissor on) :
                        // on le remet pour notre rendu 3D, sinon les faces s'occultent mal.
                        gl.enable(glow::DEPTH_TEST);
                        gl.depth_func(glow::LESS);
                        gl.depth_mask(true);
                        gl.disable(glow::BLEND);
                        gl.disable(glow::CULL_FACE);
                        gl.disable(glow::SCISSOR_TEST);
                        let (bg_o, bg_v) = if dark { ((0.20, 0.22, 0.26), (0.10, 0.12, 0.15)) }
                                           else { ((0.80, 0.82, 0.86), (0.90, 0.91, 0.94)) };
                        // fond général (bordures entre les vues) seulement quand tout est refait
                        if !a_faire.contains(&false) {
                            gl.viewport(0, 0, size.width as i32, size.height as i32);
                            gl.clear_color(bg_o.0, bg_o.1, bg_o.2, 1.0);
                            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                        }
                        gl.enable(glow::SCISSOR_TEST);
                        for (iv, k) in view::ALL_VIEWS.into_iter().enumerate() {
                            if !a_faire[iv] { continue; } // image de cette vue toujours valable
                            if let Some(sr) = view::screen_rect(cur_mode, k, wf, hf, bar_px, side_px) {
                                let (gx, gy, gw, gh) = view::gl_rect(sr, hf);
                                if gw <= 2 || gh <= 2 { continue; }
                                gl.viewport(gx, gy, gw, gh);
                                gl.scissor(gx + 1, gy + 1, gw - 2, gh - 2);
                                gl.clear_color(bg_v.0, bg_v.1, bg_v.2, 1.0);
                                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                                let (mvp, eye) = view_mvp(k, &orbit, &ortho, base, gw as f32 / gh as f32);
                                let a = mvp.to_cols_array();
                                let dim = if disp == DisplayMode::Defects { 0.6 } else { 1.0 };
                                let gray = if disp == DisplayMode::Defects { 1.0 } else { 0.0 };
                                // Mode Texturé : seul l'objet sélectionné est en couleur (les autres sombres).
                                let tex_hl = disp == DisplayMode::Textured;
                                let draw_sel = if tex_hl { sel } else { None };
                                // Mode Transparent : rendu translucide (verre), on voit à travers.
                                let alpha = if disp == DisplayMode::Transparent { 0.30 } else { 1.0 };
                                if disp == DisplayMode::Transparent {
                                    gl.enable(glow::BLEND);
                                    gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                                    gl.depth_mask(false); // pas d'écriture de profondeur : les faces arrière restent visibles
                                }
                                if disp == DisplayMode::Wireframe { gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE); }
                                draw_view(&gl, program, &scene, draw_sel, &u, &grid, &rule, &mvp, eye, show_ruler, show_grid, 0.0, dim, gray, tex_hl, alpha, rot_xf);
                                if disp == DisplayMode::Wireframe { gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL); }
                                if disp == DisplayMode::Transparent { gl.disable(glow::BLEND); gl.depth_mask(true); }
                                // 3D-IMP : surplombs à soutenir (orange) et parois trop fines (violet) sur la pièce
                                if panneau_imp { if let Some(an) = imp_an.1.as_ref() {
                                    if imp_voir_s { plaque.sur_objet(&gl, &a, &an.surplombs, [1.0, 0.55, 0.05, 0.75]); }
                                    if imp_voir_f { plaque.sur_objet(&gl, &a, &an.fines, [0.75, 0.25, 0.95, 0.85]); }
                                } }
                                // SURFACE : faces de pose en rouge sur l'objet (celle sous la souris plus vive)
                                if let Some((_, faces)) = surf.as_ref() {
                                    for (fi, f) in faces.iter().enumerate() {
                                        let vif = surf_survol.or(surf_liste) == Some(fi);
                                        plaque.sur_objet(&gl, &a, &f.tris, if vif { [1.0, 0.10, 0.10, 0.85] } else { [0.95, 0.15, 0.15, 0.55] });
                                    }
                                }
                                // cadre de coupe : plaque rouge translucide au fond (ne masque jamais un objet)
                                if let (Some(c), false) = (coupe, coupe_etape == 1) {
                                    let (ce, bas, haut) = cadre_affiche(&c, &scene);
                                    plaque.dessiner(&gl, &a, &ce.coins(bas, haut), [0.90, 0.16, 0.16, if coupe_sel { 0.42 } else { 0.30 }]);
                                }
                                // Défauts (trous rouges, erreurs jaunes) : UNIQUEMENT en mode Défauts.
                                // (C'est le mode affiché par défaut à l'ouverture et à l'import.)
                                if disp == DisplayMode::Defects {
                                    gl.disable(glow::DEPTH_TEST);
                                    err_lines.draw(&gl, &a, [1.0, 0.85, 0.0, 1.0], 3.0);
                                    bnd_lines.draw(&gl, &a, [1.0, 0.15, 0.15, 1.0], 4.0);
                                    gl.enable(glow::DEPTH_TEST);
                                }
                            }
                        }
                        gl.disable(glow::SCISSOR_TEST);
                        cache.fin(&gl);
                    }
                    cache.afficher(&gl);
                    }
                    SEL_PLUS.lock().unwrap().clear(); // photos : sélection simple seulement
                    evt_recu = false;

                    let raw_input = egui_state.take_egui_input(&window);
                    let mut import_req: Option<bool> = None; // Some(true) = ajouter à la scène
                    let mut export_req: Option<&'static str> = None; // extension choisie dans le menu Sauver
                    let mut shot_req: Option<(u32, u32)> = None;
                    // test sans clic : PATCH3D_AUTOSHOT=1 prend une photo 1024×768 au démarrage (avec PATCH3D_SHOT)
                    if std::env::var_os("PATCH3D_AUTOSHOT").is_some() && egui_ctx.frame_nr() == 5 { shot_req = Some((1024, 768)); }
                    let mut repair_req = false;
                    let mut lissage_req = false;
                    let mut simp_req = false;
                    let busy = job.is_some() || file_job.is_some();
                    let mut undo_req = false;
                    let mut redo_req = false;
                    let mut clear_req = false;
                    let mut relief_req = false;
                    let mut wire_req = false;
                    let mut creux_req = false;
                    let mut tx_act = atelier::Actions::default();
                    let mut resize_final = false;
                    // molette sur TAILLE : historique une fois la série de crans terminée (0,6 s sans cran)
                    if let Some(t) = taille_molette {
                        if t.elapsed().as_secs_f32() > 0.6 { resize_final = true; taille_molette = None; }
                        else { egui_ctx.request_repaint_after(std::time::Duration::from_millis(650)); }
                    }
                    let mut center_req = false;
                    let mut bool_req: Option<booleen::Op> = None;
                    let mut coupe_req = false;
                    let mut forme_req: Option<FormeOp> = None;
                    let mut txt_appl = false;
                    let mut imp_orient_req = false;
                    let mut imp_envoi_req: Option<impression::Lancement> = None;
                    // test sans clic : PATCH3D_REQ=lissage|repair|simp|wire|center|clear déclenche l'action à l'image 6
                    let req_test = if egui_ctx.frame_nr() == 6 { std::env::var("PATCH3D_REQ").ok() } else { None };
                    let mut fit_view: Option<ViewKind> = None; // bouton "Centrer" de l'overlay
                    let mut ui_pressed = false;
                    let full = egui_ctx.run(raw_input, |ctx| {
                        // appui souris sur l'interface (panneaux, barre, menus, boutons de navigation)
                        // (panneaux fixes = hors de la zone de vue 3D : à droite du panneau ou dans la barre du bas ;
                        //  is_pointer_over_area ne voit que les fenêtres flottantes : menus, bulles)
                        let sr = ctx.screen_rect();
                        let hors_vue = ctx.input(|i| i.pointer.interact_pos()).map_or(false, |p| p.x > sr.right() - panel_w || p.y > sr.bottom() - BAR_H);
                        ui_pressed = ctx.input(|i| i.pointer.any_pressed()) && (hors_vue || ctx.is_pointer_over_area());
                        // …sauf la barre de navigation de la vue (tourner/zoomer = permis) et le bandeau d'info
                        if ui_pressed {
                            let lay = ctx.input(|i| i.pointer.interact_pos()).and_then(|p| ctx.layer_id_at(p));
                            if let Some(l) = lay {
                                let nav = view::ALL_VIEWS.iter().any(|k| l.id == egui::Id::new("nav_overlay").with(view::kind_name(*k)));
                                if nav || l.id == egui::Id::new("toast") { ui_pressed = false; }
                            }
                        }
                        // test sans clic : PATCH3D_SPLASHTEST=1 simule un appui sur un bouton à l'image 3
                        if std::env::var_os("PATCH3D_SPLASHTEST").is_some() && ctx.frame_nr() == 3 { ui_pressed = true; }
                        ctx.set_visuals(if dark { egui::Visuals::dark() } else { visuels_clairs() });
                        THEME_CLAIR.store(!dark, std::sync::atomic::Ordering::Relaxed);
                        let ppp = ctx.pixels_per_point();
                        // Couche MIDDLE : les règles/repères passent sous les popups (Foreground) → pas de « transparence ».
                        let fg = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("fg")));
                        // Cotes de l'objet (unité de la règle) : une seule fois, angle haut-droit de toute la zone
                        // de visualisation, sans cadre (texte cerclé de noir), tant que les règles sont visibles.
                        // Pendant le tracé d'une découpe : à leur place, la boîte jaune des métriques de coupe en temps réel.
                        if let (Some(c), true) = (coupe, coupe_etape > 0) {
                            let su = unit.suffix();
                            let lt = fmt_measure(c.reperes().map_or(0.0, |r| r.2), unit);
                            let t = if coupe_etape == 1 { format!("L {lt} {su}\n{} {:.1}°", tr(lang, "coupe_angle"), angle_coupe(&c)) }
                                    else { format!("L {lt} {su}\nl {} {su}\n{} {:.1}°", fmt_measure(c.l, unit), tr(lang, "coupe_incl"), (c.incl + 360.0) % 360.0) };
                            let gal = fg.layout_no_wrap(t, egui::FontId::proportional(14.0), egui::Color32::BLACK);
                            let pos = egui::pos2((wf - side_px) / ppp - 10.0 - gal.size().x, 10.0);
                            fg.rect_filled(egui::Rect::from_min_size(pos, gal.size()).expand(4.0), 4.0, JAUNE);
                            fg.galley(pos, gal, egui::Color32::BLACK);
                        } else if let Some(c) = txt_cadre.filter(|_| panneau_texte) {
                            let su = unit.suffix();
                            let t = format!("X {} × Y {} {su}\n{} {:.1}°\nZ {}{} {su} ({})", fmt_measure(c.lx, unit), fmt_measure(c.ly, unit),
                                            tr(lang, "texte_angle"), c.angle.to_degrees().rem_euclid(360.0), if txt_relief { "+" } else { "−" }, fmt_measure(c.prof, unit),
                                            tr(lang, if txt_relief { "texte_relief_c" } else { "texte_grave_c" }));
                            let gal = fg.layout_no_wrap(t, egui::FontId::proportional(14.0), egui::Color32::BLACK);
                            let pos = egui::pos2((wf - side_px) / ppp - 10.0 - gal.size().x, 10.0);
                            fg.rect_filled(egui::Rect::from_min_size(pos, gal.size()).expand(4.0), 4.0, JAUNE);
                            fg.galley(pos, gal, egui::Color32::BLACK);
                        } else if show_ruler {
                            if let Some(i) = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None }) {
                                let e = (scene[i].max - scene[i].min) / unit.factor();
                                let right = (wf - side_px) / ppp - 10.0;
                                for (n, (nom, v)) in [(tr(lang, "largeur"), e.x), (tr(lang, "longueur"), e.y), (tr(lang, "hauteur_o"), e.z)].into_iter().enumerate() {
                                    label_at(&fg, egui::pos2(right, 8.0 + 18.0 * n as f32), egui::Align2::RIGHT_TOP, &format!("{nom} : {v:.*} {}", unit.decimales(), unit.suffix()), egui::Color32::from_gray(245), 14.0);
                                }
                            }
                        }
                        for k in view::ALL_VIEWS {
                            if let Some(sr) = rect_of(cur_mode, k) {
                                let rpts = egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp));
                                let vp = fg.with_clip_rect(rpts); // clippe les dessins à cette vue
                                if !viewer { vp.text(rpts.min + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, kind_tr(lang, k), egui::FontId::proportional(13.0), if dark { egui::Color32::from_gray(185) } else { egui::Color32::from_gray(60) }); }
                                let (mvp, eye) = view_mvp(k, &orbit, &ortho, base, sr.2 / sr.3.max(1.0));
                                if show_ruler {
                                    let axes = [(Vec3::X, egui::Color32::from_rgb(255,150,150), "X"), (Vec3::Y, egui::Color32::from_rgb(150,230,160), "Y"), (Vec3::Z, egui::Color32::from_rgb(160,200,255), "Z")];
                                    // graduations espacées pour rester lisibles quel que soit le zoom ;
                                    // les chiffres masqués par l'objet (occluded) sont ignorés.
                                    let every = (ruler_majors / 10).max(1);
                                    for (dir, col, letter) in axes {
                                        for c in (every..=ruler_majors).step_by(every as usize) {
                                            let wp = dir * (c as f32 * grid_step);
                                            let t = fmt_measure(c as f32 * grid_step, unit);
                                            let tw = ctx.fonts(|f| f.layout_no_wrap(t.clone(), egui::FontId::proportional(15.0), egui::Color32::WHITE).size().x);
                                            if let Some(sp) = ruler_label_pos(&mvp, wp, dir, sr, ppp, tw, 15.0, 6.0) {
                                                if !over_object(&mvp, sp * ppp, tw * ppp, sr, &scene) { label(&vp, sp, &t, egui::Color32::from_gray(240), 15.0); }
                                            }
                                        }
                                        let wp = dir * ((ruler_majors as f32 + 1.2) * grid_step);
                                        let t = format!("{} {}", letter, unit.suffix());
                                        let tw = ctx.fonts(|f| f.layout_no_wrap(t.clone(), egui::FontId::proportional(17.0), egui::Color32::WHITE).size().x);
                                        if let Some(sp) = ruler_label_pos(&mvp, wp, dir, sr, ppp, tw, 17.0, 6.0) {
                                            if !over_object(&mvp, sp * ppp, tw * ppp, sr, &scene) { label(&vp, sp, &t, col, 17.0); }
                                        }
                                    }
                                }
                                if let Some(g) = gizmo.as_ref() { g.draw(&vp, &mvp, sr, ppp, eye); }
                                if light_mode { let (dc, dr) = dome(&scene); lumiere::dessiner(&vp, &mvp, sr, ppp, dc, dr, lumiere_courante(), light_drag == Some(k)); }
                                if let Some(sel) = selection.filter(|_| gizmo.is_none()) {
                                    if let Some(sp) = project(&mvp, sel, sr, ppp) {
                                        let y = egui::Color32::from_rgb(255, 210, 90);
                                        vp.circle_stroke(sp, 7.0, egui::Stroke::new(2.0, y));
                                        vp.circle_filled(sp, 2.0, y);
                                    }
                                }
                                // Cadre de coupe (ciseaux) : rouge plein translucide, déplié depuis le trait L,
                                // flèche = côté A ; l = 0 : toute la hauteur de la scène (+ marge)
                                if let Some(c) = coupe {
                                    let rouge = egui::Color32::from_rgb(230, 40, 40);
                                    let su = unit.suffix();
                                    let vue = egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp));
                                    if coupe_etape == 1 {
                                        // étape 1 : le trait L seul, sa longueur, et la roue d'angle dans le plan de tracé
                                        if let (Some(a), Some(b)) = (project(&mvp, c.p0, sr, ppp), project(&mvp, c.p1, sr, ppp)) {
                                            vp.line_segment([a, b], egui::Stroke::new(3.0, rouge));
                                            vp.circle_filled(a, 4.0, rouge);
                                            cote_bord(&vp, &[a, b, b, a], [(0, 1), (0, 1)], vue, format!("L {} {su}", fmt_measure(c.reperes().map_or(0.0, |r| r.2), unit)));
                                        }
                                        if coupe_tir == Some(k) {
                                            let (iu, iw) = axes_plan(c.e);
                                            let (mut eu, mut ew) = (Vec3::ZERO, Vec3::ZERO); eu[iu] = 1.0; ew[iw] = 1.0;
                                            roue_angle(&vp, &mvp, sr, ppp, c.p0, eu, ew, angle_coupe(&c));
                                        }
                                    } else {
                                        // sélectionné : contour jaune épais, comme la sélection d'un objet
                                        let bord = if coupe_sel { egui::Stroke::new(3.0, egui::Color32::from_rgb(255, 210, 60)) } else { egui::Stroke::new(2.0, rouge) };
                                        // le remplissage est dessiné dans la scène 3D (derrière les objets) ; ici le contour seul
                                        let (ce, bas, haut) = cadre_affiche(&c, &scene);
                                        let pr: Vec<_> = ce.coins(bas, haut).iter().filter_map(|q| project(&mvp, *q, sr, ppp)).collect();
                                        if pr.len() == 4 {
                                            // 4 traits séparés : un polygone fermé aplati (cadre vu par la tranche) ferait une
                                            // pointe de raccord géante
                                            for i in 0..4 { vp.line_segment([pr[i], pr[(i + 1) % 4]], bord); }
                                        } else if let (Some(a), Some(b)) = (project(&mvp, ce.p0, sr, ppp), project(&mvp, ce.p1, sr, ppp)) {
                                            vp.line_segment([a, b], bord);
                                        }
                                        // ligne de coupe sur l'objet : trait rouge fin (précision), toujours visible
                                        lignes_rouges(&vp, &coupe_section.1, &mvp, sr, ppp);
                                        let rect_l = ce.reperes().map_or(0.0, |r| r.2);
                                        if let (Some((u, n, l)), Some(w)) = (ce.reperes(), ce.dir_l()) {
                                            let mil = ce.origine() + u * (l * 0.5) + w * ((bas + haut) * 0.5);
                                            if let (Some(a), Some(b)) = (project(&mvp, mil, sr, ppp), project(&mvp, mil + n * (l * 0.15).clamp(3.0, 30.0), sr, ppp)) {
                                                vp.arrow(a, b - a, egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 120, 100)));
                                            }
                                        }
                                        // cotes du rectangle en temps réel : L le long du trait, l = dépli
                                        // (rien si le cadre est vu par la tranche : moins de 20 px d'épaisseur à l'écran)
                                        if pr.len() == 4 && (pr[1] - pr[0]).length().min((pr[2] - pr[1]).length()) >= 20.0 {
                                            cote_bord(&vp, &pr, [(0, 1), (2, 3)], vue, format!("L {} {su}", fmt_measure(rect_l, unit)));
                                            cote_bord(&vp, &pr, [(1, 2), (3, 0)], vue, format!("l {} {su}", fmt_measure(haut - bas, unit)));
                                        }
                                        // étape 2 : roue d'inclinaison autour du trait (0° = plan de tracé, 90° = le long de l'axe)
                                        if let (2, Some((u, _, l)), Some(np)) = (coupe_etape, c.reperes(), c.perp_plan()) {
                                            roue_angle(&vp, &mvp, sr, ppp, c.origine() + u * (l * 0.5), np, c.axe(), c.incl);
                                        }
                                    }
                                }
                                // TEXTE : cadre (rouge), profondeur, poignées de rotation (jaunes), aperçu du texte, cotes
                                if panneau_texte { if let Some(c) = txt_cadre {
                                    let rouge = egui::Color32::from_rgb(235, 45, 40);
                                    let dh = if txt_relief { c.prof } else { -c.prof };
                                    let (haut_, bas_) = (c.coins(0.0), c.coins(dh));
                                    let pr = |q: Vec3| project(&mvp, q, sr, ppp);
                                    for i in 0..4 {
                                        if let (Some(a), Some(b)) = (pr(haut_[i]), pr(haut_[(i + 1) % 4])) { vp.line_segment([a, b], if txt_sel { egui::Stroke::new(3.0, JAUNE) } else { egui::Stroke::new(2.0, rouge) }); }
                                        if let (Some(a), Some(b)) = (pr(bas_[i]), pr(bas_[(i + 1) % 4])) { vp.line_segment([a, b], egui::Stroke::new(1.0, rouge.gamma_multiply(0.6))); }
                                        if let (Some(a), Some(b)) = (pr(haut_[i]), pr(bas_[i])) { vp.line_segment([a, b], egui::Stroke::new(1.0, rouge.gamma_multiply(0.6))); }
                                    }
                                    // lettres en fil rouge 3D : contour en surface, contour à la profondeur (creusé ou en
                                    // relief) et arêtes entre les deux
                                    // (contours en polylignes, arêtes espacées : une image vectorisée a des milliers de points)
                                    contours_rouges(&vp, &txt_contours.1, &c, dh, &mvp, sr, ppp);
                                    if txt_deplace.is_some() {
                                        if let Some(c0) = pr(c.pt(0.0, 0.0, 0.0)) {
                                            let bleu = egui::Color32::from_rgb(70, 160, 255);
                                            let dirs: Vec<Vec3> = if k == ViewKind::ThreeD { vec![Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] }
                                                                  else { let (rr, uu) = Ortho::right_up(k); vec![rr, -rr, uu, -uu] };
                                            for d in dirs {
                                                if let Some(p1) = pr(c.pt(0.0, 0.0, 0.0) + d) {
                                                    let v = p1 - c0;
                                                    if v.length() > 0.5 { vp.arrow(c0, v.normalized() * 30.0, egui::Stroke::new(2.5, bleu)); }
                                                }
                                            }
                                        }
                                    }
                                    // poignée de profondeur (flèche bleue le long de la normale)
                                    if let Some((b, tip)) = poignee_prof(&c, &mvp, sr, ppp, txt_relief) {
                                        let bleu = egui::Color32::from_rgb(70, 160, 255);
                                        vp.line_segment([b, tip], egui::Stroke::new(3.0, bleu));
                                        vp.circle_filled(tip, 7.0, bleu); vp.circle_stroke(tip, 7.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
                                    }
                                    // façon 3D Builder : roues de rotation au milieu des arêtes L et l, poignées de taille aux coins
                                    if !de_tranche(&c, k) {
                                        for q in c.roues() { if let Some(sp) = pr(q) {
                                            // petite roue : disque jaune cerclé de noir, 4 crans
                                            vp.circle_filled(sp, 9.0, JAUNE); vp.circle_stroke(sp, 9.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
                                            vp.circle_stroke(sp, 4.5, egui::Stroke::new(1.5, egui::Color32::BLACK));
                                            for i in 0..4 { let a = i as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4; let d = egui::vec2(a.cos(), a.sin());
                                                vp.line_segment([sp + d * 5.5, sp + d * 8.5], egui::Stroke::new(1.5, egui::Color32::BLACK)); }
                                        } }
                                        for q in c.coins(0.0) { if let Some(sp) = pr(q) {
                                            let r = egui::Rect::from_center_size(sp, egui::vec2(11.0, 11.0));
                                            vp.rect_filled(r, 2.0, egui::Color32::WHITE); vp.rect_stroke(r, 2.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
                                        } }
                                    }
                                    // cotes X / Y sur le cadre (tracé, sélection, rotation) et roue d'angle autour du centre
                                    if (txt_sel || txt_rot.is_some() || txt_echelle.is_some() || txt_prof_glisse.is_some()) && !de_tranche(&c, k) {
                                        let pc: Vec<_> = c.coins(0.0).iter().filter_map(|q| pr(*q)).collect();
                                        if pc.len() == 4 && (pc[1] - pc[0]).length().min((pc[2] - pc[1]).length()) >= 20.0 {
                                            let vue = egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp));
                                            let u_ = unit.suffix();
                                            cote_bord(&vp, &pc, [(0, 1), (2, 3)], vue, format!("X {} {u_}", fmt_measure(c.lx, unit)));
                                            cote_bord(&vp, &pc, [(1, 2), (3, 0)], vue, format!("Y {} {u_}", fmt_measure(c.ly, unit)));
                                        }
                                    }
                                    if txt_rot.is_some() {
                                        roue_angle(&vp, &mvp, sr, ppp, c.pt(0.0, 0.0, 0.0), c.u, c.v, c.angle.to_degrees());
                                    }
                                } }
                                // Booléens : là où les objets sélectionnés se croisent, trait rouge fin
                                if panneau_bool && multi.len() >= 2 { lignes_rouges(&vp, &inter_seg.1, &mvp, sr, ppp); }
                                // trou d'évacuation qui vient d'être percé (fil de fer rouge, toujours visible)
                                if let Some((_, _, sg)) = &trou_marque { lignes_rouges(&vp, sg, &mvp, sr, ppp); }
                                // Aperçu du tracé en cours (rectangle / cercle / ligne) sur la grille.
                                if let (Some(shape), Some(p0), Some(p1)) = (draw_tool, draw_start, draw_cur) {
                                    let st = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 170, 40));
                                    match shape {
                                        DrawShape::Line => {
                                            if let (Some(a), Some(b)) = (project(&mvp, p0, sr, ppp), project(&mvp, p1, sr, ppp)) { vp.line_segment([a, b], st); }
                                        }
                                        DrawShape::Rect => {
                                            let (x0, x1, y0, y1) = (p0.x.min(p1.x), p0.x.max(p1.x), p0.y.min(p1.y), p0.y.max(p1.y));
                                            let cs = [Vec3::new(x0,y0,0.0), Vec3::new(x1,y0,0.0), Vec3::new(x1,y1,0.0), Vec3::new(x0,y1,0.0)];
                                            let pr: Vec<_> = cs.iter().filter_map(|c| project(&mvp, *c, sr, ppp)).collect();
                                            if pr.len() == 4 { for i in 0..4 { vp.line_segment([pr[i], pr[(i+1)%4]], st); } }
                                        }
                                        DrawShape::Circle => {
                                            let rad = (p1 - p0).length(); let n = 48; let mut prev = None;
                                            for i in 0..=n { let ang = i as f32/n as f32*std::f32::consts::TAU;
                                                let wp = p0 + Vec3::new(rad*ang.cos(), rad*ang.sin(), 0.0);
                                                if let Some(sp) = project(&mvp, wp, sr, ppp) { if let Some(pp) = prev { vp.line_segment([pp, sp], st); } prev = Some(sp); } }
                                        }
                                    }
                                }
                            }
                        }
                        // Rectangle du zoom "Zone" en cours de tracé.
                        if let Some((_, s, c)) = zone {
                            let r = egui::Rect::from_two_pos(egui::pos2(s.0 as f32 / ppp, s.1 as f32 / ppp), egui::pos2(c.0 as f32 / ppp, c.1 as f32 / ppp));
                            fg.rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(90, 150, 230, 40));
                            fg.rect_stroke(r, 0.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 180, 240)));
                        }
                        // --- Overlay de navigation : apparaît au survol de chaque vue ---
                        // mode OBJ-AXE : les outils de navigation sont désactivés (le clic gauche reste à l'objet)
                        if gizmo.is_some() || panneau_bool || panneau_forme { nav_tool = None; }
                        let ptr = ctx.pointer_latest_pos();
                        let menu_barre_ouvert = ctx.memory(|m| m.any_popup_open());
                        // Logo ÉCLAIRAGE : haut-gauche de CHAQUE fenêtre de vue (vue unique ou 4 vues),
                        // au survol de cette fenêtre (ou dans toutes si le mode est actif)
                        // Icône GRILLE 3D : en bas à gauche de la zone de visualisation ; chaque clic passe à
                        // grille + axes -> grille seule -> axes seuls -> rien -> …
                        if !viewer {
                            let coin = egui::pos2(8.0, (hf - bar_px) / ppp - 8.0);
                            egui::Area::new(egui::Id::new("grille3d")).order(egui::Order::Foreground).fixed_pos(coin).pivot(egui::Align2::LEFT_BOTTOM).show(ctx, |ui| {
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click());
                                let pnt = ui.painter();
                                let gm = grille_mode();
                                pnt.rect_filled(rect, 7.0, if resp.hovered() { egui::Color32::from_gray(70) } else { egui::Color32::from_rgba_unmultiplied(30, 32, 38, 230) });
                                pnt.rect_stroke(rect, 7.0, egui::Stroke::new(1.2, egui::Color32::from_gray(120)));
                                draw_grille3d(pnt, rect.shrink(6.0), matches!(gm, 0 | 1), matches!(gm, 0 | 2));
                                let etat = tr(lang, ["g_etat0", "g_etat1", "g_etat2", "g_etat3"][gm as usize % 4]);
                                let resp = resp.on_hover_text(format!("{} : {etat}\n{}", tr(lang, "g_aff"), tr(lang, "g_cycle")));
                                if resp.clicked() {
                                    let n = (gm + 1) % 4;
                                    GRILLE_MODE.store(n, std::sync::atomic::Ordering::Relaxed);
                                    show_grid = matches!(n, 0 | 1);
                                }
                            });
                        }
                        // icône seulement dans la PREMIÈRE fenêtre (en haut à gauche) ; les autres fenêtres
                        // n'affichent que le réglage (dôme). Visible au survol de n'importe quelle vue, visionneuse comprise.
                        let premiere = view::ALL_VIEWS.iter().copied().filter_map(|k| rect_of(cur_mode, k).map(|r| (k, r)))
                            .min_by(|a, b| (a.1 .1, a.1 .0).partial_cmp(&(b.1 .1, b.1 .0)).unwrap_or(std::cmp::Ordering::Equal));
                        let survol_vues = ptr.map_or(false, |p| view::ALL_VIEWS.iter().any(|&k| rect_of(cur_mode, k).map_or(false, |sr| {
                            egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp)).contains(p) })));
                        { if let Some((k, sr)) = premiere {
                            {
                                let rpts = egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp));
                                let survol = survol_vues;
                                if (survol || light_mode) && !menu_barre_ouvert && gizmo.is_none() {
                                    egui::Area::new(egui::Id::new("eclairage").with(view::kind_name(k))).order(egui::Order::Foreground)
                                        .fixed_pos(rpts.min + egui::vec2(8.0, 26.0)).show(ctx, |ui| {
                                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                                            let pnt = ui.painter();
                                            let fond = if light_mode { egui::Color32::from_rgb(58, 110, 150) } else if resp.hovered() { egui::Color32::from_gray(70) } else { egui::Color32::from_rgba_unmultiplied(30, 32, 38, 230) };
                                            pnt.rect_filled(rect, 7.0, fond);
                                            pnt.rect_stroke(rect, 7.0, egui::Stroke::new(1.2, egui::Color32::from_gray(if light_mode { 230 } else { 120 })));
                                            lumiere::projecteur(pnt, rect.center() + egui::vec2(-3.0, -3.0), egui::vec2(0.7, 0.7).normalized(), 12.0, egui::Color32::from_rgb(255, 215, 60));
                                            let resp = resp.on_hover_text(tr(lang, "lum_tip"));
                                            if resp.double_clicked() { lumiere_regler(lumiere::defaut()); }
                                            else if resp.clicked() { light_mode = !light_mode; if light_mode { gizmo = None; } }
                                        });
                                }
                            }
                        } }
                        // VISIONNEUSE : icône Export (disquette) en bas à gauche ; un clic corrige d'office les défauts
                        // (réparation habituelle) puis ouvre le menu de sauvegarde
                        if viewer { if let Some((_, sr)) = premiere {
                            let rpts = egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp));
                            let eid = egui::Id::new("pop_export_vis");
                            egui::Area::new(eid.with("btn")).order(egui::Order::Foreground).fixed_pos(rpts.left_bottom() + egui::vec2(10.0, -10.0))
                                .pivot(egui::Align2::LEFT_BOTTOM).show(ctx, |ui| {
                                    let lib = if busy && export_apres_rep { tr(lang, "export_rep") } else { tr(lang, "export") };
                                    let r = icon_button_w(ui, lib, ui.memory(|m| m.is_popup_open(eid)) || export_apres_rep, draw_floppy, 1.0, 72.0).on_hover_text(tr(lang, "export_tip"));
                                    // test sans clic : PATCH3D_OPEN=export simule UN clic dès l'image 4 (objet chargé)
                                    let test = std::env::var("PATCH3D_OPEN").as_deref() == Ok("export") && !busy && scene.iter().any(|o| o.nverts > 0);
                                    if test && !EXPORT_TEST.load(std::sync::atomic::Ordering::Relaxed) { ui.ctx().request_repaint(); }
                                    let clic = r.clicked() || (test && ui.ctx().frame_nr() >= 4 && !EXPORT_TEST.swap(true, std::sync::atomic::Ordering::Relaxed));
                                    if clic && !busy {
                                        if defects.holes + defects.face_errors > 0 { repair_req = true; export_apres_rep = true; }
                                        else { ui.memory_mut(|m| m.toggle_popup(eid)); }
                                    }
                                    if ouvrir_export { ui.memory_mut(|m| m.open_popup(eid)); ouvrir_export = false; }
                                    let n_tot = scene.iter().filter(|o| o.nverts > 0).count();
                                    bar_menu_above(ui.ctx(), eid, &r, 330.0, |ui| { if let Some(e) = menu_sauver(ui, lang, 0, n_tot) { export_req = Some(e); export_fichier = true; } });
                                });
                            // bouton Fil de fer / Plein en haut à droite (même style que l'icône d'éclairage) : logo fil de fer
                            // pour passer en fil de fer, logo facettes pour revenir au rendu plein
                            let fil = disp == DisplayMode::Wireframe;
                            egui::Area::new(egui::Id::new("vis_fil")).order(egui::Order::Foreground)
                                .fixed_pos(egui::pos2(rpts.right() - 8.0, rpts.top() + 26.0)).pivot(egui::Align2::RIGHT_TOP).show(ctx, |ui| {
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                                    let pnt = ui.painter();
                                    let fond = if resp.hovered() { egui::Color32::from_gray(70) } else { egui::Color32::from_rgba_unmultiplied(40, 44, 52, 200) };
                                    pnt.rect_filled(rect, 7.0, fond);
                                    pnt.rect_stroke(rect, 7.0, egui::Stroke::new(1.2, egui::Color32::from_gray(120)));
                                    let ir = rect.shrink(7.0);
                                    if fil { draw_facettes(pnt, ir) } else { draw_fil_de_fer(pnt, ir) }
                                    if resp.on_hover_text(tr(lang, if fil { "vis_plein_tip" } else { "vis_fil_tip" })).clicked() {
                                        disp = if fil { DisplayMode::Textured } else { DisplayMode::Wireframe };
                                    }
                                });
                            // jauge EXPORTATION : la jauge de calcul de l'interface, 1,5 fois plus grande, centrée en bas
                            let etat = if chargement_initial && file_job.is_some() {
                                let t = debut_appli.elapsed().as_secs_f32();
                                let estime = 0.9 * (1.0 - (-t / taille_chargement).exp());
                                let f = progress::get().max(estime);
                                Some((f, format!("{} {} %", tr(lang, "chargement"), (f * 100.0) as u32), true))
                            }
                                       else if export_apres_rep && job.is_some() { let f = progress::get(); Some((f, format!("{} {} %", tr(lang, "exportation"), (f * 100.0) as u32), false)) }
                                       else if export_fichier && file_job.is_some() { Some((1.0, tr(lang, "exportation").to_string(), true)) }
                                       else if export_fin.map_or(false, |t| t.elapsed().as_secs_f32() < 1.2) { Some((1.0, format!("{} 100 %", tr(lang, "exportation")), true)) }
                                       else { None };
                            if let Some((frac, txt, vert)) = etat {
                                let w = ((PANEL_W - 22.0) * 1.5).min(rpts.width() - 2.0 * 110.0).max(200.0);
                                egui::Area::new(egui::Id::new("jauge_export")).order(egui::Order::Foreground)
                                    .fixed_pos(egui::pos2(rpts.center().x, rpts.bottom() - 14.0)).pivot(egui::Align2::CENTER_BOTTOM).interactable(false)
                                    .show(ctx, |ui| thermo_gauge_k(ui, frac, true, &txt, w, vert, 1.5));
                                ctx.request_repaint_after(std::time::Duration::from_millis(60));
                            }
                        } }
                        for k in view::ALL_VIEWS {
                            if let Some(sr) = rect_of(cur_mode, k) {
                                let rpts = egui::Rect::from_min_size(egui::pos2(sr.0 / ppp, sr.1 / ppp), egui::vec2(sr.2 / ppp, sr.3 / ppp));
                                let hover = ptr.map_or(false, |p| rpts.contains(p));
                                // un menu de la barre est déployé (vues, Taille, Sauver…) : pas de boutons de survol
                                // menu déployé ou mode éclairage (projecteur) : pas de boutons de survol
                                // modes OBJ-AXE et BOOLÉENS : pas de barre de navigation au survol (elle recouvrirait
                                // SURFACE / GRAVITÉ / SORTIR) ; la souris navigue toujours (clic droit, molette, milieu)
                                if !hover || viewer || menu_barre_ouvert || light_mode || gizmo.is_some() || panneau_bool || panneau_forme || panneau_imp { continue; }
                                let pos = egui::pos2(rpts.center().x, rpts.max.y - 48.0);
                                egui::Area::new(egui::Id::new("nav_overlay").with(view::kind_name(k)))
                                    .fixed_pos(pos).order(egui::Order::Foreground)
                                    .pivot(egui::Align2::CENTER_CENTER)
                                    .show(ctx, |ui| {
                                        egui::Frame::popup(ui.style()).fill(egui::Color32::from_rgba_unmultiplied(30, 32, 38, 230)).show(ui, |ui| {
                                            ui.spacing_mut().item_spacing.x = 3.0;
                                            ui.horizontal(|ui| {
                                                // Icône + label, surbrillance si l'outil est actif (même style que la barre).
                                                if icon_button(ui, tr(lang, "nav_zone"), nav_tool == Some(NavTool::Zone), draw_nav_zone).clicked() {
                                                    nav_tool = if nav_tool == Some(NavTool::Zone) { None } else { Some(NavTool::Zone) };
                                                }
                                                if icon_button(ui, tr(lang, "nav_zoom"), nav_tool == Some(NavTool::Zoom), draw_nav_zoom).clicked() {
                                                    nav_tool = if nav_tool == Some(NavTool::Zoom) { None } else { Some(NavTool::Zoom) };
                                                }
                                                if icon_button(ui, tr(lang, "nav_mvt"), nav_tool == Some(NavTool::Pan), draw_nav_move).clicked() {
                                                    nav_tool = if nav_tool == Some(NavTool::Pan) { None } else { Some(NavTool::Pan) };
                                                }
                                                if k == ViewKind::ThreeD {
                                                    if icon_button(ui, tr(lang, "nav_rot"), nav_tool == Some(NavTool::Rotate), draw_nav_rotate).clicked() {
                                                        nav_tool = if nav_tool == Some(NavTool::Rotate) { None } else { Some(NavTool::Rotate) };
                                                    }
                                                }
                                                if icon_button(ui, tr(lang, "centrer"), false, draw_nav_center).clicked() { fit_view = Some(k); }
                                            });
                                        });
                                    });
                            }
                        }
                        // Bouton GRAVITÉ (outil de rotation ouvert) : en bas à droite de la zone de visualisation.
                        let mut grav_req = false;
                        if grav_test && ctx.frame_nr() >= 4 && !scene.is_empty() {
                            if gizmo.is_none() { gizmo = Some(gizmo::Gizmo::new(0, scene[0].min, scene[0].max)); }
                            if ctx.frame_nr() >= 8 { grav_req = true; grav_test = false; }
                        }
                        // outil de rotation refermé : le choix de face s'arrête aussi
                        if gizmo.is_none() && (surf.is_some() || surf_analyse.is_some()) { surf = None; surf_survol = None; surf_analyse = None; }
                        let mut surf_req = false;
                        // largeur de SURFACE / GRAVITÉ selon la langue (SCHWERKRAFT…) : rien ne déborde
                        let larg_btn = |t: &str| (ctx.fonts(|f| f.layout_no_wrap(t.to_string(), egui::FontId::proportional(13.5), egui::Color32::BLACK).size().x) + 48.0).max(118.0);
                        let (w_surf, w_grav) = (larg_btn(&tr(lang, "surf_btn")), larg_btn(&tr(lang, "grav_btn")));
                        if grav_anim.is_none() && gizmo.as_ref().map_or(false, |g| g.drag.is_none()) {
                            // Bouton SURFACE : à gauche de GRAVITÉ
                            let coin_s = egui::pos2((wf - side_px) / ppp - 14.0 - 46.0 - w_grav - 8.0, (hf - bar_px) / ppp - 14.0);
                            egui::Area::new(egui::Id::new("surface")).order(egui::Order::Foreground)
                                .fixed_pos(coin_s).pivot(egui::Align2::RIGHT_BOTTOM).show(ctx, |ui| {
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w_surf, 38.0), egui::Sense::click());
                                    let pnt = ui.painter();
                                    let actif = surf.is_some() || surf_analyse.is_some();
                                    let fond = if actif { egui::Color32::from_rgb(220, 50, 45) } else if resp.hovered() { egui::Color32::from_rgb(250, 222, 110) } else { JAUNE };
                                    pnt.rect_filled(rect, 7.0, fond);
                                    pnt.rect_stroke(rect, 7.0, egui::Stroke::new(1.4, egui::Color32::from_rgb(255, 232, 150)));
                                    let enc = if actif { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 30, 25) };
                                    draw_surface(pnt, egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 7.0), egui::vec2(24.0, 24.0)), enc);
                                    pnt.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::LEFT_CENTER, tr(lang, "surf_btn"), egui::FontId::proportional(13.5), if actif { egui::Color32::WHITE } else { egui::Color32::BLACK });
                                    if resp.on_hover_text(tr(lang, "surf_tip")).clicked() { surf_req = true; }
                                });
                        }
                        if std::env::var("PATCH3D_OPEN").as_deref() == Ok("surface") && ctx.frame_nr() == 6 && gizmo.is_some() { surf_req = true; }
                        if surf_req {
                            if surf.is_some() || surf_analyse.is_some() { surf = None; surf_survol = None; surf_analyse = None; }
                            else {
                                match gizmo.as_ref().map(|g| g.obj).or(sel).filter(|&i| i < scene.len() && scene[i].nverts > 0) {
                                    None => status_msg = Some((tr(lang, "surf_nosel").into(), true, std::time::Instant::now())),
                                    Some(i) => {
                                        // analyse en arrière-plan : l'appli reste fluide, la boîte montre l'avancement
                                        let v = scene[i].verts.clone();
                                        progress::reset(); // la jauge de la colonne de droite suit l'analyse
                                        surf_analyse = Some((i, std::time::Instant::now(), std::thread::spawn(move || gravite::faces_de_pose(&v))));
                                    }
                                }
                            }
                        }
                        if surf_analyse.as_ref().map_or(false, |a| a.2.is_finished()) {
                            let (i, _, h) = surf_analyse.take().unwrap();
                            let f = h.join().unwrap_or_default();
                            if i >= scene.len() {}
                            else if f.is_empty() { status_msg = Some((tr(lang, "surf_aucune").into(), true, std::time::Instant::now())); }
                            else { surf = Some((i, f)); surf_survol = None; }
                        }
                        if surf_analyse.is_some() { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
                        // ===== boîte ANALYSE DES SURFACES (en haut de la zone de visualisation) =====
                        let mut pose_req: Option<(usize, Vec3)> = None;
                        let mut liste_survol = None;
                        if (surf_analyse.is_some() || surf.is_some()) && !viewer {
                            let centre_x = (wf - side_px) / ppp * 0.5;
                            egui::Area::new(egui::Id::new("analyse_surfaces")).order(egui::Order::Foreground)
                                .fixed_pos(egui::pos2(centre_x, 46.0)).pivot(egui::Align2::CENTER_TOP).show(ctx, |ui| {
                                    egui::Frame::popup(ui.style()).fill(egui::Color32::from_rgb(24, 26, 30)).stroke(egui::Stroke::new(1.5, JAUNE))
                                        .inner_margin(egui::Margin::same(10.0)).show(ui, |ui| {
                                        ui.set_width(360.0);
                                        title_box(ui, &tr(lang, "surf_analyse").to_uppercase(), 13.5, 360.0);
                                        ui.add_space(4.0);
                                        if let Some((i, t0, _)) = &surf_analyse {
                                            ui.horizontal(|ui| {
                                                ui.spinner();
                                                ui.label(egui::RichText::new(format!("{} ({} {}) — {:.1} s", tr(lang, "surf_encours"), scene.get(*i).map_or(0, |o| o.tris.len()), tr(lang, "triangles"), t0.elapsed().as_secs_f32())).size(12.5));
                                            });
                                        } else if let Some((i, faces)) = &surf {
                                            ui.label(egui::RichText::new(format!("{} {}", faces.len(), tr(lang, "surf_trouvees"))).size(12.5).strong());
                                            ui.add(egui::Label::new(egui::RichText::new(tr(lang, "surf_choisir")).size(11.5).color(egui::Color32::from_gray(190))).wrap(true));
                                            ui.add_space(4.0);
                                            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                                for (k, f) in faces.iter().enumerate() {
                                                    let vif = surf_survol.or(surf_liste) == Some(k);
                                                    // orientation en mots : où se trouve cette face sur l'objet tel qu'il est posé
                                                    let o = if f.n.z < -0.9 { "surf_o_dessous" } else if f.n.z > 0.9 { "surf_o_dessus" } else if f.n.x < -0.9 { "surf_o_gauche" }
                                                        else if f.n.x > 0.9 { "surf_o_droite" } else if f.n.y < -0.9 { "surf_o_avant" } else if f.n.y > 0.9 { "surf_o_arriere" } else { "surf_o_incline" };
                                                    let txt = format!("{:>2}.  {:>9} mm²   {}", k + 1, fmt_aire(f.aire), tr(lang, o));
                                                    let r = ui.add_sized(egui::vec2(ui.available_width(), 22.0), egui::SelectableLabel::new(vif, egui::RichText::new(txt).size(12.0).monospace()));
                                                    if r.hovered() { liste_survol = Some(k); }
                                                    if r.on_hover_text(tr(lang, "surf_poser_tip")).clicked() { pose_req = Some((*i, f.n)); }
                                                }
                                            });
                                        }
                                    });
                                });
                        }
                        if liste_survol != surf_liste { surf_liste = liste_survol; ctx.request_repaint(); }
                        if let Some((i, n)) = pose_req {
                            if i < scene.len() { grav_anim = Some(GravAnim::vers(i, &scene[i], n)); selection = None; }
                            surf = None; surf_survol = None; surf_liste = None;
                        }
                        if grav_anim.is_none() && gizmo.as_ref().map_or(false, |g| g.drag.is_none()) {
                            let coin = egui::pos2((wf - side_px) / ppp - 14.0 - 46.0, (hf - bar_px) / ppp - 14.0);
                            egui::Area::new(egui::Id::new("gravite")).order(egui::Order::Foreground)
                                .fixed_pos(coin).pivot(egui::Align2::RIGHT_BOTTOM).show(ctx, |ui| {
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w_grav, 38.0), egui::Sense::click());
                                    let pnt = ui.painter();
                                    let fond = if resp.hovered() { egui::Color32::from_rgb(250, 222, 110) } else { JAUNE };
                                    pnt.rect_filled(rect, 7.0, fond);
                                    pnt.rect_stroke(rect, 7.0, egui::Stroke::new(1.4, egui::Color32::from_rgb(255, 232, 150)));
                                    // logo clignotant rouge (créneau 0,5 s)
                                    let on = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                                    let rouge = if on { egui::Color32::from_rgb(220, 20, 20) } else { egui::Color32::from_rgb(120, 60, 40).gamma_multiply(0.45) };
                                    draw_gravite(pnt, egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 7.0), egui::vec2(24.0, 24.0)), rouge);
                                    pnt.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::LEFT_CENTER, tr(lang, "grav_btn"), egui::FontId::proportional(13.5), egui::Color32::BLACK);
                                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                                    if resp.on_hover_text(tr(lang, "grav_tip")).clicked() { grav_req = true; }
                                });
                        }
                        // Menu du clic droit sur une pièce : Dégrouper la pièce / toutes les pièces
                        if let Some((pos, i, c, n, groupe)) = menu_piece.clone() {
                            let r = egui::Area::new(egui::Id::new("menu_piece")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
                                egui::Frame::menu(ui.style()).inner_margin(egui::Margin::symmetric(6.0, 4.0)).show(ui, |ui| {
                                    ui.set_max_width(160.0);
                                    ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
                                    let item = |ui: &mut egui::Ui, t: &str| ui.add(egui::Button::new(egui::RichText::new(t).size(12.5)).frame(false).wrap(false));
                                    if !groupe.is_empty() {
                                        if item(ui, &tr(lang, "grouper").replace("{n}", &groupe.len().to_string())).clicked() { grouper_req = Some(groupe.clone()); }
                                    }
                                    if n >= 2 {
                                        ui.label(egui::RichText::new(tr(lang, "piece_titre").replace("{k}", &(c + 1).to_string()).replace("{n}", &n.to_string())).size(11.5).color(JAUNE));
                                        if item(ui, tr(lang, "degroup_piece")).clicked() { degroup_req = Some((i, c, false)); }
                                        if item(ui, &tr(lang, "degroup_tout").replace("{n}", &n.to_string())).clicked() { degroup_req = Some((i, c, true)); }
                                    }
                                });
                            });
                            // clic ailleurs ou Échap : le menu se ferme
                            let ailleurs = ctx.input(|inp| inp.pointer.any_pressed()) && !r.response.hovered() && !ctx.is_pointer_over_area()
                                || ctx.input(|inp| inp.key_pressed(egui::Key::Escape));
                            if degroup_req.is_some() || grouper_req.is_some() || ailleurs { menu_piece = None; }
                        }
                        // Bouton SORTIR (modes OBJ-AXE et BOOLÉENS) : dans l'angle, à droite de GRAVITÉ
                        if grav_anim.is_none() && (gizmo.as_ref().map_or(false, |g| g.drag.is_none()) || panneau_bool || panneau_forme || panneau_imp || tex.actif) && !viewer {
                            let coin_x = egui::pos2((wf - side_px) / ppp - 14.0, (hf - bar_px) / ppp - 14.0);
                            egui::Area::new(egui::Id::new("sortir")).order(egui::Order::Foreground)
                                .fixed_pos(coin_x).pivot(egui::Align2::RIGHT_BOTTOM).show(ctx, |ui| {
                                    // icône seule (le libellé est dans l'infobulle)
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::click());
                                    let pnt = ui.painter();
                                    let fond = if resp.hovered() { egui::Color32::from_rgb(250, 222, 110) } else { JAUNE };
                                    pnt.rect_filled(rect, 7.0, fond);
                                    pnt.rect_stroke(rect, 7.0, egui::Stroke::new(1.4, egui::Color32::from_rgb(255, 232, 150)));
                                    draw_sortir(pnt, egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)), egui::Color32::from_rgb(40, 40, 40));
                                    if resp.on_hover_text(format!("{} : {}", tr(lang, "sortir_btn"), tr(lang, "sortir_tip"))).clicked() {
                                        // ferme l'outil de rotation et le panneau Booléens (vue et outils d'avant rétablis)
                                        gizmo = None; surf = None; surf_survol = None; panneau_bool = false; panneau_forme = false; panneau_imp = false;
                                        tex.fermer();
                                    }
                                });
                        }
                        // MODE TEXTURE : cercle du pinceau (rouge en effacement, Maj)
                        if let Some((c, r)) = tx_curseur {
                            let col = if mods.shift_key() { egui::Color32::from_rgb(235, 90, 70) } else { JAUNE };
                            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tx_pinceau")));
                            p.circle_stroke(c, r.max(3.0), egui::Stroke::new(1.6, col));
                            p.circle_filled(c, 1.8, col);
                        }
                        if grav_req && grav_anim.is_none() {
                            if let Some(i) = gizmo.as_ref().map(|g| g.obj).filter(|&i| i < scene.len()) {
                                match GravAnim::new(i, &scene[i]) {
                                    Ok(a) => { grav_anim = Some(a); selection = None; }
                                    Err(_) => status_msg = Some((format!("{} : {}", tr(lang, "grav"), tr(lang, "grav_novol")), true, std::time::Instant::now())),
                                }
                            }
                        }
                        // Chute animée : une image par trame jusqu'à l'immobilisation.
                        if let Some(a) = &grav_anim {
                            let i = a.obj;
                            if i >= scene.len() || scene[i].verts.len() != a.depart.len() { grav_anim = None; }
                            else {
                                let fin = a.appliquer(&mut scene[i]);
                                unsafe { scene[i].upload(&gl); }
                                if let Some(g) = gizmo.as_mut().filter(|g| g.obj == i) { *g = gizmo::Gizmo::new(i, scene[i].min, scene[i].max); }
                                if fin {
                                    grav_anim = None;
                                    defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                    history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                    if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                    let e = scene[i].max - scene[i].min;
                                    println!("GRAVITÉ (image {}) : boîte {:.2} x {:.2} x {:.2}, z min {:.3}", egui_ctx.frame_nr(), e.x, e.y, e.z, scene[i].min.z);
                                    status_msg = Some((tr(lang, "grav_ok").into(), false, std::time::Instant::now()));
                                } else { ctx.request_repaint(); }
                            }
                        }
                        // Toast d'import (succès/erreur), visible ~6 s en haut au centre.
                        if let Some((msg, is_err, t)) = &status_msg {
                            if t.elapsed().as_secs_f32() < 6.0 {
                                let area = egui::Area::new(egui::Id::new("toast")).order(egui::Order::Foreground)
                                    .anchor(egui::Align2::CENTER_TOP, egui::vec2(-panel_w * 0.5, 14.0)); // centré sur la zone de visualisation
                                let (bg, fg) = if *is_err { (egui::Color32::from_rgb(120, 40, 40), egui::Color32::from_rgb(255, 220, 210)) }
                                               else { (JAUNE, egui::Color32::BLACK) }; // bandeau d'info des actions : jaune, texte noir
                                area.show(ctx, |ui| {
                                    egui::Frame::popup(ui.style()).fill(bg).stroke(egui::Stroke::new(1.2, if *is_err { egui::Color32::from_rgb(200, 90, 90) } else { egui::Color32::from_rgb(255, 232, 150) })).show(ui, |ui| {
                                        ui.label(egui::RichText::new(msg).size(13.0).strong().color(fg));
                                    });
                                });
                                ctx.request_repaint();
                            }
                        }
                        if !viewer {
                        egui::SidePanel::right("outils").exact_width(panel_w).resizable(false).show(ctx, |ui| {
                            // --- Bas fixe du panneau : Annuler / Refaire puis jauge de calcul ---
                            // Bas du panneau : ANNULER / REFAIRE exactement sur la rangée de boutons de la barre du bas
                            // (position relevée à l'image précédente), jauge « Calcul » juste au-dessus.
                            const GAUGE_H: f32 = 30.0;
                            egui::TopBottomPanel::bottom("calcul").show_separator_line(true).exact_height(BAR_H - 4.0 + GAUGE_H).show_inside(ui, |ui| {
                                let x0 = ui.max_rect().left();
                                let wpan = panel_w - 22.0;
                                let (row_top, row_h) = bar_row.unwrap_or((ui.ctx().screen_rect().bottom() - BAR_H + 5.0, 50.0));
                                let (frac, txt) = match (&job, job_done) {
                                    (Some(j), _) => { let f = progress::get(); (f, format!("{} {} %", tr(lang, job_key(j.kind)), (f * 100.0) as u32)) }
                                    // menu Fichier (ouverture / sauvegarde) : jauge FIXE, pleine, VERTE
                                    (None, _) if file_job.is_some() => (1.0, tr(lang, "fichier_en_cours").to_string()),
                                    // OBJ-AXE → SURFACE : avancement de l'analyse des surfaces
                                    (None, _) if surf_analyse.is_some() => { let f = progress::get(); (f, format!("{} {} %", tr(lang, "surf_analyse").to_uppercase(), (f * 100.0) as u32)) }
                                    (None, Some(t)) if t.elapsed().as_secs_f32() < 1.2 => (1.0, "100 %".to_string()),
                                    _ => (0.0, String::new()),
                                };
                                let g = egui::Rect::from_min_max(egui::pos2(x0, row_top - GAUGE_H), egui::pos2(x0 + wpan, row_top - 4.0));
                                ui.allocate_ui_at_rect(g, |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    thermo_gauge_c(ui, frac, !txt.is_empty(), &txt, wpan, job.is_none() && file_job.is_some());
                                });
                                // ANNULER aussi disponible pour revenir d'une étape d'outil (tracé du texte, cadre de coupe…)
                                let etape_outil = (panneau_texte && txt_cadre.is_some()) || coupe.is_some() || coupe_arme || surf.is_some();
                                let can_undo = (hist_idx > 0 || etape_outil) && !busy;
                                let can_redo = hist_idx + 1 < history.len() && !busy;
                                let b = egui::Rect::from_min_max(egui::pos2(x0, row_top), egui::pos2(x0 + wpan, row_top + row_h));
                                ui.allocate_ui_at_rect(b, |ui| { ui.horizontal(|ui| {
                                    let w = (wpan - ui.spacing().item_spacing.x) * 0.5;
                                    if history_button(ui, tr(lang, "annuler"), can_undo, false, w, row_h).clicked() && can_undo { undo_req = true; }
                                    if history_button(ui, tr(lang, "refaire"), can_redo, true, w, row_h).clicked() && can_redo { redo_req = true; }
                                }); });
                            });
                            egui::ScrollArea::vertical().show(ui, |ui| {
                            // marge à droite : les boutons pleine largeur ne sont plus rognés par le bord
                            ui.set_max_width(panel_w - 22.0);
                            ui.add_space(6.0);
                            if panneau_imp {
                                // ===== PRÉPARATION À L'IMPRESSION (3D-IMP) : toute la colonne de droite =====
                                {
                                    let st = ui.style_mut();
                                    for (k, t) in [(egui::TextStyle::Body, 13.5), (egui::TextStyle::Button, 13.5)] { st.text_styles.insert(k, egui::FontId::proportional(t)); }
                                    st.spacing.interact_size.y = 24.0;
                                }
                                let retour = egui::Button::new(egui::RichText::new(format!("◀  {}", tr(lang, "bool_retour").to_uppercase())).size(12.5).strong().color(egui::Color32::WHITE))
                                    .fill(egui::Color32::from_gray(60)).rounding(egui::Rounding::same(6.0)).stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110)));
                                if ui.add_sized(egui::vec2(ui.available_width(), 26.0), retour).clicked() { panneau_imp = false; }
                                ui.add_space(6.0);
                                title_box(ui, &tr(lang, "imp_titre").to_uppercase(), 11.5, ui.available_width());
                                // pièces concernées (comme SAUVER : la sélection, sinon toute la scène)
                                let choix_imp: Vec<usize> = if multi.len() >= 2 { multi.clone() } else if let Some(i) = sel.filter(|&i| i < scene.len() && scene[i].nverts > 0) { vec![i] }
                                                            else { (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect() };
                                let txt_sel_ = if multi.len() >= 2 || sel.is_some() { tr(lang, "imp_sel") } else { tr(lang, "imp_tout") };
                                ui.label(egui::RichText::new(format!("{} : {}", txt_sel_, choix_imp.len())).size(12.0).color(egui::Color32::from_rgb(250, 200, 90)));
                                let sous_titre = |ui: &mut egui::Ui, t: &str| { ui.add_space(8.0); ui.label(egui::RichText::new(t).size(13.0).strong().color(egui::Color32::from_rgb(120, 180, 240))); };
                                let pastille = |ui: &mut egui::Ui, ok: Option<bool>, t: String| {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                                        let c = match ok { Some(true) => egui::Color32::from_rgb(60, 200, 90), Some(false) => egui::Color32::from_rgb(240, 70, 50), None => egui::Color32::from_rgb(240, 160, 40) };
                                        ui.painter().circle_filled(r.center(), 6.0, c);
                                        ui.add(egui::Label::new(egui::RichText::new(t).size(12.5)).wrap(true));
                                    });
                                };
                                let f = unit.factor(); let su = unit.suffix();
                                // ---- VÉRIFICATIONS ----
                                sous_titre(ui, &tr(lang, "imp_verif").to_uppercase());
                                match imp_an.1.clone() {
                                    None => { ui.label(egui::RichText::new(tr(lang, "imp_calcul")).size(12.0).color(egui::Color32::from_gray(170))); }
                                    Some(an) => {
                                        if imp_job.is_some() { ui.label(egui::RichText::new(tr(lang, "imp_calcul")).size(11.0).color(egui::Color32::from_gray(150))); }
                                        // dimensions de la pièce (à comparer au plateau de l'imprimante)
                                        ui.label(egui::RichText::new(format!("{} : {} × {} × {} {su}", tr(lang, "imp_dims"), fmt_measure(an.taille.x, unit), fmt_measure(an.taille.y, unit), fmt_measure(an.taille.z, unit))).size(12.5));
                                        // étanchéité
                                        if an.trous + an.erreurs == 0 { pastille(ui, Some(true), tr(lang, "imp_etanche").to_string()); }
                                        else {
                                            pastille(ui, Some(false), format!("{} : {} {}, {} {}", tr(lang, "imp_non_etanche"), an.trous, tr(lang, "trous"), an.erreurs, tr(lang, "faces_err")));
                                            if ui.add_enabled(!busy, egui::Button::new(egui::RichText::new(tr(lang, "repair").to_uppercase()).color(egui::Color32::BLACK)).fill(JAUNE)).clicked() { repair_req = true; }
                                        }
                                        // posée sur le plateau
                                        if an.zmin.abs() < 0.01 { pastille(ui, Some(true), tr(lang, "imp_pose").to_string()); }
                                        else {
                                            let t = if an.zmin > 0.0 { format!("{} {} {su}", tr(lang, "imp_flotte"), fmt_measure(an.zmin, unit)) } else { format!("{} {} {su}", tr(lang, "imp_sous"), fmt_measure(-an.zmin, unit)) };
                                            pastille(ui, Some(false), t);
                                            if ui.add_enabled(!busy, egui::Button::new(egui::RichText::new(tr(lang, "forme_poser").to_uppercase()).color(egui::Color32::BLACK)).fill(JAUNE)).clicked() {
                                                if multi.len() < 2 && sel.is_none() { multi = choix_imp.clone(); sel = multi.last().copied(); }
                                                forme_req = Some(FormeOp::Poser);
                                            }
                                        }
                                        // surplombs
                                        let pc = |x: f32| if an.aire > 0.0 { x / an.aire * 100.0 } else { 0.0 };
                                        let su2 = format!("{su}²");
                                        // vert : aucun surplomb ; orange : des supports seront nécessaires (le trancheur les génère)
                                        pastille(ui, if an.aire_surplombs < 1e-3 { Some(true) } else { None },
                                            if an.aire_surplombs < 1e-3 { tr(lang, "imp_pas_surplomb").to_string() }
                                            else { format!("{} : {} {su2} ({:.1} %)", tr(lang, "imp_surplombs"), fmt_measure(an.aire_surplombs / (f * f) * f, unit), pc(an.aire_surplombs)) });
                                        ui.horizontal(|ui| {
                                            case_jaune(ui, &mut imp_voir_s, &tr(lang, "imp_voir_orange"));
                                        });
                                        ui.horizontal(|ui| { ui.label(tr(lang, "imp_angle")); ui.add(egui::DragValue::new(&mut imp_angle).clamp_range(10.0..=89.0).speed(0.5).suffix(" °").max_decimals(0)).on_hover_text(tr(lang, "imp_angle_tip")); });
                                        // parois fines
                                        pastille(ui, if an.aire_fines < 1e-3 { Some(true) } else { None },
                                            if an.aire_fines < 1e-3 { format!("{} ({} {su})", tr(lang, "imp_pas_fine"), fmt_measure(imp_buse * 2.0, unit)) }
                                            else { format!("{} < {} {su} : {} {su2} ({:.1} %)", tr(lang, "imp_fines"), fmt_measure(imp_buse * 2.0, unit), fmt_measure(an.aire_fines / (f * f) * f, unit), pc(an.aire_fines)) });
                                        ui.horizontal(|ui| { case_jaune(ui, &mut imp_voir_f, &tr(lang, "imp_voir_violet")); });
                                        ui.horizontal(|ui| { ui.label(tr(lang, "imp_buse")); let mut v = imp_buse / f; if ui.add(egui::DragValue::new(&mut v).speed(0.01).max_decimals(unit.decimales()).suffix(format!(" {su}"))).on_hover_text(tr(lang, "imp_buse_tip")).changed() { imp_buse = (v * f).clamp(0.1, 2.0); } });
                                        // orientation conseillée (un seul objet)
                                        if ui.add_enabled(!busy && choix_imp.len() == 1 && grav_anim.is_none(), egui::Button::new(egui::RichText::new(tr(lang, "imp_orient").to_uppercase()).color(egui::Color32::BLACK)).fill(JAUNE).min_size(egui::vec2(ui.available_width(), 26.0)))
                                            .on_hover_text(tr(lang, "imp_orient_tip")).clicked() { imp_orient_req = true; }
                                        // ---- MATIÈRE ----
                                        sous_titre(ui, &tr(lang, "imp_matiere").to_uppercase());
                                        egui::ComboBox::from_id_source("imp_mat").width(140.0).selected_text(impression::MATIERES[imp_mat].0).show_ui(ui, |ui| {
                                            for (k, m) in impression::MATIERES.iter().enumerate() { ui.selectable_value(&mut imp_mat, k, format!("{}  ({} g/cm³)", m.0, m.1)); }
                                        });
                                        let cm3 = an.volume / 1000.0;
                                        let g = cm3 * impression::MATIERES[imp_mat].1;
                                        let metres = an.volume / (std::f32::consts::PI * 0.875 * 0.875) / 1000.0;
                                        ui.label(egui::RichText::new(format!("{} : {:.1} cm³ · {:.0} g · {:.1} m (Ø 1,75)", tr(lang, "imp_plein"), cm3, g, metres)).size(12.0));
                                        ui.label(egui::RichText::new(tr(lang, "imp_plein_note")).size(11.0).color(egui::Color32::from_gray(160)));
                                    }
                                }
                                // ---- TRANCHEUR ----
                                sous_titre(ui, &tr(lang, "imp_trancheur").to_uppercase());
                                let nt = impression::TRANCHEURS.len();
                                let installe = |k: usize| imp_installes.as_ref().and_then(|l| l.get(k).cloned()).flatten();
                                let nom_t = |k: usize| if k < nt { impression::TRANCHEURS[k].nom.to_string() } else { imp_autre.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr(lang, "imp_autre").to_string()) };
                                let sel_t = if imp_tr == usize::MAX { tr(lang, "imp_recherche").to_string() } else { nom_t(imp_tr) };
                                egui::ComboBox::from_id_source("imp_tr").width(ui.available_width() - 8.0).selected_text(sel_t).show_ui(ui, |ui| {
                                    for k in 0..nt {
                                        let t = format!("{}  —  {}", impression::TRANCHEURS[k].nom, tr(lang, if installe(k).is_some() { "imp_installe" } else { "imp_absent" }));
                                        ui.selectable_value(&mut imp_tr, k, egui::RichText::new(t).color(if installe(k).is_some() { egui::Color32::from_rgb(120, 220, 140) } else { egui::Color32::from_gray(140) }));
                                    }
                                    ui.selectable_value(&mut imp_tr, nt, tr(lang, "imp_autre"));
                                });
                                if imp_tr == nt {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(imp_autre.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "—".into())).size(11.0));
                                        if ui.button(tr(lang, "imp_choisir_prog")).clicked() && imp_choix_prog.is_none() {
                                            imp_choix_prog = Some(std::thread::spawn(|| rfd::FileDialog::new().pick_file()));
                                        }
                                    });
                                }
                                let lance = if imp_tr < nt { installe(imp_tr) } else { imp_autre.clone().map(impression::Lancement::Programme) };
                                if imp_tr < nt && lance.is_none() && imp_installes.is_some() {
                                    ui.label(egui::RichText::new(tr(lang, "imp_non_trouve")).size(11.5).color(egui::Color32::from_rgb(240, 150, 80)));
                                }
                                ui.add_space(4.0);
                                let pret = lance.is_some() && !choix_imp.is_empty() && !busy;
                                if ui.add_enabled_ui(pret, |ui| ui.add_sized(egui::vec2(ui.available_width(), 36.0), yellow_btn(&tr(lang, "imp_envoyer").to_uppercase(), 14.0))).inner
                                    .on_hover_text(tr(lang, "imp_envoyer_tip")).clicked() { imp_envoi_req = lance.clone(); }
                                if let Some((m, err)) = &imp_msg {
                                    ui.add(egui::Label::new(egui::RichText::new(m).size(12.0).color(if *err { egui::Color32::from_rgb(240, 110, 90) } else { egui::Color32::from_rgb(130, 210, 140) })).wrap(true));
                                }
                            } else if tex.actif {
                                // ===== MODE TEXTURE : texture, pinceau des zones lisses, appliquer ; le reste replié =====
                                let cible = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None });
                                tx_act = tex.panneau(ui, lang, cible.map(|i| (i, &scene[i].verts[..])), busy);
                            } else if panneau_forme {
                                // ===== Panneau MISE EN FORME (remplace le panneau d'outils) =====
                                let retour = egui::Button::new(egui::RichText::new(format!("◀  {}", tr(lang, "bool_retour").to_uppercase())).size(12.5).strong().color(egui::Color32::WHITE))
                                    .fill(egui::Color32::from_gray(60)).rounding(egui::Rounding::same(6.0)).stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110)));
                                if ui.add_sized(egui::vec2(ui.available_width(), 26.0), retour).clicked() { panneau_forme = false; }
                                ui.add_space(6.0);
                                title_box(ui, &tr(lang, "forme_btn").to_uppercase(), 14.0, ui.available_width());
                                ui.add_space(2.0);
                                // sélection : objet seul ou groupe (Ctrl+clic) ; boîte englobante commune
                                let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else { sel.filter(|&i| i < scene.len() && scene[i].nverts > 0).into_iter().collect() };
                                let (glo, ghi) = choix.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, b), &i| (a.min(scene[i].min), b.max(scene[i].max)));
                                let actif = !choix.is_empty() && !busy;
                                let info = if choix.is_empty() { tr(lang, "aucun_obj").to_string() } else { format!("{} : {}", tr(lang, "bool_nsel"), choix.len()) };
                                ui.label(egui::RichText::new(info).size(12.0).color(if choix.is_empty() { egui::Color32::from_rgb(235, 150, 90) } else { egui::Color32::from_rgb(250, 200, 90) }));
                                let petit = |ui: &mut egui::Ui, t: &str, w: f32, tip: &str| -> bool {
                                    ui.add_enabled(actif, egui::Button::new(egui::RichText::new(t).size(12.0).color(egui::Color32::BLACK)).fill(JAUNE).min_size(egui::vec2(w, 24.0)))
                                        .on_hover_text(tip).clicked()
                                };
                                let wcol = ((ui.available_width() - 8.0) / 3.0).floor();
                                let axes = ["X", "Y", "Z"];
                                // --- POSITION ---
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(tr(lang, "forme_pos")).size(12.5).strong().color(egui::Color32::from_rgb(120, 180, 240)));
                                if !choix.is_empty() {
                                    // valeurs infimes ramenées à 0 : pas de « -0.0 » affiché
                                    let net = |v: f32| if v.abs() < 0.005 { 0.0 } else { v };
                                    let mut p3 = [net(glo.x), net(glo.y), net(glo.z)];
                                    let avant = p3;
                                    ui.horizontal(|ui| {
                                        for k in 0..3 {
                                            ui.label(axes[k]);
                                            ui.add_enabled(actif, egui::DragValue::new(&mut p3[k]).speed(0.1).max_decimals(2)).on_hover_text(tr(lang, "forme_pos_tip"));
                                        }
                                    });
                                    if p3 != avant { forme_req = Some(FormeOp::Pos(Vec3::new(p3[0] - avant[0], p3[1] - avant[1], p3[2] - avant[2]))); }
                                }
                                if ui.add_enabled(actif, egui::Button::new(egui::RichText::new(tr(lang, "forme_poser")).size(12.0).color(egui::Color32::BLACK)).fill(JAUNE).min_size(egui::vec2(ui.available_width(), 24.0)))
                                    .on_hover_text(tr(lang, "forme_poser_tip")).clicked() { forme_req = Some(FormeOp::Poser); }
                                // --- ROTATION ---
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(tr(lang, "forme_rot")).size(12.5).strong().color(egui::Color32::from_rgb(120, 180, 240)));
                                for k in 0..3 {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(axes[k]).strong());
                                        if petit(ui, "−90°", 60.0, &tr(lang, "forme_rot_tip")) { forme_req = Some(FormeOp::Rot(k, -90.0)); }
                                        if petit(ui, "+90°", 60.0, &tr(lang, "forme_rot_tip")) { forme_req = Some(FormeOp::Rot(k, 90.0)); }
                                    });
                                }
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_source("f_axe_rot").width(44.0).selected_text(axes[f_axe_rot]).show_ui(ui, |ui| { for k in 0..3 { ui.selectable_value(&mut f_axe_rot, k, axes[k]); } });
                                    ui.add(egui::DragValue::new(&mut f_angle).speed(0.5).suffix(" °").max_decimals(2));
                                    if petit(ui, &tr(lang, "forme_appl"), 50.0, &tr(lang, "forme_rot_tip")) { forme_req = Some(FormeOp::Rot(f_axe_rot, f_angle)); }
                                });
                                // --- MIROIR ---
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(tr(lang, "forme_miroir")).size(12.5).strong().color(egui::Color32::from_rgb(120, 180, 240)));
                                ui.horizontal(|ui| { for k in 0..3 { if petit(ui, axes[k], wcol - 4.0, &tr(lang, "forme_miroir_tip")) { forme_req = Some(FormeOp::Miroir(k)); } } });
                                // --- ÉCHELLE ---
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(tr(lang, "forme_ech")).size(12.5).strong().color(egui::Color32::from_rgb(120, 180, 240)));
                                ui.horizontal(|ui| {
                                    ui.add(egui::DragValue::new(&mut f_ech).clamp_range(1.0..=10000.0).speed(0.5).suffix(" %").max_decimals(1));
                                    if petit(ui, &tr(lang, "forme_appl"), 50.0, &tr(lang, "forme_ech_tip")) { forme_req = Some(FormeOp::Echelle(Vec3::splat(f_ech / 100.0))); }
                                });
                                ui.horizontal(|ui| {
                                    for k in 0..3 { ui.label(axes[k]); ui.add(egui::DragValue::new(&mut f_ech3[k]).clamp_range(1.0..=10000.0).speed(0.5).suffix("%").max_decimals(0)); }
                                });
                                if ui.add_enabled(actif, egui::Button::new(egui::RichText::new(tr(lang, "forme_ech_axes")).size(12.0).color(egui::Color32::BLACK)).fill(JAUNE).min_size(egui::vec2(ui.available_width(), 24.0)))
                                    .on_hover_text(tr(lang, "forme_ech_tip")).clicked() { forme_req = Some(FormeOp::Echelle(Vec3::new(f_ech3[0], f_ech3[1], f_ech3[2]) / 100.0)); }
                                // --- ALIGNER / RÉPARTIR (plusieurs objets) ---
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(tr(lang, "forme_aligner")).size(12.5).strong().color(egui::Color32::from_rgb(120, 180, 240)));
                                let plusieurs = choix.len() >= 2 && !busy;
                                let noms = [["forme_g", "forme_cx", "forme_d"], ["forme_av", "forme_cy", "forme_ar"], ["forme_bas", "forme_mil", "forme_haut"]];
                                for k in 0..3 {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(axes[k]).strong());
                                        for m in 0..3u8 {
                                            if ui.add_enabled(plusieurs, egui::Button::new(egui::RichText::new(tr(lang, noms[k][m as usize])).size(11.0).color(egui::Color32::BLACK)).fill(JAUNE).min_size(egui::vec2((wcol - 10.0).max(40.0), 22.0)))
                                                .on_hover_text(tr(lang, "forme_aligner_tip")).clicked() { forme_req = Some(FormeOp::Aligner(k, m)); }
                                        }
                                    });
                                }
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "forme_repartir"));
                                    for k in 0..2 {
                                        if ui.add_enabled(choix.len() >= 3 && !busy, egui::Button::new(egui::RichText::new(axes[k]).size(12.0).color(egui::Color32::BLACK)).fill(JAUNE).min_size(egui::vec2(36.0, 22.0)))
                                            .on_hover_text(tr(lang, "forme_repartir_tip")).clicked() { forme_req = Some(FormeOp::Repartir(k)); }
                                    }
                                });
                                // --- DUPLIQUER ---
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(tr(lang, "forme_dup")).size(12.5).strong().color(egui::Color32::from_rgb(120, 180, 240)));
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "forme_lin"));
                                    ui.add(egui::DragValue::new(&mut f_n).clamp_range(2..=100).speed(0.1).prefix("× "));
                                    egui::ComboBox::from_id_source("f_dir").width(40.0).selected_text(axes[f_dir]).show_ui(ui, |ui| { for k in 0..3 { ui.selectable_value(&mut f_dir, k, axes[k]); } });
                                });
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "forme_pas"));
                                    ui.add(egui::DragValue::new(&mut f_pas).clamp_range(0.1..=5000.0).speed(0.2).suffix(" mm").max_decimals(2));
                                    if petit(ui, &tr(lang, "forme_appl"), 50.0, &tr(lang, "forme_lin_tip")) { forme_req = Some(FormeOp::Lineaire(f_n, f_pas, f_dir)); }
                                });
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "forme_circ"));
                                    ui.add(egui::DragValue::new(&mut f_cn).clamp_range(2..=100).speed(0.1).prefix("× "));
                                });
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "forme_rayon"));
                                    ui.add(egui::DragValue::new(&mut f_cr).clamp_range(0.1..=5000.0).speed(0.2).suffix(" mm").max_decimals(2));
                                    if petit(ui, &tr(lang, "forme_appl"), 50.0, &tr(lang, "forme_circ_tip")) { forme_req = Some(FormeOp::Circulaire(f_cn, f_cr)); }
                                });
                            } else if panneau_texte {
                                // ===== Sous-panneau TEXTE & RELIEF (toute la colonne de droite) =====
                                let retour = egui::Button::new(egui::RichText::new(format!("◀  {}", tr(lang, "bool_btn").to_uppercase())).size(12.5).strong().color(egui::Color32::WHITE))
                                    .fill(egui::Color32::from_gray(60)).rounding(egui::Rounding::same(6.0)).stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110)));
                                if ui.add_sized(egui::vec2(ui.available_width(), 26.0), retour).clicked() { panneau_texte = false; }
                                ui.add_space(6.0);
                                title_box(ui, &tr(lang, if txt_image { "img_btn" } else { "texte_btn" }).to_uppercase(), 14.0, ui.available_width());
                                ui.add_space(4.0);
                                // champs plus grands et plus lisibles dans ce panneau
                                {
                                    let st = ui.style_mut();
                                    for (k, t) in [(egui::TextStyle::Body, 14.5), (egui::TextStyle::Button, 14.0), (egui::TextStyle::Small, 12.0)] {
                                        st.text_styles.insert(k, egui::FontId::proportional(t));
                                    }
                                    st.spacing.interact_size.y = 26.0;
                                    st.spacing.item_spacing.y = 6.0;
                                }
                                if txt_image {
                                    // ===== INSÈRE IMAGE : choix du fichier, aperçu noir & blanc avec le tracé vectorisé =====
                                    if ui.add_enabled(img_choix.is_none(), egui::Button::new(egui::RichText::new(tr(lang, "img_choisir")).size(13.0)).min_size(egui::vec2(ui.available_width(), 28.0))).clicked() {
                                        img_choix = Some(std::thread::spawn(|| rfd::FileDialog::new().set_directory(paths::start_dir())
                                            .add_filter("Images", &["png", "jpg", "jpeg", "bmp"]).pick_file()));
                                    }
                                    match img.as_ref() {
                                        None => { ui.add(egui::Label::new(egui::RichText::new(tr(lang, "img_aucune")).size(12.0).color(egui::Color32::from_rgb(235, 150, 90))).wrap(true)); }
                                        Some(im) => {
                                            ui.label(egui::RichText::new(format!("{}  ({} × {} px)", im.nom, im.w, im.h)).size(11.5).color(egui::Color32::from_gray(190)));
                                            if img_tex.as_ref().map_or(true, |t| t.0 != im.generation) {
                                                let px: Vec<egui::Color32> = im.masque.iter().map(|&b| if b { egui::Color32::from_gray(15) } else { egui::Color32::from_gray(245) }).collect();
                                                let ci = egui::ColorImage { size: [im.w, im.h], pixels: px };
                                                img_tex = Some((im.generation, ui.ctx().load_texture("img_vecto", ci, egui::TextureOptions::NEAREST)));
                                            }
                                            if let Some((_, tex)) = &img_tex {
                                                let k = (ui.available_width() / im.w as f32).min(170.0 / im.h as f32);
                                                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), im.h as f32 * k), egui::Sense::hover());
                                                let zone = egui::Rect::from_center_size(r.center(), egui::vec2(im.w as f32 * k, im.h as f32 * k));
                                                ui.painter().image(tex.id(), zone, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                                                // tracé vectorisé par-dessus (rouge, comme dans les vues)
                                                let rouge = egui::Stroke::new(1.2, egui::Color32::from_rgb(235, 45, 40));
                                                let pt = |q: [f64; 2]| egui::pos2(zone.left() + q[0] as f32 * k, zone.bottom() - q[1] as f32 * k);
                                                for c in &im.polys {
                                                    let mut v: Vec<egui::Pos2> = c.iter().map(|&q| pt(q)).collect();
                                                    v.push(v[0]);
                                                    ui.painter().add(egui::Shape::line(v, rouge));
                                                }
                                            }
                                            let np: usize = im.polys.iter().map(|c| c.len()).sum();
                                            ui.label(egui::RichText::new(format!("{} {} · {} {}", im.polys.len(), tr(lang, "img_stats"), np, tr(lang, "img_points"))).size(11.5).color(egui::Color32::from_gray(190)));
                                            if im.polys.is_empty() { ui.label(egui::RichText::new(tr(lang, "img_vide")).size(11.5).color(egui::Color32::from_rgb(235, 150, 90))); }
                                        }
                                    }
                                } else {
                                // la police choisie est chargée dans l'interface : la saisie s'affiche avec elle
                                if let (Some(i), Some(liste)) = (txt_police, texte::polices()) {
                                    if txt_police_ui != Some(i) {
                                        if let Some(pol) = liste.get(i) {
                                            if let Some(oct) = texte::octets(pol) {
                                                let mut defs = egui::FontDefinitions::default();
                                                let mut fd = egui::FontData::from_owned(oct);
                                                fd.index = pol.index;
                                                defs.font_data.insert("saisie".into(), fd);
                                                let mut fam = vec!["saisie".to_string()];
                                                fam.extend(defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
                                                defs.families.insert(egui::FontFamily::Name("saisie".into()), fam);
                                                ui.ctx().set_fonts(defs);
                                            }
                                        }
                                        txt_police_ui = Some(i);
                                    }
                                }
                                let police_saisie = if txt_police_ui.is_some() && ui.fonts(|f| f.families().contains(&egui::FontFamily::Name("saisie".into()))) {
                                    egui::FontId::new(22.0, egui::FontFamily::Name("saisie".into()))
                                } else { egui::FontId::proportional(20.0) };
                                // saisie du texte (plusieurs lignes possibles), grande, dans la police choisie
                                ui.add(egui::TextEdit::multiline(&mut txt_saisie).desired_rows(3).desired_width(f32::INFINITY).font(police_saisie).hint_text(tr(lang, "texte_saisie")));
                                // justification : gauche / centre / droite
                                ui.horizontal(|ui| {
                                    for (j, tip) in [(0u8, "texte_j_g"), (1, "texte_j_c"), (2, "texte_j_d")] {
                                        let (r, resp) = ui.allocate_exact_size(egui::vec2(44.0, 30.0), egui::Sense::click());
                                        let actif = txt_just == j;
                                        ui.painter().rect(r, 5.0, if actif { JAUNE } else if resp.hovered() { egui::Color32::from_gray(70) } else { egui::Color32::from_gray(48) }, egui::Stroke::new(1.0, egui::Color32::from_gray(110)));
                                        draw_justif(ui.painter(), r.shrink2(egui::vec2(9.0, 4.0)), j, if actif { egui::Color32::BLACK } else { egui::Color32::from_gray(220) });
                                        if resp.on_hover_text(tr(lang, tip)).clicked() { txt_just = j; }
                                    }
                                });
                                }
                                ui.add_space(6.0);
                                // le texte se pose de lui-même sur la face visée au centre de la vue ; ce bouton l'y replace
                                if ui.add_sized(egui::vec2(ui.available_width(), 32.0), yellow_btn(&tr(lang, "texte_recentrer").to_uppercase(), 12.5)).on_hover_text(tr(lang, "texte_aide")).clicked() {
                                    txt_cadre = None;
                                }
                                ui.add(egui::Label::new(egui::RichText::new(tr(lang, "texte_aide")).size(11.0).color(egui::Color32::from_gray(170))).wrap(true));
                                // dimensions du cadre (unité en cours), modifiables
                                if let Some(c) = txt_cadre.as_mut() {
                                    let f = unit.factor();
                                    let mut champ = |ui: &mut egui::Ui, nom: &str, val: &mut f32| {
                                        let mut x = *val / f;
                                        ui.horizontal(|ui| { ui.label(nom); if ui.add(egui::DragValue::new(&mut x).speed(0.05).max_decimals(unit.decimales()).suffix(format!(" {}", unit.suffix()))).changed() { *val = (x * f).max(0.05); } });
                                    };
                                    // X / Y découlent du texte (police, taille) : affichés seulement
                                    ui.label(egui::RichText::new(format!("X {} × Y {} {}", fmt_measure(c.lx, unit), fmt_measure(c.ly, unit), unit.suffix())).size(12.0).color(egui::Color32::from_gray(190)));
                                    champ(ui, &tr(lang, "texte_prof"), &mut c.prof);
                                    let mut deg = c.angle.to_degrees();
                                    ui.horizontal(|ui| { ui.label(tr(lang, "texte_angle")); if ui.add(egui::DragValue::new(&mut deg).speed(0.5).suffix(" °").max_decimals(1)).changed() { c.angle = deg.to_radians(); } });
                                }
                                ui.add_space(4.0);
                                if radio_jaune(ui, txt_relief, &tr(lang, "texte_relief")).clicked() { txt_relief = true; }
                                if radio_jaune(ui, !txt_relief, &tr(lang, "texte_grave")).clicked() { txt_relief = false; }
                                case_jaune(ui, &mut txt_suivre, &tr(lang, "texte_suivre").to_uppercase()).on_hover_text(tr(lang, "texte_suivre_tip"));
                                ui.add_space(4.0);
                                let pret = txt_cadre.is_some() && !busy && if txt_image { img.as_ref().map_or(false, |i| !i.polys.is_empty()) }
                                                                           else { !txt_saisie.trim().is_empty() && txt_police.is_some() };
                                if ui.add_enabled_ui(pret, |ui| ui.add_sized(egui::vec2(ui.available_width(), 34.0), yellow_btn(&tr(lang, "texte_appl").to_uppercase(), 13.5))).inner
                                    .on_hover_text(tr(lang, "texte_appl_tip")).clicked() { txt_appl = true; }
                                if txt_image {
                                    // ===== VECTORISATION : réduction en noir & blanc, tolérance du tracé, taille =====
                                    ui.add_space(10.0);
                                    title_box(ui, &tr(lang, "img_vecto").to_uppercase(), 13.0, ui.available_width());
                                    egui::ScrollArea::vertical().id_source("vecto").auto_shrink([false, true]).show(ui, |ui| {
                                        ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(60.0);
                                        let f = unit.factor();
                                        let mut x = img_largeur / f;
                                        ui.horizontal(|ui| { ui.label(tr(lang, "img_largeur")); if ui.add(egui::DragValue::new(&mut x).speed(0.1).max_decimals(unit.decimales()).suffix(format!(" {}", unit.suffix()))).on_hover_text(tr(lang, "img_largeur_tip")).changed() { img_largeur = (x * f).max(0.5); } });
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(tr(lang, "img_reduc")).strong());
                                        for (m, k) in [(vectorise::Mode::Seuil, "img_m_seuil"), (vectorise::Mode::Auto, "img_m_auto"), (vectorise::Mode::Adaptatif, "img_m_adapt"), (vectorise::Mode::Couleurs, "img_m_coul")] {
                                            if radio_jaune(ui, img_prm.mode == m, &tr(lang, k)).clicked() { img_prm.mode = m; }
                                        }
                                        match img_prm.mode {
                                            vectorise::Mode::Seuil => { ui.label(tr(lang, "img_seuil")); ui.add(egui::Slider::new(&mut img_prm.seuil, 1..=254)).on_hover_text(tr(lang, "img_seuil_tip")); }
                                            vectorise::Mode::Adaptatif => { ui.label(tr(lang, "img_ecart")); ui.add(egui::Slider::new(&mut img_prm.ecart, 0..=60)).on_hover_text(tr(lang, "img_ecart_tip")); }
                                            vectorise::Mode::Couleurs => {
                                                ui.label(tr(lang, "img_ncoul"));
                                                ui.add(egui::Slider::new(&mut img_prm.n_coul, 2..=12));
                                                // palette : clic = couleur pleine (cadre jaune + coche noire) ou vide
                                                if let Some(im) = img.as_ref() {
                                                    ui.horizontal_wrapped(|ui| {
                                                        for (i, c) in im.palette().iter().enumerate() {
                                                            let (r, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
                                                            let plein = img_prm.pleines.get(i).copied().unwrap_or(false);
                                                            ui.painter().rect_filled(r, 4.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                                                            ui.painter().rect_stroke(r, 4.0, if plein { egui::Stroke::new(3.0, JAUNE) } else { egui::Stroke::new(1.0, egui::Color32::from_gray(110)) });
                                                            if plein {
                                                                ui.painter().rect_filled(egui::Rect::from_center_size(r.center(), egui::vec2(12.0, 12.0)), 2.0, egui::Color32::BLACK);
                                                                ui.painter().rect_stroke(egui::Rect::from_center_size(r.center(), egui::vec2(12.0, 12.0)), 2.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
                                                            }
                                                            if resp.on_hover_text(tr(lang, "img_coul_tip")).clicked() { if let Some(b) = img_prm.pleines.get_mut(i) { *b = !*b; } }
                                                        }
                                                    });
                                                }
                                            }
                                            vectorise::Mode::Auto => {}
                                        }
                                        case_jaune(ui, &mut img_prm.inverser, &tr(lang, "img_inverser").to_uppercase()).on_hover_text(tr(lang, "img_inverser_tip"));
                                        case_jaune(ui, &mut img_prm.sans_cadre, &tr(lang, "img_sans_cadre").to_uppercase()).on_hover_text(tr(lang, "img_sans_cadre_tip"));
                                        ui.add_space(4.0);
                                        // libellé au-dessus, curseur sur toute la largeur (rien n'est rogné dans la colonne étroite)
                                        ui.label(tr(lang, "img_flou"));
                                        ui.add(egui::Slider::new(&mut img_prm.flou, 0..=8).suffix(" px")).on_hover_text(tr(lang, "img_flou_tip"));
                                        ui.label(tr(lang, "img_res"));
                                        ui.add(egui::Slider::new(&mut img_prm.res, 100..=1500).step_by(50.0).suffix(" px")).on_hover_text(tr(lang, "img_res_tip"));
                                        ui.label(tr(lang, "img_nettoyage"));
                                        ui.add(egui::Slider::new(&mut img_prm.nettoyage, 0.0..=400.0).logarithmic(true).suffix(" px²")).on_hover_text(tr(lang, "img_nettoyage_tip"));
                                        ui.label(tr(lang, "img_tol"));
                                        ui.add(egui::Slider::new(&mut img_prm.tolerance, 0.0..=10.0).max_decimals(2).suffix(" px")).on_hover_text(tr(lang, "img_tol_tip"));
                                    });
                                } else {
                                // ===== POLICE : taille et aperçu en direct =====
                                ui.add_space(10.0);
                                title_box(ui, &tr(lang, "texte_police").to_uppercase(), 13.0, ui.available_width());
                                {
                                    let f = unit.factor();
                                    let mut x = txt_taille / f;
                                    ui.horizontal(|ui| { ui.label(tr(lang, "texte_taille")); if ui.add(egui::DragValue::new(&mut x).speed(0.05).max_decimals(unit.decimales()).suffix(format!(" {}", unit.suffix()))).on_hover_text(tr(lang, "texte_taille_tip")).changed() { txt_taille = (x * f).max(0.1); } });
                                }
                                ui.add(egui::TextEdit::singleline(&mut txt_filtre).desired_width(f32::INFINITY).hint_text(tr(lang, "texte_filtre")));
                                match texte::polices() {
                                    None => { ui.label(egui::RichText::new(tr(lang, "texte_charge")).size(11.5).color(egui::Color32::from_gray(170))); ui.ctx().request_repaint_after(std::time::Duration::from_millis(300)); }
                                    Some(liste) => {
                                        if txt_police.is_none() {
                                            txt_police = liste.iter().position(|p| p.nom == "DejaVu Sans" || p.nom == "Arial").or(if liste.is_empty() { None } else { Some(0) });
                                        }
                                        let filtre = txt_filtre.to_lowercase();
                                        let idx: Vec<usize> = (0..liste.len()).filter(|&i| filtre.is_empty() || liste[i].nom.to_lowercase().contains(&filtre)).collect();
                                        if txt_apercus.len() > 600 { txt_apercus.clear(); }
                                        // la liste descend jusqu'en bas de la colonne, avec ascenseur toujours visible ;
                                        // chaque police : son nom, écrit dans la police elle-même
                                        let haut_liste = (ui.clip_rect().bottom() - ui.cursor().top() - 6.0).max(180.0);
                                        // ascenseur large et bleu (bien visible)
                                        {
                                            let st = ui.style_mut();
                                            let mut sc = egui::style::ScrollStyle::solid();
                                            sc.bar_width = 16.0; sc.handle_min_length = 36.0;
                                            st.spacing.scroll = sc;
                                            st.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(55, 135, 230);
                                            st.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(90, 165, 250);
                                            st.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(130, 190, 255);
                                            st.visuals.extreme_bg_color = egui::Color32::from_rgb(25, 35, 50);
                                        }
                                        egui::ScrollArea::vertical().id_source("polices").max_height(haut_liste).auto_shrink([false, false])
                                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                                            .show_rows(ui, 34.0, idx.len(), |ui, rangee| {
                                            for &i in &idx[rangee] {
                                                let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width() - 4.0, 32.0), egui::Sense::click());
                                                let choisi = txt_police == Some(i);
                                                ui.painter().rect_filled(r, 4.0, if choisi { egui::Color32::from_rgb(70, 62, 25) } else if resp.hovered() { egui::Color32::from_gray(58) } else { egui::Color32::from_gray(40) });
                                                if choisi { ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(1.5, JAUNE)); }
                                                let cle = (i, String::new());
                                                if !txt_apercus.contains_key(&cle) {
                                                    let ap = texte::contours(&liste[i], &liste[i].nom, 1.0, 0, 0.0).map(|(p, w, h)| (texte::triangles(&p), w, h)).unwrap_or_default();
                                                    txt_apercus.insert(cle.clone(), ap);
                                                }
                                                let coul = if choisi { JAUNE } else { egui::Color32::from_gray(235) };
                                                match txt_apercus.get(&cle).filter(|a| a.2 > 0.0 && !a.0.is_empty()) {
                                                    Some((tris, w, h)) => {
                                                        let zone = r.shrink2(egui::vec2(6.0, 4.0));
                                                        let k = (zone.height() / h).min(zone.width() / w.max(1e-3));
                                                        let o = egui::pos2(zone.left() + w * 0.5 * k, zone.center().y);
                                                        let mut m = egui::Mesh::default();
                                                        for t in tris {
                                                            let base = m.vertices.len() as u32;
                                                            for q in t { m.colored_vertex(egui::pos2(o.x + q[0] * k, o.y - q[1] * k), coul); }
                                                            m.add_triangle(base, base + 1, base + 2);
                                                        }
                                                        ui.painter().add(egui::Shape::mesh(m));
                                                    }
                                                    None => { ui.painter().text(r.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, &liste[i].nom, egui::FontId::proportional(14.0), coul); }
                                                }
                                                if resp.on_hover_text(&liste[i].nom).clicked() { txt_police = Some(i); }
                                            }
                                        });
                                    }
                                }
                                }
                            } else if panneau_bool {
                                // ===== Panneau BOOLÉENS (remplace le panneau d'outils) =====
                                let retour = egui::Button::new(egui::RichText::new(format!("◀  {}", tr(lang, "bool_retour").to_uppercase())).size(12.5).strong().color(egui::Color32::WHITE))
                                    .fill(egui::Color32::from_gray(60)).rounding(egui::Rounding::same(6.0)).stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110)));
                                if ui.add_sized(egui::vec2(ui.available_width(), 26.0), retour).clicked() { panneau_bool = false; }
                                ui.add_space(6.0);
                                title_box(ui, &tr(lang, "bool_btn").to_uppercase(), 14.0, ui.available_width());
                                ui.add_space(4.0);
                                ui.add(egui::Label::new(egui::RichText::new(tr(lang, "bool_aide")).size(12.0)).wrap(true));
                                ui.add_space(6.0);
                                // objets sélectionnés, dans l'ordre des clics (le dernier sert d'outil à SOUSTRAIRE)
                                let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else { sel.filter(|&i| i < scene.len()).into_iter().collect() };
                                egui::Frame::none().stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(110))).rounding(egui::Rounding::same(6.0)).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(egui::RichText::new(format!("{} : {}", tr(lang, "bool_nsel"), choix.len())).size(12.5).strong());
                                    for (k, &i) in choix.iter().enumerate() {
                                        let dernier = k + 1 == choix.len() && choix.len() >= 2;
                                        let t = format!("{} {} · {} {}", tr(lang, "bool_obj"), i + 1, scene[i].tris.len(), tr(lang, "triangles"));
                                        ui.label(egui::RichText::new(t).size(12.0).color(if dernier { egui::Color32::from_rgb(250, 160, 70) } else { ui.visuals().text_color() }));
                                        if dernier { ui.label(egui::RichText::new(format!("      ({})", tr(lang, "bool_dernier"))).size(11.0).color(egui::Color32::from_rgb(250, 160, 70))); }
                                    }
                                });
                                ui.add_space(4.0);
                                if ui.add_enabled(scene.iter().filter(|o| o.nverts > 0).count() >= 2, egui::Button::new(egui::RichText::new(format!("{}  (Ctrl+A)", tr(lang, "bool_tout"))).size(12.0))).clicked() {
                                    multi = (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect();
                                    sel = multi.last().copied(); selection = None; gizmo = None;
                                }
                                ui.add_space(8.0);
                                let pret = choix.len() >= 2 && !busy;
                                for (op, k, tip) in [(booleen::Op::Union, "bool_union", "bool_union_tip"), (booleen::Op::Soustraction, "bool_soustr", "bool_soustr_tip"), (booleen::Op::Intersection, "bool_inter", "bool_inter_tip")] {
                                    if bool_op_button(ui, op, &tr(lang, k).to_uppercase(), pret).on_hover_text(tr(lang, tip)).clicked() { bool_req = Some(op); }
                                    ui.add_space(4.0);
                                }
                                ui.checkbox(&mut bool_garder_b, tr(lang, "bool_garder")).on_hover_text(tr(lang, "bool_garder_tip"));
                                if !pret && !busy {
                                    ui.add_space(4.0);
                                    ui.add(egui::Label::new(egui::RichText::new(tr(lang, "bool_choisir")).size(11.5).color(egui::Color32::from_rgb(235, 150, 90))).wrap(true));
                                }
                                // ===== DÉCOUPE (ciseaux) =====
                                ui.add_space(12.0);
                                title_box(ui, &tr(lang, "coupe_btn").to_uppercase(), 14.0, ui.available_width());
                                ui.add_space(4.0);
                                ui.add(egui::Label::new(egui::RichText::new(tr(lang, "coupe_aide")).size(12.0)).wrap(true));
                                ui.add_space(6.0);
                                let rouge = egui::Color32::from_rgb(220, 50, 45);
                                let (cr, cresp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
                                {
                                    let pnt = ui.painter();
                                    // jaune à contour rouge ; rouge plein quand les ciseaux sont armés
                                    let fond = if coupe_arme { rouge } else if cresp.hovered() { egui::Color32::from_rgb(250, 222, 120) } else { JAUNE };
                                    pnt.rect(cr, 6.0, fond, egui::Stroke::new(2.0, rouge));
                                    // icône + libellé centrés ensemble dans le bouton
                                    let t = if coupe_arme { tr(lang, ["coupe_t1", "coupe_t2", "coupe_t3"][coupe_etape.min(2) as usize]).to_string() } else { tr(lang, "coupe_btn").to_uppercase() };
                                    let police = egui::FontId::proportional(if coupe_arme { 11.5 } else { 13.5 });
                                    let tw = pnt.layout_no_wrap(t.clone(), police.clone(), egui::Color32::BLACK).size().x;
                                    let x0 = (cr.center().x - (34.0 + 8.0 + tw) * 0.5).max(cr.left() + 4.0);
                                    let ic = egui::Rect::from_center_size(egui::pos2(x0 + 17.0, cr.center().y), egui::vec2(34.0, 34.0));
                                    draw_ciseaux(pnt, ic, if coupe_arme { egui::Color32::WHITE } else { egui::Color32::from_rgb(225, 45, 40) });
                                    pnt.text(egui::pos2(x0 + 42.0, cr.center().y), egui::Align2::LEFT_CENTER, t, police, if coupe_arme { egui::Color32::WHITE } else { egui::Color32::BLACK });
                                }
                                if cresp.on_hover_text(tr(lang, "coupe_tip")).clicked() {
                                    coupe_arme = !coupe_arme;
                                    // désarmés en plein tracé : trait inachevé abandonné, l déjà en cours = gardé tel quel
                                    if !coupe_arme { if coupe_etape == 1 { coupe = None; } coupe_etape = 0; coupe_tir = None; }
                                    // découpe : passage direct en 4 vues (la vue d'avant revient à la fin)
                                    if coupe_arme && mode != ViewMode::Quad { mode_avant_coupe = Some(mode); mode = ViewMode::Quad; }
                                }
                                if let Some(mut c) = coupe {
                                    ui.add_space(4.0);
                                    ui.add(egui::Label::new(egui::RichText::new(tr(lang, "coupe_objet")).size(11.5).color(if coupe_sel { egui::Color32::from_rgb(255, 210, 60) } else { ui.visuals().weak_text_color() })).wrap(true));
                                    if let (Some((u, _, l)), Some(n)) = (c.reperes(), c.perp_plan()) {
                                        // angle de la ligne et décalage perpendiculaire, modifiables au clavier / à la souris
                                        let (iu, iw) = axes_plan(c.e);
                                        let mut ang = u[iw].atan2(u[iu]).to_degrees();
                                        let mil = (c.p0 + c.p1) * 0.5;
                                        let mut dec = mil.dot(n);
                                        let (ang0, dec0) = (ang, dec);
                                        let (incl0, dl0) = (c.incl, c.l);
                                        ui.add_space(6.0);
                                        egui::Grid::new("coupe_grid").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
                                            ui.label(egui::RichText::new(tr(lang, "coupe_angle")).size(12.0));
                                            ui.add(egui::DragValue::new(&mut ang).speed(0.5).suffix(" °").max_decimals(1));
                                            ui.end_row();
                                            ui.label(egui::RichText::new(tr(lang, "coupe_pos")).size(12.0)).on_hover_text(tr(lang, "coupe_pos_tip"));
                                            ui.add(egui::DragValue::new(&mut dec).speed(0.2).suffix(" mm").max_decimals(2)).on_hover_text(tr(lang, "coupe_pos_tip"));
                                            ui.end_row();
                                            ui.label(egui::RichText::new(tr(lang, "coupe_incl")).size(12.0)).on_hover_text(tr(lang, "coupe_incl_tip"));
                                            ui.add(egui::DragValue::new(&mut c.incl).speed(0.5).suffix(" °").max_decimals(1)).on_hover_text(tr(lang, "coupe_incl_tip"));
                                            ui.end_row();
                                            ui.label(egui::RichText::new("l").size(12.0)).on_hover_text(tr(lang, "coupe_l_tip"));
                                            ui.add(egui::DragValue::new(&mut c.l).speed(0.2).clamp_range(0.0..=1.0e5).suffix(" mm").max_decimals(2)).on_hover_text(tr(lang, "coupe_l_tip"));
                                            ui.end_row();
                                        });
                                        if c.incl != incl0 || c.l != dl0 { coupe = Some(c); }
                                        if ang != ang0 || dec != dec0 {
                                            let a = ang.to_radians();
                                            let mut u2 = Vec3::ZERO; u2[iu] = a.cos(); u2[iw] = a.sin();
                                            let n2 = u2.cross(c.axe()).normalize();
                                            let m2 = mil - n * dec0 + n2 * dec;
                                            c.p0 = m2 - u2 * (l * 0.5); c.p1 = m2 + u2 * (l * 0.5);
                                            coupe = Some(c);
                                        }
                                    }
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(tr(lang, "coupe_garder")).size(12.0));
                                    ui.horizontal_wrapped(|ui| {
                                        for (v, k) in [(0u8, "coupe_deux"), (1, "coupe_a"), (2, "coupe_b")] {
                                            ui.radio_value(&mut coupe_garder, v, egui::RichText::new(tr(lang, k)).size(12.0));
                                        }
                                    });
                                    ui.add_enabled(coupe_garder == 0, egui::Checkbox::new(&mut coupe_ecart, tr(lang, "coupe_ecart")));
                                    ui.add_space(4.0);
                                    let go = egui::Button::new(egui::RichText::new(tr(lang, "coupe_go").to_uppercase()).size(13.5).strong().color(egui::Color32::WHITE))
                                        .fill(if busy { rouge.gamma_multiply(0.4) } else { rouge }).rounding(egui::Rounding::same(6.0));
                                    if ui.add_enabled_ui(!busy, |ui| ui.add_sized(egui::vec2(ui.available_width(), 32.0), go)).inner.on_hover_text(tr(lang, "coupe_go_tip")).clicked() { coupe_req = true; }
                                    ui.add_space(2.0);
                                    if ui.add_sized(egui::vec2(ui.available_width(), 24.0), egui::Button::new(egui::RichText::new(tr(lang, "coupe_effacer")).size(12.0))).clicked() { coupe = None; }
                                }
                                // ===== TEXTE & RELIEF : entrée du sous-panneau (toute la colonne) =====
                                ui.add_space(12.0);
                                title_box(ui, &tr(lang, "texte_btn").to_uppercase(), 14.0, ui.available_width());
                                ui.add_space(4.0);
                                if ui.add_sized(egui::vec2(ui.available_width(), 34.0), yellow_btn(&tr(lang, "texte_btn").to_uppercase(), 13.5)).on_hover_text(tr(lang, "texte_tip")).clicked() {
                                    panneau_texte = true; txt_image = false; txt_cadre = None; // posé de lui-même sur la face visée
                                }
                                ui.add_space(4.0);
                                // INSÈRE IMAGE : même pose que le texte ; le sélecteur s'ouvre tout de suite s'il n'y a pas encore d'image
                                if ui.add_sized(egui::vec2(ui.available_width(), 34.0), yellow_btn(&tr(lang, "img_btn").to_uppercase(), 13.5)).on_hover_text(tr(lang, "img_tip")).clicked() {
                                    panneau_texte = true; txt_image = true; txt_cadre = None;
                                    if img.is_none() && img_choix.is_none() {
                                        img_choix = Some(std::thread::spawn(|| rfd::FileDialog::new().set_directory(paths::start_dir())
                                            .add_filter("Images", &["png", "jpg", "jpeg", "bmp"]).pick_file()));
                                    }
                                }
                            } else {
                            // --- AFFICHAGE : modes de visualisation (au-dessus des outils) ---
                            // Titre « Affichage » + engrenage des Options dans l'angle droit de la boîte à outils.
                            ui.horizontal(|ui| {
                                // titre + engrenage sur la MÊME ligne (même hauteur 24), engrenage collé à droite
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let tw = ui.available_width() - 28.0;
                                title_box(ui, &tr(lang, "affichage").to_uppercase(), 14.0, tw);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let oid2 = egui::Id::new("pop_options");
                                    // test sans clic : PATCH3D_OPEN=options ouvre ce menu au démarrage
                                    if std::env::var("PATCH3D_OPEN").as_deref() == Ok("options") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(oid2)); }
                                    let ores = gear_button(ui, ui.memory(|m| m.is_popup_open(oid2)), 24.0).on_hover_text(tr(lang, "options"));
                                    if ores.clicked() { ui.memory_mut(|m| m.toggle_popup(oid2)); }
                                    egui::popup::popup_above_or_below_widget(ui, oid2, &ores, egui::AboveOrBelow::Below, |ui| {
                                        let blue = title_blue(ui); // même bleu que les titres (accordé au thème)
                                        // Cadre OPAQUE + bordure bleue autour de tout le menu Options.
                                        egui::Frame::none()
                                            .fill(if dark { egui::Color32::from_gray(28) } else { egui::Color32::from_gray(245) })
                                            .stroke(egui::Stroke::new(1.5, blue))
                                            .rounding(egui::Rounding::same(6.0))
                                            .inner_margin(egui::Margin::same(9.0))
                                            .show(ui, |ui| {
                                            ui.set_min_width(240.0);
                                            ui.set_max_width(260.0);
                                            // lisibilité : tous les textes du menu en clair franc (sombre) / noir (clair)
                                            ui.visuals_mut().override_text_color = Some(if dark { egui::Color32::from_gray(240) } else { egui::Color32::from_gray(15) });
                                            if !dark { ui.visuals_mut().selection.bg_fill = egui::Color32::from_rgb(160, 200, 245); } // sélection claire : texte noir lisible
                                            // Activer GPU : visible seulement si un GPU de calcul est utilisable
                                            if let Some((g, _)) = &gpu {
                                                if gpu_check(ui, gpu_on, tr(lang, "gpu_on")).on_hover_text(perf_tooltip(lang, &g.name)).clicked() {
                                                    gpu_on = !gpu_on;
                                                    perf::GPU_CHECKED.store(gpu_on, std::sync::atomic::Ordering::Relaxed);
                                                    paths::set_flag("gpu", gpu_on);
                                                }
                                                ui.separator();
                                            }
                                            // 3D-Patch en visionneuse 3D : application par défaut des formats 3D
                                            let on = assoc_etat.load(std::sync::atomic::Ordering::Relaxed);
                                            if gpu_check(ui, on, tr(lang, "assoc_on")).on_hover_text(tr(lang, "assoc_tip")).clicked() && assoc_job.is_none() {
                                                assoc_etat.store(!on, std::sync::atomic::Ordering::Relaxed);
                                                assoc_job = Some(std::thread::spawn(move || if on { assoc::disable() } else { assoc::enable() }));
                                            }
                                            ui.separator();
                                            title_box(ui, tr(lang, "langue"), 13.0, ui.available_width());
                                            for l in LANGS {
                                                if ui.selectable_label(lang == l, l.name()).clicked() { lang = l; }
                                            }
                                            ui.separator();
                                            title_box(ui, tr(lang, "theme"), 13.0, ui.available_width());
                                            if ui.selectable_label(dark, tr(lang, "sombre")).clicked() { dark = true; }
                                            if ui.selectable_label(!dark, tr(lang, "clair")).clicked() { dark = false; }
                                            // Séparateur BLEU (couleur des langues) au-dessus de la licence.
                                            ui.add_space(6.0);
                                            let w = ui.available_width();
                                            let (sr, _) = ui.allocate_exact_size(egui::vec2(w, 2.0), egui::Sense::hover());
                                            ui.painter().rect_filled(sr, 1.0, blue);
                                            ui.add_space(6.0);
                                            // --- Info & licence ---
                                            title_box(ui, &format!("3D-Patch — {}", tr(lang, "a_propos")), 13.0, ui.available_width());
                                            ui.label(egui::RichText::new(format!("{} {}", tr(lang, "version"), env!("CARGO_PKG_VERSION"))).size(12.5));
                                            ui.label(egui::RichText::new(tr(lang, "modeleur")).size(12.5));
                                            ui.separator();
                                            title_box(ui, tr(lang, "licence"), 13.0, ui.available_width());
                                            ui.horizontal(|ui| {
                                                if let Some(tex) = &author_tex {
                                                    ui.image((tex.id(), egui::vec2(64.0, 64.0)));
                                                }
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Alain Valette-Clary").strong().size(13.0));
                                                    ui.label(egui::RichText::new("ΩiA-Group").size(12.5));
                                                    ui.label(egui::RichText::new("INPI DSO2025023838").size(12.5));
                                                    ui.hyperlink_to(egui::RichText::new("LinkedIn").size(12.5).color(if dark { egui::Color32::from_rgb(120, 190, 255) } else { egui::Color32::from_rgb(0, 80, 170) }), "https://www.linkedin.com/in/alain-valette-clary-524b28352/");
                                                });
                                            });
                                            // Copyright sur plusieurs lignes (retour à la ligne automatique).
                                            ui.add(egui::Label::new(egui::RichText::new(format!("© 2026 Alain Valette-Clary / ΩiA-Group — {}", tr(lang, "droits"))).size(12.0)).wrap(true));
                                            ui.separator();
                                            ui.label(egui::RichText::new(tr(lang, "composants")).size(12.5).strong());
                                            ui.label(egui::RichText::new("OpenCASCADE (LGPL-2.1)\nmanifold-rust (Apache-2.0)\nopenNURBS (McNeel) · OpenSKP (MIT) · Assimp (BSD-3)\negui · glow · winit · glam (MIT/Apache-2.0)\nrfd · image (MIT)").size(12.0));
                                            });
                                    });
                                });
                            });
                            ui.separator();
                            let modes = [
                                (DisplayMode::Textured,    tr(lang, "textured")),
                                (DisplayMode::Transparent, tr(lang, "transparent")),
                                (DisplayMode::Wireframe,   tr(lang, "wireframe")),
                                (DisplayMode::Defects,     tr(lang, "defauts_mode")),
                            ];
                            for (m, label) in modes {
                                let on = disp == m;
                                // actif = JAUNE texte noir ; inactif = noir texte blanc (comme les vues de la barre)
                                let btn = egui::Button::new(egui::RichText::new(label.to_uppercase()).size(13.0).strong()
                                        .color(if on { egui::Color32::BLACK } else { egui::Color32::WHITE }))
                                    .min_size(egui::vec2(0.0, 26.0))
                                    .fill(if on { JAUNE } else { egui::Color32::BLACK })
                                    .rounding(egui::Rounding::same(6.0))
                                    .stroke(egui::Stroke::new(1.2, egui::Color32::from_gray(if on { 230 } else { 110 })));
                                if ui.add_sized(egui::vec2(ui.available_width(), 26.0), btn).clicked() { disp = m; }
                            }

                            ui.add_space(6.0);
                            ui.separator();
                            ui.label(egui::RichText::new(format!("{} {} · {} obj.", scene.iter().map(|o| o.tris.len()).sum::<usize>(), tr(lang, "triangles"), scene.iter().filter(|o| o.nverts > 0).count())).weak().size(11.0));
                            if let Some(sel) = selection {
                                ui.label(egui::RichText::new(format!("P ({:.1}, {:.1}, {:.1})", sel.x, sel.y, sel.z)).size(11.0));
                            }

                            // --- Métriques de défauts + boutons correcteurs, EN BAS du panneau ---
                            ui.add_space(10.0);
                            ui.separator();
                            // simple titre (pas un menu) : texte rouge, sans cadre
                            // rouge s'il reste des défauts, vert foncé si l'objet est sain
                            let sain = defects.face_errors == 0 && defects.holes == 0;
                            let tcol = if sain { egui::Color32::from_rgb(40, 140, 60) } else {
                                // clignotement franc (0,5 s allumé / 0,5 s atténué) : 2 rafraîchissements/s seulement
                                let t = ui.input(|i| i.time);
                                let reste = 0.5 - (t % 0.5);
                                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(reste + 0.01));
                                if (t / 0.5).floor() as i64 % 2 == 0 { egui::Color32::from_rgb(240, 70, 70) } else { egui::Color32::from_rgb(240, 70, 70).gamma_multiply(0.22) }
                            };
                            ui.label(egui::RichText::new(tr(lang, "defauts").to_uppercase()).strong().size(13.0).color(tcol));
                            ui.horizontal(|ui| {
                                let (cj, cr) = if dark { (egui::Color32::from_rgb(232, 200, 60), egui::Color32::from_rgb(235, 90, 90)) }
                                               else { (egui::Color32::from_rgb(150, 105, 0), egui::Color32::from_rgb(185, 25, 25)) };
                                ui.colored_label(cj, egui::RichText::new(format!("{} {}", defects.face_errors, tr(lang, "faces"))).strong());
                                ui.colored_label(cr, egui::RichText::new(format!("{} {}", defects.holes, tr(lang, "trous"))).strong());
                            });
                            ui.add_space(4.0);
                            // Boutons correcteurs d'erreurs : lissage interne, réparation classique + A.I., tout en bas.
                            let b0 = yellow_btn(&tr(lang, "lissage").to_uppercase(), 13.0);
                            if ui.add_enabled_ui(!busy, |ui| ui.add_sized(egui::vec2(ui.available_width(), 34.0), b0)).inner.on_hover_text(tr(lang, "lissage_tip")).clicked() { lissage_req = true; }
                            let b1 = yellow_btn(&tr(lang, "repair").to_uppercase(), 13.0);
                            if ui.add_enabled_ui(!busy, |ui| ui.add_sized(egui::vec2(ui.available_width(), 34.0), b1)).inner.on_hover_text(tr(lang, "repair_tip")).clicked() { repair_req = true; }
                            // --- Simplification : nombre de polygones réglable (− simplifie, + lisse) ---
                            ui.add_space(8.0);
                            ui.separator();
                            if section(ui, "simp_section", &tr(lang, "simplif").to_uppercase()) {
                                let cur_n = sel.filter(|&k| k < scene.len()).or(if scene.len() == 1 { Some(0) } else { None })
                                    .map(|k| scene[k].tris.len()).unwrap_or(0);
                                if cur_n != simp_seen { simp_seen = cur_n; simp_target = cur_n; }
                                ui.label(egui::RichText::new(format!("{} : {}", tr(lang, "polygones"), cur_n)).strong());
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "cible"));
                                    let speed = (simp_target as f64 / 200.0).max(1.0);
                                    ui.add(egui::DragValue::new(&mut simp_target).speed(speed).clamp_range(4..=simplify::MAX_TRIS));
                                });
                                ui.horizontal(|ui| {
                                    for (lbl, f) in [("−50 %", 0.5f64), ("−10 %", 0.9), ("+10 %", 1.1), ("×2", 2.0)] {
                                        if ui.add(yellow_btn(lbl, 11.0)).clicked() { simp_target = ((simp_target as f64 * f).round() as usize).clamp(4, simplify::MAX_TRIS); }
                                    }
                                });
                                ui.checkbox(&mut simp_env, tr(lang, "simp_env")).on_hover_text(tr(lang, "simp_env_tip"));
                                let b2 = yellow_btn(&tr(lang, "simplif").to_uppercase(), 13.0);
                                if ui.add_enabled_ui(!busy, |ui| ui.add_sized(egui::vec2(ui.available_width(), 34.0), b2)).inner.on_hover_text(tr(lang, "simplif_tip")).clicked() { simp_req = true; }
                            }
                            // --- Fil de fer imprimable (dépend de la Simplification) : replié par défaut ---
                            ui.add_space(8.0);
                            if section(ui, "wire_section", &tr(lang, "wireframe").to_uppercase()) {
                                // Un seul fil, de la largeur de la buse (débit). Pas de volume ajouté.
                                ui.horizontal(|ui| { ui.label(tr(lang, "wire_nozzle")); ui.add(egui::DragValue::new(&mut wire_nozzle).clamp_range(0.1..=2.0).speed(0.05).suffix(" mm")); });
                                ui.horizontal(|ui| { ui.label(tr(lang, "wire_thick")); ui.add(egui::DragValue::new(&mut wire_thick).clamp_range(0.1..=10.0).speed(0.05).suffix(" mm")).on_hover_text(tr(lang, "wire_thicktip")); });
                                ui.horizontal(|ui| { ui.label(tr(lang, "wire_meshres")); ui.add(egui::DragValue::new(&mut wire_mesh_res).clamp_range(6..=200).speed(0.5)).on_hover_text(tr(lang, "wire_meshtip")); });
                                ui.horizontal(|ui| { ui.label(tr(lang, "wire_found")); ui.add(egui::DragValue::new(&mut wire_found).clamp_range(0.0..=50.0).speed(0.1).suffix(" mm")).on_hover_text(tr(lang, "wire_foundtip")); });
                                ui.horizontal(|ui| { ui.label(tr(lang, "wire_res")); ui.add(egui::DragValue::new(&mut wire_sides).clamp_range(3..=24).speed(0.2)); });
                                if ui.add_enabled_ui(!busy, |ui| ui.add_sized(egui::vec2(ui.available_width(), 30.0),
                                    yellow_btn(&tr(lang, "wire_make").to_uppercase(), 13.0))).inner.on_hover_text(tr(lang, "wire_tip")).clicked() { wire_req = true; }
                                if let Some(m) = &wire_msg {
                                    let col = if m.starts_with('!') { egui::Color32::from_rgb(235, 110, 90) } else { egui::Color32::from_rgb(130, 200, 130) };
                                    ui.colored_label(col, m);
                                }
                            }
                            // --- CREUSER : l'objet devient une coque (économie de matière), replié par défaut ---
                            ui.add_space(8.0);
                            if section(ui, "creux_section", &tr(lang, "creux_btn").to_uppercase()) {
                                ui.horizontal(|ui| { ui.label(tr(lang, "creux_ep")); ui.add(egui::DragValue::new(&mut creux_ep).clamp_range(0.4..=50.0).speed(0.05).suffix(" mm")).on_hover_text(tr(lang, "creux_ep_tip")); });
                                case_jaune(ui, &mut creux_trou, &tr(lang, "creux_trou")).on_hover_text(tr(lang, "creux_trou_tip"));
                                ui.add_enabled_ui(creux_trou, |ui| { ui.horizontal(|ui| { ui.label(tr(lang, "creux_diam")); ui.add(egui::DragValue::new(&mut creux_diam).clamp_range(0.5..=30.0).speed(0.05).suffix(" mm")); }); });
                                if ui.add_enabled_ui(!busy, |ui| ui.add_sized(egui::vec2(ui.available_width(), 30.0),
                                    yellow_btn(&tr(lang, "creux_btn").to_uppercase(), 13.0))).inner.on_hover_text(tr(lang, "creux_tip")).clicked() { creux_req = true; }
                                if let Some(m) = &creux_msg {
                                    let col = if m.starts_with('!') { egui::Color32::from_rgb(235, 110, 90) } else { egui::Color32::from_rgb(130, 200, 130) };
                                    ui.add(egui::Label::new(egui::RichText::new(m).color(col)).wrap(true));
                                }
                            }
                            // --- TEXTURES : passe toute la colonne de droite en mode Texture ---
                            ui.add_space(8.0);
                            if ui.add_sized(egui::vec2(ui.available_width(), 30.0), yellow_btn(&tr(lang, "tx_titre").to_uppercase(), 13.0))
                                .on_hover_text(tr(lang, "tx_tip")).clicked() { tex.ouvrir(); }
                            } // fin panneau d'outils
                            });
                        });
                        egui::TopBottomPanel::bottom("barre").exact_height(BAR_H).show(ctx, |ui| {
                            // Barre à GÉOMÉTRIE VARIABLE (largeur = zone d'affichage, le panneau de droite
                            // descend jusqu'en bas) : Fichier/Sauver/Effacer à gauche, vues au centre des
                            // fenêtres d'affichage, le reste calé à droite. Si la place manque, tout
                            // rétrécit ensemble (échelle `sc`) : les groupes ne se chevauchent jamais.
                            let full = ui.max_rect();
                            let bar_top = full.top();
                            const GAP: f32 = 14.0;
                            const W_LEFT: f32 = 5.0 * 56.0 + 4.0 * 6.0;
                            const W_VIEWS: f32 = 120.0; // un seul bouton de vue (menu déroulant vertical)
                            let sc = (full.width() / (W_LEFT + W_VIEWS + bar_wr1 + 2.0 * GAP + 8.0)).clamp(0.4, 1.0);
                            ui.spacing_mut().item_spacing.x = 6.0 * sc;
                            let (wl, wv, wr) = (W_LEFT * sc, W_VIEWS * sc, bar_wr1 * sc);
                            let top = full.top() + 3.0;
                            if bar_row != Some((top, 50.0 * sc)) { bar_row = Some((top, 50.0 * sc)); ui.ctx().request_repaint(); }
                            // rangée limitée à la hauteur d'un bouton : tous les groupes centrés sur la même ligne
                            let row = |x0: f32, x1: f32| egui::Rect::from_min_max(egui::pos2(x0, top), egui::pos2(x1, top + 50.0 * sc));
                            // vues : centrées sur la zone d'affichage, bornées par les deux autres groupes
                            let vx = (full.center().x - wv * 0.5).clamp(full.left() + wl + GAP * sc, (full.right() - wr - GAP * sc - wv).max(full.left() + wl + GAP * sc));
                            // --- Gauche : fichier / sauver / effacer ---
                            ui.allocate_ui_at_rect(row(full.left(), full.left() + wl), |ui| { ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                // Fichier : Ouvrir (remplace la scène) ou Importer (ajoute un objet), comme 3D Builder
                                let fid = egui::Id::new("pop_fichier");
                                let fr_ = icon_button_s(ui, tr(lang, "fichier"), ui.memory(|m| m.is_popup_open(fid)), draw_folder, sc);
                                if fr_.clicked() && !busy { ui.memory_mut(|m| m.toggle_popup(fid)); }
                                if std::env::var("PATCH3D_OPEN").as_deref() == Ok("fichier") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(fid)); }
                                bar_menu_at(ui.ctx(), fid, &fr_, bar_top, false, 300.0, |ui| {
                                    yellow_title(ui, tr(lang, "fichier"));
                                    for (k, tip, ajout) in [("f_ouvrir", "f_ouvrir_tip", false), ("f_importer", "f_importer_tip", true)] {
                                        if ui.add_sized(egui::vec2(284.0, 28.0), egui::Button::new(egui::RichText::new(tr(lang, k)).size(12.5))).on_hover_text(tr(lang, tip)).clicked() {
                                            import_req = Some(ajout); ui.memory_mut(|m| m.close_popup());
                                        }
                                    }
                                });
                                // Sauver : le FORMAT se choisit d'abord (le sélecteur de fichiers ne change pas l'extension)
                                let sid = egui::Id::new("pop_save");
                                let sr_ = icon_button_s(ui, tr(lang, "sauver"), ui.memory(|m| m.is_popup_open(sid)), draw_floppy, sc);
                                if sr_.clicked() && !busy { ui.memory_mut(|m| m.toggle_popup(sid)); }
                                if std::env::var("PATCH3D_OPEN").as_deref() == Ok("sauver") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(sid)); }
                                let n_sel = if multi.len() >= 2 { multi.len() } else if sel.map_or(false, |i| i < scene.len()) { 1 } else { 0 };
                                let n_tot = scene.iter().filter(|o| o.nverts > 0).count();
                                bar_menu_at(ui.ctx(), sid, &sr_, bar_top, false, 330.0, |ui| { if let Some(e) = menu_sauver(ui, lang, n_sel, n_tot) { export_req = Some(e); } });
                                // 3D-IMP : préparation à l'impression (panneau qui occupe toute la colonne de droite)
                                let ib = icon_button_s(ui, "3D-IMP", true, draw_imprimante, sc).on_hover_text(tr(lang, "imp_tip"));
                                if panneau_imp { ui.painter().rect_stroke(ib.rect.shrink(1.0), 6.0, egui::Stroke::new(2.0, JAUNE)); }
                                if ib.clicked() { panneau_imp = !panneau_imp; if panneau_imp { panneau_bool = false; panneau_forme = false; } }
                                // Appareil photo : copie d'écran de la visualisation en cours, choix de la définition.
                                let pid = egui::Id::new("pop_photo");
                                if std::env::var("PATCH3D_OPEN").as_deref() == Ok("photo") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(pid)); }
                                let pr = icon_button_s(ui, tr(lang, "photo"), ui.memory(|m| m.is_popup_open(pid)), draw_camera, sc);
                                if pr.clicked() && !busy { ui.memory_mut(|m| m.toggle_popup(pid)); }
                                // grille compacte (2 rangées) : tient dans la bande sous les vues
                                bar_menu_at(ui.ctx(), pid, &pr, bar_top, false, 330.0, |ui| {
                                    yellow_title(ui, tr(lang, "photo_tit"));
                                    let res = [(1024u32, 768u32, "1024×768"), (1280, 720, "1280×720 HD"), (1920, 1080, "1920×1080 FHD"),
                                               (2560, 1440, "2560×1440 QHD"), (3840, 2160, "3840×2160 4K")];
                                    for rangee in res.chunks(3) {
                                        ui.horizontal(|ui| {
                                            for &(w, h, t) in rangee {
                                                if ui.add_sized(egui::vec2(98.0, 26.0), egui::Button::new(egui::RichText::new(t).size(11.0))).clicked() {
                                                    shot_req = Some((w, h)); ui.memory_mut(|m| m.close_popup());
                                                }
                                            }
                                        });
                                    }
                                });
                                if icon_button_s(ui, tr(lang, "effacer"), false, draw_cross, sc).on_hover_text(tr(lang, "effacer_tip")).clicked() { clear_req = true; }
                            }); });
                            // --- Centre : sélecteur de vues ---
                            ui.allocate_ui_at_rect(row(vx, vx + wv), |ui| { ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                // Vues : UN bouton (vue courante, jaune) ; au clic, les choix se déploient
                                // VERTICALEMENT vers le haut, la vue courante en jaune.
                                let vues = [
                                    (ViewMode::Single(ViewKind::ThreeD), "3D"),
                                    (ViewMode::Quad, tr(lang, "vues4")),
                                    (ViewMode::Single(ViewKind::Top), tr(lang, "top")),
                                    (ViewMode::Single(ViewKind::Front), tr(lang, "front")),
                                    (ViewMode::Single(ViewKind::Right), tr(lang, "right")),
                                ];
                                let vid = egui::Id::new("pop_vues");
                                let ouvert = ui.memory(|m| m.is_popup_open(vid));
                                let courant = vues.iter().find(|v| v.0 == mode).map_or("3D", |v| v.1);
                                let yellow = egui::Color32::from_rgb(235, 204, 84);
                                let vb = ui.add(egui::Button::new(egui::RichText::new(format!("{}  {}", courant.to_uppercase(), if ouvert { "v" } else { "^" })).size(13.0 * sc.max(0.6)).strong().color(egui::Color32::BLACK))
                                    .min_size(egui::vec2(W_VIEWS * sc, 50.0 * sc)).fill(yellow).rounding(egui::Rounding::same(8.0))
                                    .stroke(egui::Stroke::new(1.5, egui::Color32::from_gray(230))))
                                    .on_hover_text(tr(lang, "vue_tip"));
                                if vb.clicked() { ui.memory_mut(|m| m.toggle_popup(vid)); }
                                if std::env::var("PATCH3D_OPEN").as_deref() == Ok("vues") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(vid)); }
                                if ui.memory(|m| m.is_popup_open(vid)) {
                                    let bw = vb.rect.width();
                                    let r = egui::Area::new(vid.with("area")).order(egui::Order::Foreground)
                                        .fixed_pos(egui::pos2(vb.rect.left(), vb.rect.top() - 6.0)).pivot(egui::Align2::LEFT_BOTTOM)
                                        .show(ui.ctx(), |ui| {
                                            egui::Frame::popup(ui.style()).fill(egui::Color32::from_rgb(24, 26, 30)).inner_margin(egui::Margin::same(5.0)).show(ui, |ui| {
                                                ui.spacing_mut().item_spacing.y = 5.0;
                                                ui.vertical(|ui| {
                                                    for (m, name) in vues {
                                                        let on = mode == m;
                                                        let (fill, txt) = if on { (yellow, egui::Color32::BLACK) } else { (egui::Color32::BLACK, egui::Color32::WHITE) };
                                                        let btn = egui::Button::new(egui::RichText::new(name.to_uppercase()).size(13.0).strong().color(txt))
                                                            .fill(fill).rounding(egui::Rounding::same(8.0))
                                                            .stroke(egui::Stroke::new(1.5, egui::Color32::from_gray(if on { 230 } else { 110 })));
                                                        if ui.add_sized(egui::vec2(bw - 10.0, 34.0), btn).clicked() {
                                                            mode = m; window.request_redraw();
                                                            ui.memory_mut(|mm| mm.close_popup());
                                                        }
                                                    }
                                                });
                                            });
                                        });
                                    if r.response.clicked_elsewhere() && !vb.clicked() { ui.memory_mut(|m| m.close_popup()); }
                                }
                            }); });
                            // --- Droite : Règles (+ cotes), Taille / Snap Grille, Centrer, Options ---
                            let rg = ui.allocate_ui_at_rect(row(full.right() - wr, full.right()), |ui| { ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                // Bouton Règle + cotes de l'objet EMPILÉES EN DESSOUS (même colonne).
                                // menu Unités : ouvert AU-DESSUS du bouton UNITÉS (clic gauche), ou par clic droit sur la règle ;
                                // pas d'infobulle tant qu'il est ouvert
                                let uid = egui::Id::new("pop_unites");
                                let u_ouvert = ui.memory(|m| m.is_popup_open(uid));
                                // Bouton de centrage (bleu comme Règles), à gauche de la règle : colle l'objet sélectionné à la base X Y Z.
                                if icon_button_s(ui, tr(lang, "centrer"), true, draw_center, sc).clicked() { center_req = true; }
                                ui.vertical(|ui| {
                                    let rresp = ruler_button(ui, unit.suffix(), show_ruler, sc);
                                    let rresp = if u_ouvert { rresp } else { rresp.on_hover_text(tr(lang, "regle_tip")) };
                                    if rresp.clicked() { show_ruler = !show_ruler; window.request_redraw(); }
                                    if rresp.secondary_clicked() { ui.memory_mut(|m| m.toggle_popup(uid)); }
                                });
                                {
                                    let ub = menu_button(ui, tr(lang, "unites"), u_ouvert, 108.0 * sc, sc);
                                    let ub = if u_ouvert { ub } else { ub.on_hover_text(tr(lang, "unites_tip")) };
                                    if ub.clicked() { ui.memory_mut(|m| m.toggle_popup(uid)); }
                                    if std::env::var("PATCH3D_OPEN").as_deref() == Ok("unites") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(uid)); }
                                    bar_menu_above(ui.ctx(), uid, &ub, 230.0, |ui| {
                                        yellow_title(ui, tr(lang, "unites"));
                                        // système d'unités : deux cases exclusives (métrique / notation anglaise)
                                        let avant = unit;
                                        // case du système en cours : JAUNE à coche noire (même style que les cases de l'engrenage)
                                        let met = unit.metrique();
                                        if gpu_check(ui, met, tr(lang, "u_metrique")).clicked() && !met { unit = Unit::Mm; }
                                        if gpu_check(ui, !met, tr(lang, "u_anglais")).clicked() && met { unit = Unit::In; }
                                        ui.separator();
                                        let choix: &[(Unit, &str)] = if unit.metrique() { &[(Unit::Mm, "u_mm"), (Unit::Cm, "u_cm"), (Unit::M, "u_m")] }
                                                                     else { &[(Unit::In, "u_in"), (Unit::Ft, "u_ft"), (Unit::Yd, "u_yd")] };
                                        for &(u, k) in choix {
                                            if ui.selectable_label(unit == u, tr(lang, k)).clicked() { unit = u; ui.memory_mut(|m| m.close_popup()); }
                                        }
                                        if unit != avant {
                                            // grille et règle recalées (dims_grille) sur des pas ronds du nouveau système
                                            window.request_redraw();
                                        }
                                    });
                                }
                                // Taille et Snap Grille côte à côte (même ligne que Centrer).
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let zid = egui::Id::new("pop_resize");
                                let zr = menu_button(ui, tr(lang, "redim"), ui.memory(|m| m.is_popup_open(zid)), 108.0 * sc, sc)
                                    .on_hover_text(tr(lang, "taille_tip"));
                                if zr.clicked() { ui.memory_mut(|m| m.toggle_popup(zid)); }
                                if std::env::var("PATCH3D_OPEN").as_deref() == Ok("taille") && ui.ctx().frame_nr() == 2 { ui.memory_mut(|m| m.open_popup(zid)); }
                                // C. molette sur le bouton TAILLE (ou sur son menu) : redimensionne l'objet
                                let sur_menu = ui.ctx().memory(|m| m.area_rect(zid.with("area"))).zip(ui.ctx().pointer_latest_pos()).map_or(false, |(r, p)| r.contains(p));
                                if zr.hovered() || sur_menu {
                                    let dy = ui.input(|i| i.raw_scroll_delta.y);
                                    let ai = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None });
                                    if let (true, Some(i)) = (dy != 0.0, ai) {
                                        let f = (dy * 0.0008).exp().clamp(0.8, 1.25); // ~4 % par cran ; pousser (dy>0) = agrandir
                                        let c = Vec3::new((scene[i].min.x + scene[i].max.x) * 0.5, (scene[i].min.y + scene[i].max.y) * 0.5, scene[i].min.z);
                                        scene[i].scale_about(c, Vec3::splat(f)); unsafe { scene[i].upload(&gl); }
                                        if let Some(s) = selection.as_mut() { *s = c + (*s - c) * f; }
                                        taille_molette = Some(std::time::Instant::now());
                                        window.request_redraw();
                                    }
                                }
                                bar_menu_above(ui.ctx(), zid, &zr, 250.0, |ui| {
                                    let ai = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None });
                                    match ai {
                                        None => { ui.label(tr(lang, "aucun_obj")); }
                                        Some(i) => {
                                            let ext = scene[i].max - scene[i].min;
                                            let uf = unit.factor(); // taille affichée dans l'unité de la règle
                                            let sfx = format!(" {}", unit.suffix());
                                            let mut w = (ext.x.max(1e-4)) / uf;
                                            let mut h = (ext.z.max(1e-4)) / uf;
                                            let rw = ui.horizontal(|ui| { ui.label(tr(lang, "largeur")); ui.add(egui::DragValue::new(&mut w).clamp_range(0.0001..=100000.0).speed(0.1 / uf).suffix(&sfx)) }).inner;
                                            let rh = ui.horizontal(|ui| { ui.label(tr(lang, "hauteur_o")); ui.add(egui::DragValue::new(&mut h).clamp_range(0.0001..=100000.0).speed(0.1 / uf).suffix(&sfx)) }).inner;
                                            ui.checkbox(&mut resize_lock, tr(lang, "chaine"));
                                            if rw.changed() || rh.changed() {
                                                let c = Vec3::new((scene[i].min.x + scene[i].max.x) * 0.5, (scene[i].min.y + scene[i].max.y) * 0.5, scene[i].min.z);
                                                // les ratios sont sans unité (l'unité se simplifie)
                                                let f = if resize_lock {
                                                    let ratio = if rw.changed() { (w * uf) / ext.x.max(1e-4) } else { (h * uf) / ext.z.max(1e-4) };
                                                    Vec3::splat(ratio)
                                                } else {
                                                    Vec3::new((w * uf) / ext.x.max(1e-4), 1.0, (h * uf) / ext.z.max(1e-4))
                                                };
                                                if f.is_finite() && f.min_element() > 0.0 {
                                                    scene[i].scale_about(c, f); unsafe { scene[i].upload(&gl); }
                                                    if let Some(s) = selection.as_mut() { *s = c + (*s - c) * f; }
                                                    window.request_redraw();
                                                }
                                            }
                                            if rw.drag_released() || rh.drag_released() || rw.lost_focus() || rh.lost_focus() { resize_final = true; }
                                        }
                                    }
                                });
                                }); // fin ligne Taille + Snap Grille
                                // FORME (mise en forme) : à gauche d'OBJ-AXE ; ouvre / ferme son panneau d'outils
                                let fb = icon_button_s(ui, &tr(lang, "forme_btn").to_uppercase(), true, draw_forme, sc).on_hover_text(tr(lang, "forme_tip"));
                                if panneau_forme { ui.painter().rect_stroke(fb.rect.shrink(1.0), 6.0, egui::Stroke::new(2.0, JAUNE)); }
                                if fb.clicked() { panneau_forme = !panneau_forme; if panneau_forme { panneau_bool = false; panneau_imp = false; } }
                                // OBJ-AXE (bleu) : ouvre / ferme l'outil de rotation
                                let oa = icon_button_s(ui, "OBJ-AXE", true, draw_obj_axe, sc)
                                    .on_hover_text(tr(lang, "objaxe_tip"));
                                if oa.clicked() {
                                    if gizmo.is_some() { gizmo = None; }
                                    else {
                                        match sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None }) {
                                            Some(i) => { gizmo = Some(gizmo::Gizmo::new(i, scene[i].min, scene[i].max)); sel = Some(i); light_mode = false; }
                                            None => status_msg = Some((tr(lang, "objaxe_nosel").into(), true, std::time::Instant::now())),
                                        }
                                    }
                                    window.request_redraw();
                                }
                                // BOOLÉENS : remplace le panneau d'outils par le panneau des booléens (et inversement)
                                // largeur ajustée au libellé (BOOLÉENS, BOOLEANOS…) : rien n'est rogné
                                let lib = tr(lang, "bool_btn").to_uppercase();
                                let lw = ui.fonts(|f| f.layout_no_wrap(lib.clone(), egui::FontId::proportional((10.5 * sc).max(7.0)), egui::Color32::WHITE).size().x) / sc;
                                let bb = icon_button_w(ui, &lib, true, draw_booleens, sc, (lw + 14.0).max(56.0)).on_hover_text(tr(lang, "bool_tip"));
                                if panneau_bool { ui.painter().rect_stroke(bb.rect.shrink(1.0), 6.0, egui::Stroke::new(2.0, JAUNE)); }
                                if bb.clicked() { panneau_bool = !panneau_bool; if panneau_bool { panneau_forme = false; panneau_imp = false; } }
                            }); });
                            // largeur réelle du groupe droit -> échelle de l'image suivante (aucun chevauchement)
                            let w1 = rg.response.rect.width() / sc;
                            if (w1 - bar_wr1).abs() > 1.0 { bar_wr1 = w1; ui.ctx().request_repaint(); }
                        });
                        } // fin « pas en visionneuse »
                    });
                    egui_state.handle_platform_output(&window, full.platform_output);
                    // INSÈRE IMAGE : réglage changé dans le panneau → image suivante tout de suite (vues et aperçu à jour)
                    if panneau_texte && txt_image && img_vu != format!("{:?} {img_largeur} {}", img_prm, img.as_ref().map_or(0, |i| i.generation)) { window.request_redraw(); }
                    let delai_repaint = full.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay);
                    let prims = egui_ctx.tessellate(full.shapes, full.pixels_per_point);
                    egui_painter.paint_and_update_textures([size.width, size.height], full.pixels_per_point, &prims, &full.textures_delta);
                    for id in full.textures_delta.free { egui_painter.free_texture(id); }

                    if let Some(r) = &req_test {
                        match r.as_str() { "lissage" => lissage_req = true, "repair" => repair_req = true, "simp" => simp_req = true,
                                           "wire" => wire_req = true, "center" => center_req = true, "clear" => clear_req = true, _ => {} }
                    }
                    // Premier appui sur un bouton : le logo d'accueil disparaît avant toute action.
                    if splash && ui_pressed {
                        for o in &scene { unsafe { o.delete(&gl); } }
                        scene.clear(); sel = None; selection = None; face_normal = None; drag = None;
                        history = vec![snapshot(&scene)]; hist_idx = 0;
                        defects = unsafe { recompute_defects(&gl, &scene, None, &mut bnd_lines, &mut err_lines) };
                        splash = false;
                        window.request_redraw();
                    }
                    // Index de l'objet actif (sélectionné, ou unique).
                    let active = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None });
                    if egui_ctx.frame_nr() == 8 { if let Some(f) = std::env::var_os("PATCH3D_SAUVE") { export_req = Some(if f.to_string_lossy().ends_with(".3mf") { "3mf" } else { "stl" }); } }
                    if let (Some(ext_choisie), true) = (export_req, file_job.is_none() && job.is_none()) {
                        let mut verts = Vec::new();
                        // couleurs : celles des objets colorés, jaune pour les autres (si au moins un est coloré)
                        // seuls les objets sélectionnés sont sauvegardés (réunis dans le même fichier) ;
                        // sans sélection : toute la scène
                        let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else if let Some(i) = sel.filter(|&i| i < scene.len()) { vec![i] }
                                                else { (0..scene.len()).collect() };
                        let a_sauver: Vec<&Obj> = choix.iter().map(|&i| &scene[i]).filter(|o| o.nverts > 0).collect();
                        let couleurs = a_sauver.iter().any(|o| o.has_colors());
                        let mut cols = Vec::new();
                        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                        for o in &a_sauver { lo = lo.min(o.min); hi = hi.max(o.max); }
                        // 3MF, SolidCraft : chaque objet reste SÉPARÉ dans le fichier (positions et couleurs) ;
                        // les autres formats les réunissent en un seul maillage
                        let separes: Vec<(Vec<f32>, Option<Vec<f32>>)> = if ext_choisie == "3mf" || ext_choisie == "solidcraft" {
                            a_sauver.iter().map(|o| (o.verts.clone(), if o.has_colors() { Some(o.cols.clone()) } else { None })).collect()
                        } else { Vec::new() };
                        for o in a_sauver {
                            verts.extend_from_slice(&o.verts);
                            if couleurs { if o.has_colors() { cols.extend_from_slice(&o.cols); } else { for _ in 0..o.verts.len() / 6 { cols.extend_from_slice(&[0.93, 0.80, 0.33]); } } }
                        }
                        let m = stl::Mesh { verts, min: lo, max: hi };
                        file_job = Some(std::thread::spawn(move || {
                            let nom_fmt = FORMATS_SAUVE.iter().find(|f| f.0 == ext_choisie).map_or("", |f| f.1);
                            let ext_f = ext_fichier(ext_choisie);
                            let name = format!("{}.{ext_f}", paths::last_file().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "scene".into()));
                            // test sans sélecteur : PATCH3D_SAUVE=<fichier>
                            let forcé = std::env::var_os("PATCH3D_SAUVE").map(std::path::PathBuf::from);
                            let Some(mut p) = forcé.or_else(|| rfd::FileDialog::new().set_directory(paths::start_dir()).set_file_name(name)
                                .add_filter(nom_fmt, &[ext_f]).save_file())
                            else { return FileOut::Cancelled };
                            // extension effacée ou changée à la main : on remet celle du format choisi
                            if p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() != Some(ext_f) {
                                let mut n = p.file_name().map(|n| n.to_os_string()).unwrap_or_default(); n.push(format!(".{ext_f}")); p.set_file_name(n);
                            }
                            let ext = ext_choisie.to_string();
                            let res = match ext.as_str() {
                                "obj" => stl::write_obj(&p, &m), "ply" => stl::write_ply(&p, &m),
                                "3mf" => threemf::write_objets(&p, &separes.iter().map(|(v, c)| (v.as_slice(), c.as_deref())).collect::<Vec<_>>()),
                                "solidcraft" => solidcraft::ecrire(&p, &separes.iter().map(|(v, c)| (v.as_slice(), c.as_deref())).collect::<Vec<_>>()),
                                "3dm" | "3dm8" => rhino::export(&p, &m.verts, if couleurs { Some(&cols) } else { None }, if ext == "3dm8" { 8 } else { 5 }).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                                "skp" => sketchup::export(&p, &m.verts, if couleurs { Some(&cols) } else { None }).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                                e if assimp::export_id(e).is_some() => assimp::export(&p, e, &m.verts, if couleurs { Some(&cols) } else { None })
                                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                                _ => write_stl(&p, &m),
                            };
                            match res { Ok(()) => { paths::remember(&p); FileOut::Saved(p) } Err(e) => FileOut::Failed(format!("{} — {e}", tr(lang, "err_sauve"))) }
                        }));
                    }
                    // test sans clic : PATCH3D_FILETEST=1 simule un accès Fichier de 6 s (jauge verte fixe)
                    if std::env::var_os("PATCH3D_FILETEST").is_some() && egui_ctx.frame_nr() == 3 && file_job.is_none() {
                        file_job = Some(std::thread::spawn(|| { std::thread::sleep(std::time::Duration::from_secs(6)); FileOut::Cancelled }));
                    }
                    if let (Some(ajout), true) = (import_req, file_job.is_none() && job.is_none()) {
                        file_job = Some(std::thread::spawn(move || {
                            // test sans sélecteur : PATCH3D_IMPORT=<fichier>
                            let forced = std::env::var_os("PATCH3D_IMPORT").map(std::path::PathBuf::from);
                            let Some(p) = forced.or_else(|| rfd::FileDialog::new().set_directory(paths::start_dir())
                                .add_filter(tr(lang, "tous_modeles"), EXT_OUVERTURE)
                                .add_filter("STL", &["stl"]).add_filter("OBJ", &["obj"]).add_filter("PLY", &["ply"]).add_filter("3MF", &["3mf"])
                                .add_filter("3DM (Rhino / MoI)", &["3dm"]).add_filter("SketchUp", &["skp","skb"]).add_filter("3DS", &["3ds"]).add_filter("DAE (Collada / SketchUp)", &["dae"]).add_filter("FBX", &["fbx"]).add_filter("glTF", &["gltf","glb"])
                                .add_filter("STEP", &["step","stp"]).add_filter("IGES", &["iges","igs"])
                                .add_filter("Blender", &["blend"]).add_filter("FreeCAD", &["fcstd","fcstd1"])
                                .add_filter("Fusion 360", &["f3d","f3z"]).add_filter("AutoCAD", &["dxf","dwg"]).add_filter("SolidCraft", &["solidcraft","solvecraft"]).pick_file())
                            else { return FileOut::Cancelled };
                            match charger_objets(&p) {
                                Ok(objs) => { paths::remember(&p); let d = if ajout { None } else { analyse_ouverture(&objs) }; FileOut::Opened(p, objs, ajout, d) }
                                Err(e) => FileOut::Failed(format!("{} — {e}", tr(lang, "err_import"))),
                            }
                        }));
                    }
                    // Copie d'écran : on redessine la visualisation en cours (mode de vue + mode
                    // d'affichage) dans une image hors écran à la définition choisie (MSAA 4×).
                    if let Some((sw, sh)) = shot_req { if file_job.is_none() {
                        let (w, h) = (sw as i32, sh as i32);
                        // textes superposés (règles, axes, cotes, noms de vues) recalculés pour l'image
                        let mut texts: Vec<ShotText> = Vec::new();
                        let mut boxes: Vec<ShotBox> = Vec::new();
                        let ppp = egui_ctx.pixels_per_point();
                        let rgb = unsafe {
                            let ms_fbo = gl.create_framebuffer().unwrap();
                            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(ms_fbo));
                            let ms_col = gl.create_renderbuffer().unwrap();
                            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(ms_col));
                            gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, 4, glow::RGBA8, w, h);
                            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(ms_col));
                            let ms_dep = gl.create_renderbuffer().unwrap();
                            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(ms_dep));
                            gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, 4, glow::DEPTH_COMPONENT24, w, h);
                            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(ms_dep));
                            gl.enable(glow::DEPTH_TEST); gl.depth_func(glow::LESS); gl.depth_mask(true);
                            gl.disable(glow::BLEND); gl.disable(glow::CULL_FACE); gl.disable(glow::SCISSOR_TEST);
                            let (bg_o, bg_v) = if dark { ((0.20, 0.22, 0.26), (0.10, 0.12, 0.15)) } else { ((0.80, 0.82, 0.86), (0.90, 0.91, 0.94)) };
                            gl.viewport(0, 0, w, h);
                            gl.clear_color(bg_o.0, bg_o.1, bg_o.2, 1.0);
                            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                            gl.enable(glow::SCISSOR_TEST);
                            // épaisseur des traits proportionnelle à la définition (lisibles en 4K)
                            let lw = (sh as f32 / hf.max(1.0)).max(1.0);
                            for k in view::ALL_VIEWS {
                                if let Some(sr) = view::screen_rect(mode, k, w as f32, h as f32, 0.0, 0.0) {
                                    let (gx, gy, gw, gh) = view::gl_rect(sr, h as f32);
                                    if gw <= 2 || gh <= 2 { continue; }
                                    gl.viewport(gx, gy, gw, gh);
                                    gl.scissor(gx + 1, gy + 1, gw - 2, gh - 2);
                                    gl.clear_color(bg_v.0, bg_v.1, bg_v.2, 1.0);
                                    gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                                    let (mvp, eye) = view_mvp(k, &orbit, &ortho, base, gw as f32 / gh as f32);
                                    // échelle texte : points écran -> pixels de l'image (même proportion que la vue à l'écran)
                                    let scr_h = view::screen_rect(mode, k, wf, hf, bar_px, side_px).map_or(hf, |r| r.3).max(1.0);
                                    let kt = ppp * sr.3 / scr_h;
                                    let clip = [sr.0, sr.1, sr.0 + sr.2, sr.1 + sr.3];
                                    texts.push(ShotText { x: sr.0 + 8.0 * kt, y: sr.1 + 6.0 * kt, text: kind_tr(lang, k).to_string(), col: [185, 185, 185], size: 13.0 * kt, anchor: 2, clip });
                                    if show_ruler {
                                        let axes = [(Vec3::X, [255u8, 150, 150], "X"), (Vec3::Y, [150, 230, 160], "Y"), (Vec3::Z, [160, 200, 255], "Z")];
                                        let every = (ruler_majors / 10).max(1);
                                        for (dir, col, letter) in axes {
                                            for c in (every..=ruler_majors).step_by(every as usize) {
                                                let wp = dir * (c as f32 * grid_step);
                                                    let t = fmt_measure(c as f32 * grid_step, unit);
                                                let tw = egui_ctx.fonts(|f| f.layout_no_wrap(t.clone(), egui::FontId::proportional(15.0), egui::Color32::WHITE).size().x) * kt;
                                                if let Some(sp) = ruler_label_pos(&mvp, wp, dir, sr, 1.0, tw, 15.0 * kt, 6.0 * kt).filter(|sp| !over_object(&mvp, *sp, tw, sr, &scene)) { texts.push(ShotText { x: sp.x, y: sp.y, text: t, col: [240, 240, 240], size: 15.0 * kt, anchor: 0, clip }); }
                                            }
                                            let wp = dir * ((ruler_majors as f32 + 1.2) * grid_step);
                                            let t = format!("{} {}", letter, unit.suffix());
                                            let tw = egui_ctx.fonts(|f| f.layout_no_wrap(t.clone(), egui::FontId::proportional(17.0), egui::Color32::WHITE).size().x) * kt;
                                            if let Some(sp) = ruler_label_pos(&mvp, wp, dir, sr, 1.0, tw, 17.0 * kt, 6.0 * kt).filter(|sp| !over_object(&mvp, *sp, tw, sr, &scene)) { texts.push(ShotText { x: sp.x, y: sp.y, text: t, col, size: 17.0 * kt, anchor: 0, clip }); }
                                        }
                                    }
                                    let a = mvp.to_cols_array();
                                    let dim = if disp == DisplayMode::Defects { 0.6 } else { 1.0 };
                                    let gray = if disp == DisplayMode::Defects { 1.0 } else { 0.0 };
                                    let tex_hl = disp == DisplayMode::Textured;
                                    let draw_sel = if tex_hl { sel } else { None };
                                    let alpha = if disp == DisplayMode::Transparent { 0.30 } else { 1.0 };
                                    if disp == DisplayMode::Transparent { gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA); gl.depth_mask(false); }
                                    if disp == DisplayMode::Wireframe { gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE); }
                                    draw_view(&gl, program, &scene, draw_sel, &u, &grid, &rule, &mvp, eye, show_ruler, show_grid, 0.0, dim, gray, tex_hl, alpha, None);
                                    if disp == DisplayMode::Wireframe { gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL); }
                                    if disp == DisplayMode::Transparent { gl.disable(glow::BLEND); gl.depth_mask(true); }
                                    if disp == DisplayMode::Defects {
                                        gl.disable(glow::DEPTH_TEST);
                                        err_lines.draw(&gl, &a, [1.0, 0.85, 0.0, 1.0], 3.0 * lw);
                                        bnd_lines.draw(&gl, &a, [1.0, 0.15, 0.15, 1.0], 4.0 * lw);
                                        gl.enable(glow::DEPTH_TEST);
                                    }
                                }
                            }
                            gl.disable(glow::SCISSOR_TEST);
                            // cotes : une seule fois, angle haut-droit de l'image, sans cadre
                            if show_ruler {
                                if let Some(i) = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None }) {
                                    let e = (scene[i].max - scene[i].min) / unit.factor();
                                    let ka = ppp * h as f32 / (hf - bar_px).max(1.0);
                                    let clip = [0.0, 0.0, w as f32, h as f32];
                                    for (n, (nom, v)) in [(tr(lang, "largeur"), e.x), (tr(lang, "longueur"), e.y), (tr(lang, "hauteur_o"), e.z)].into_iter().enumerate() {
                                        texts.push(ShotText { x: w as f32 - 10.0 * ka, y: (8.0 + 18.0 * n as f32) * ka, text: format!("{nom} : {v:.*} {}", unit.decimales(), unit.suffix()), col: [245, 245, 245], size: 14.0 * ka, anchor: 1, clip });
                                    }
                                }
                            }
                            // résolution du MSAA vers une image simple, puis lecture
                            let fbo = gl.create_framebuffer().unwrap();
                            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                            let col = gl.create_renderbuffer().unwrap();
                            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(col));
                            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, w, h);
                            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(col));
                            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(ms_fbo));
                            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo));
                            gl.blit_framebuffer(0, 0, w, h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
                            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                            let mut buf = vec![0u8; (w * h * 4) as usize];
                            gl.read_pixels(0, 0, w, h, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(&mut buf));
                            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                            for f in [fbo, ms_fbo] { gl.delete_framebuffer(f); }
                            for rb in [col, ms_col, ms_dep] { gl.delete_renderbuffer(rb); }
                            gl.viewport(0, 0, size.width as i32, size.height as i32);
                            // RGBA bas→haut  =>  RGB haut→bas
                            let mut rgb = vec![0u8; (w * h * 3) as usize];
                            for y in 0..h as usize { for x in 0..w as usize {
                                let si = ((h as usize - 1 - y) * w as usize + x) * 4; let di = (y * w as usize + x) * 3;
                                rgb[di..di + 3].copy_from_slice(&buf[si..si + 3]);
                            }}
                            rgb
                        };
                        let font_bytes: Vec<u8> = egui::FontDefinitions::default().font_data.get("Ubuntu-Light").map(|f| f.font.to_vec()).unwrap_or_default();
                        file_job = Some(std::thread::spawn(move || {
                            let mut rgb = rgb;
                            stamp_texts(&mut rgb, sw, sh, &font_bytes, &boxes, &texts);
                            stamp_logo(&mut rgb, sw, sh);
                            let mut stem = paths::last_file().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "3D-Patch".into());
                            // retire un suffixe « _LxH » d'une photo précédente (pas d'empilement)
                            if let Some(i) = stem.rfind('_') { let t = &stem[i + 1..]; if t.split_once('x').map_or(false, |(a, b)| !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())) { stem.truncate(i); } }
                            // test sans sélecteur : PATCH3D_SHOT=<fichier.jpg>
                            let forced = std::env::var_os("PATCH3D_SHOT").map(std::path::PathBuf::from);
                            let Some(mut p) = forced.or_else(|| rfd::FileDialog::new().set_directory(paths::start_dir()).set_file_name(format!("{stem}_{sw}x{sh}.jpg"))
                                .add_filter("JPEG", &["jpg", "jpeg"]).save_file())
                            else { return FileOut::Cancelled };
                            if !matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("jpg") | Some("jpeg")) { p.set_extension("jpg"); }
                            let res = std::fs::File::create(&p).map_err(|e| e.to_string()).and_then(|f| {
                                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), 92);
                                enc.encode(&rgb, sw, sh, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())
                            });
                            match res { Ok(()) => { paths::remember(&p); FileOut::Saved(p) } Err(e) => FileOut::Failed(format!("{} — {e}", tr(lang, "err_photo"))) }
                        }));
                        window.request_redraw();
                    }}
                    if assoc_job.as_ref().map_or(false, |h| h.is_finished()) {
                        let r = assoc_job.take().unwrap().join().unwrap_or_else(|_| Err(tr(lang, "err_interne").into()));
                        let ok = r.is_ok();
                        let m = r.unwrap_or_else(|e| format!("{} — {e}", tr(lang, "err_assoc")));
                        println!("{m}");
                        status_msg = Some((m, !ok, std::time::Instant::now()));
                        let a = assoc_etat.clone(); a.store(assoc::is_enabled(), std::sync::atomic::Ordering::Relaxed);
                        window.request_redraw();
                    }
                    if assoc_job.is_some() { window.request_redraw(); }
                    // Fin d'une ouverture / sauvegarde en arrière-plan.
                    if file_job.as_ref().map_or(false, |h| h.is_finished()) {
                        if export_fichier { export_fichier = false; export_fin = Some(std::time::Instant::now()); }
                        let initial = chargement_initial;
                        match file_job.take().unwrap().join() {
                            Ok(FileOut::Opened(p, objs, ajout, pre)) => {
                                let mut nouveaux: Vec<Obj> = objs.into_iter().map(|(v, c)| unsafe { Obj::with_colors(&gl, v, c.unwrap_or_default()) }).collect();
                                if ajout && scene.iter().any(|o| o.nverts > 0) {
                                    // Importer : le(s) objet(s) posé(s) à droite de la scène existante (comme une forme créée)
                                    let prior = scene_bounds(&scene);
                                    let (glo, ghi) = scene_bounds(&nouveaux);
                                    let gap = r * 0.12 + 4.0;
                                    let d = Vec3::new(prior.1.x + gap - glo.x, (prior.0.y + prior.1.y - glo.y - ghi.y) * 0.5, 0.0);
                                    for o in nouveaux.iter_mut() { o.translate(d); unsafe { o.upload(&gl); } }
                                    scene.retain(|o| o.nverts > 0);
                                } else {
                                    for o in &scene { unsafe { o.delete(&gl); } }
                                    scene.clear();
                                }
                                let debut = scene.len();
                                scene.extend(nouveaux);
                                // plusieurs objets (3MF) : tous sélectionnés ensemble
                                multi = if scene.len() - debut >= 2 { (debut..scene.len()).collect() } else { Vec::new() };
                                sel = Some(scene.len() - 1);
                                // à l'import : objet affiché en texturé (le fichier de la visionneuse garde l'affichage de départ)
                                if !chargement_initial { disp = DisplayMode::Textured; }
                                defects = match pre {
                                    // défauts déjà analysés dans le fil de chargement : seulement les traits à envoyer au GPU
                                    Some(d) => unsafe { bnd_lines.update(&gl, &d.boundary); err_lines.update(&gl, &d.errors); d },
                                    None => unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) },
                                };
                                let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                                orbit = Orbit { yaw: CAM_YAW, pitch: CAM_PITCH, dist: r * 3.2, target: base };
                                ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];
                                selection = None;
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                let nt: usize = scene[debut..].iter().map(|o| o.tris.len()).sum();
                                let no = scene.len() - debut;
                                println!("Importé {} — {no} objet(s), {} triangles. Défauts : {} face(s), {} trou(s)", p.display(), nt, defects.face_errors, defects.holes);
                                let objets = if no > 1 { format!("{no} {} · ", tr(lang, "sauve_objets")) } else { String::new() };
                                if chargement_initial {
                                    // fichier ouvert par double-clic : affiché comme au démarrage, sans annonce ni étape d'annulation
                                    if no == 1 { sel = None; }
                                    history = vec![snapshot(&scene)]; hist_idx = 0;
                                } else {
                                    status_msg = Some((format!("{} : {objets}{} {}", tr(lang, "importe"), nt, tr(lang, "triangles")), false, std::time::Instant::now()));
                                }
                            }
                            Ok(FileOut::Saved(p)) => {
                                println!("Exporté : {}", p.display());
                                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                status_msg = Some((format!("{} : {name}", tr(lang, "sauve_ok")), false, std::time::Instant::now()));
                            }
                            Ok(FileOut::Cancelled) => {}
                            Ok(FileOut::Failed(e)) => { eprintln!("{e}"); status_msg = Some((e, true, std::time::Instant::now())); }
                            Err(_) => { status_msg = Some((tr(lang, "err_io").into(), true, std::time::Instant::now())); }
                        }
                        if initial { chargement_initial = false; }
                        window.request_redraw();
                    }
                    if file_job.is_some() { window.request_redraw(); }
                    // test sans clic : PATCH3D_AJOUT=cyl ajoute un cylindre traversant au centre de l'objet ouvert
                    if egui_ctx.frame_nr() == 3 && std::env::var("PATCH3D_AJOUT").as_deref() == Ok("cyl") { new_prim = Some(occt::Prim::Cylinder { r: 6.0, h: 200.0 }); }
                    if let Some(prim) = new_prim.take() {
                        match occt::primitive(prim, occt::DEFLEXION) {
                            Ok(mut nm) => {
                                clean_prim(&mut nm); // primitive irréprochable (0 défaut)
                                place_at_origin(&mut nm);
                                let had = scene.iter().any(|o| o.nverts > 0);
                                let prior = scene_bounds(&scene);
                                let mut o = unsafe { Obj::new(&gl, nm.verts) };
                                if had && std::env::var_os("PATCH3D_AJOUT").is_some() && egui_ctx.frame_nr() < 6 {
                                    // test : outil centré dans l'objet existant et dépassant dessous (perçage traversant)
                                    let c = (prior.0 + prior.1) * 0.5;
                                    o.translate(Vec3::new(c.x - (o.min.x + o.max.x) * 0.5, c.y - (o.min.y + o.max.y) * 0.5, prior.0.z - 5.0 - o.min.z)); unsafe { o.upload(&gl); }
                                } else if had { let gap = r * 0.12 + 4.0; o.translate(Vec3::new(prior.1.x + gap, 0.0, 0.0)); unsafe { o.upload(&gl); } }
                                scene.retain(|o| o.nverts > 0); // retire l'éventuel objet vide initial
                                scene.push(o);
                                sel = Some(scene.len() - 1);
                                defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                                // recadre toujours sur l'ensemble de la scène : l'objet créé est visible
                                orbit = Orbit { yaw: CAM_YAW, pitch: CAM_PITCH, dist: r * 3.2, target: base };
                                ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];
                                let _ = had;
                                selection = None;
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                            }
                            Err(rc) => eprintln!("primitive OCCT échouée (code {rc})"),
                        }
                        window.request_redraw();
                    }
                    // Fin d'un calcul en arrière-plan : on applique le résultat à l'objet.
                    if job.as_ref().map_or(false, |j| j.handle.is_finished()) {
                        let j = job.take().unwrap();
                        let dt = j.t0.elapsed().as_secs_f32();
                        match j.handle.join() {
                            Ok(Ok((_, msg))) if j.kind == JobKind::Coupe => {
                                // chaque objet coupé est remplacé par ses morceaux (index décroissants : pas de décalage)
                                let (ecart, garder) = coupe_ecart_job.take().unwrap_or((Vec3::ZERO, 0));
                                let mut res = std::mem::take(&mut *COUPE_RES.lock().unwrap());
                                res.sort_by(|a, b| b.0.cmp(&a.0));
                                let mut dernier = None;
                                for (i, a, b) in res {
                                    if i >= scene.len() { continue; }
                                    let garde = if garder == 1 { a.clone() } else { b.clone() };
                                    scene[i].verts = garde; scene[i].cols.clear(); scene[i].refresh(); unsafe { scene[i].upload(&gl); }
                                    if garder == 0 {
                                        let mut o = unsafe { Obj::new(&gl, a) };
                                        if ecart != Vec3::ZERO { o.translate(ecart); unsafe { o.upload(&gl); } }
                                        scene.insert(i + 1, o);
                                        dernier = Some(i + 1);
                                    } else { dernier = Some(i); }
                                }
                                // traité en dernier = index le plus bas : aucune insertion ne l'a décalé
                                sel = dernier; multi.clear(); selection = None; gizmo = None;
                                coupe = None; fin_outil = true;
                                let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                outil_ann = coupe_lance.map(|c| (hist_idx, OutilAnn::Coupe(c)));
                                defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                let msg = format!("{msg} — {dt:.1} s");
                                println!("{msg}");
                                status_msg = Some((msg, false, std::time::Instant::now()));
                            }
                            Ok(Ok((v, msg))) if j.kind == JobKind::Bool && j.obj < scene.len() && v.len() >= 18 => {
                                // le résultat remplace le premier objet sélectionné ; les autres disparaissent
                                // (sauf l'objet soustrait si « conserver » est coché)
                                let mut i = j.obj;
                                scene[i].verts = v; scene[i].cols.clear(); scene[i].refresh(); unsafe { scene[i].upload(&gl); }
                                let mut suppr: Vec<usize> = std::mem::take(&mut bool_suppr).into_iter().filter(|&b| b < scene.len() && b != i).collect();
                                suppr.sort_unstable(); suppr.dedup();
                                for &b in suppr.iter().rev() {
                                    unsafe { scene[b].delete(&gl); } scene.remove(b);
                                    if b < i { i -= 1; }
                                }
                                sel = Some(i); multi.clear(); selection = None; gizmo = None; fin_outil = true;
                                // règles, grille et cadrage à la taille de la nouvelle scène (l'outil B a pu disparaître)
                                let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                                orbit.target = base; orbit.dist = r * 3.2;
                                ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                defects = unsafe { recompute_defects(&gl, &scene, Some(i), &mut bnd_lines, &mut err_lines) };
                                let msg = format!("{msg} — {dt:.1} s");
                                println!("{msg}");
                                status_msg = Some((msg, false, std::time::Instant::now()));
                            }
                            Ok(Ok((v, msg))) if j.obj < scene.len() && v.len() >= 18 => {
                                let i = j.obj;
                                if v != scene[i].verts {
                                    scene[i].verts = v; scene[i].refresh(); unsafe { scene[i].upload(&gl); }
                                    history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                    if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                }
                                defects = unsafe { recompute_defects(&gl, &scene, Some(i), &mut bnd_lines, &mut err_lines) };
                                let msg = match j.kind {
                                    JobKind::Repair => format!("{msg} : {} {}, {} {}", defects.face_errors, tr(lang, "faces_err"), defects.holes, tr(lang, "trous")),
                                    JobKind::Wire => { wire_msg = Some(format!("{} {} · {msg}", scene[i].tris.len(), tr(lang, "triangles"))); format!("{} : {} {}", tr(lang, "wireframe"), scene[i].tris.len(), tr(lang, "triangles")) }
                                    JobKind::Texture => { tex.apres_application(msg.clone()); msg }
                                    JobKind::Texte => { txt_cadre = None; fin_outil = true; outil_ann = txt_lance.map(|c| (hist_idx, OutilAnn::Texte(c))); msg }
                                    JobKind::Creux => {
                                        creux_msg = Some(msg.clone());
                                        // trou percé : cylindre en fil de fer (2 cercles + 8 génératrices)
                                        trou_marque = booleen::trou_perce().map(|[x, y, zb, zh, r]| {
                                            let pt = |a: f32, z: f32| Vec3::new(x + r * a.cos(), y + r * a.sin(), z);
                                            let mut sg = Vec::new();
                                            for k in 0..32 {
                                                let (a0, a1) = (k as f32 / 32.0 * std::f32::consts::TAU, (k + 1) as f32 / 32.0 * std::f32::consts::TAU);
                                                sg.push([pt(a0, zb), pt(a1, zb)]); sg.push([pt(a0, zh), pt(a1, zh)]);
                                                if k % 4 == 0 { sg.push([pt(a0, zb), pt(a0, zh)]); }
                                            }
                                            (i, scene[i].min, sg)
                                        });
                                        msg
                                    }
                                    _ => msg,
                                };
                                let msg = format!("{msg} — {dt:.1} s");
                                println!("{msg}");
                                status_msg = Some((msg, false, std::time::Instant::now()));
                            }
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => {
                                if j.kind == JobKind::Wire { wire_msg = Some(format!("! {e}")); }
                                if j.kind == JobKind::Creux { creux_msg = Some(format!("! {e}")); }
                                eprintln!("calcul impossible : {e}");
                                status_msg = Some((e, true, std::time::Instant::now()));
                            }
                            Err(_) => { status_msg = Some((tr(lang, "err_calc").into(), true, std::time::Instant::now())); }
                        }
                        progress::reset();
                        // icône Export de la visionneuse : la réparation d'office finie (même en échec), le menu de sauvegarde s'ouvre
                        if j.kind == JobKind::Repair && export_apres_rep { export_apres_rep = false; ouvrir_export = true; }
                        job_done = Some(std::time::Instant::now());
                        window.request_redraw();
                    }
                    // test sans clic : PATCH3D_REQ=bool_union|bool_soustr|bool_inter (tous les objets sélectionnés)
                    if bool_test.is_some() && egui_ctx.frame_nr() >= 6 && scene.len() >= 2 && job.is_none() {
                        // tout sélectionner (ordre de la scène : le dernier objet est l'outil)
                        multi = (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect(); sel = multi.last().copied();
                        match bool_test.take().as_deref() {
                            Some("bool_union") => bool_req = Some(booleen::Op::Union), Some("bool_soustr") => bool_req = Some(booleen::Op::Soustraction),
                            Some("bool_inter") => bool_req = Some(booleen::Op::Intersection), _ => {} }
                    } else if bool_test.is_some() { egui_ctx.request_repaint(); }
                    // test sans clic : PATCH3D_COUPE="x0,y0,x1,y1" trace la ligne (vue de dessus) et coupe à l'image 6
                    if egui_ctx.frame_nr() == 6 { if let Ok(t) = std::env::var("PATCH3D_COUPE") {
                        let v: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if v.len() == 4 { coupe = Some(booleen::Cadre::new(Vec3::new(v[0], v[1], 0.0), Vec3::new(v[2], v[3], 0.0), 2)); coupe_req = true; }
                    }}
                    // test sans clic : PATCH3D_COUPE_TRACE="x0,y0,x1,y1" = trait L en cours (vue 3D) ; avec ",incl,l" = l en cours de dépli
                    if egui_ctx.frame_nr() == 3 { if let Ok(t) = std::env::var("PATCH3D_COUPE_TRACE") {
                        let v: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if v.len() == 4 || v.len() == 6 {
                            let p0 = Vec3::new(v[0], v[1], 0.0);
                            let mut c = booleen::Cadre::new(p0, aimante_45(p0, Vec3::new(v[2], v[3], 0.0), 2), 2);
                            if v.len() == 6 { c.incl = v[4]; c.l = v[5]; }
                            coupe = Some(c); coupe_tir = Some(ViewKind::ThreeD); coupe_arme = true; coupe_etape = if v.len() == 6 { 2 } else { 1 };
                        }
                    }}
                    if coupe_req && job.is_none() && file_job.is_none() {
                        coupe_lance = coupe;
                        if let Some(c) = coupe.filter(|c| c.reperes().is_some()).map(|c| cadre_etendu(&c, &scene)) {
                            // objets sélectionnés, sinon tous
                            let cibles: Vec<usize> = if multi.len() >= 2 { multi.clone() } else if let Some(i) = sel.filter(|&i| i < scene.len()) { vec![i] }
                                                     else { (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect() };
                            let vs: Vec<(usize, Vec<f32>)> = cibles.iter().map(|&i| (i, scene[i].verts.clone())).collect();
                            let (lo, hi) = scene_bounds(&scene);
                            let ecart = if coupe_ecart && coupe_garder == 0 { c.reperes().unwrap().1 * ((hi - lo).length() * 0.04).max(2.0) } else { Vec3::ZERO };
                            coupe_ecart_job = Some((ecart, coupe_garder));
                            job = Some(spawn_job(cibles.first().copied().unwrap_or(0), JobKind::Coupe, move || {
                                let mut res = Vec::new();
                                let mut err = None;
                                for (i, v) in vs {
                                    match booleen::couper(&v, &c) { Ok((a, b)) => res.push((i, a, b)), Err(e) => err = Some(e) }
                                }
                                if res.is_empty() { return Err(err.unwrap_or_else(|| tr(lang, "coupe_rate").into())); }
                                let n = res.len();
                                *COUPE_RES.lock().unwrap() = res;
                                Ok((Vec::new(), format!("{} : {} {}", tr(lang, "coupe_btn"), n * 2, tr(lang, "coupe_morceaux"))))
                            }));
                        }
                    }
                    // 3D-IMP : orientation conseillée (pose animée sur la face qui demande le moins de supports)
                    if imp_orient_req && grav_anim.is_none() {
                        let choix: Vec<usize> = if let Some(i) = sel.filter(|&i| i < scene.len() && scene[i].nverts > 0) { vec![i] } else { (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect() };
                        if let [i] = choix[..] {
                            match impression::orientation(&scene[i].verts, imp_angle) {
                                Some((n, c, actuel)) if c < actuel - 1.0 => {
                                    grav_anim = Some(GravAnim::vers(i, &scene[i], n));
                                    imp_msg = Some((format!("{} : {:.0} → {:.0} mm²", tr(lang, "imp_orient_ok"), actuel, c), false));
                                }
                                _ => imp_msg = Some((tr(lang, "imp_orient_deja").into(), false)),
                            }
                        }
                    }
                    // 3D-IMP : envoi au trancheur (3MF : objets séparés, à leur position)
                    if let Some(l) = imp_envoi_req.take() {
                        let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else if let Some(i) = sel.filter(|&i| i < scene.len() && scene[i].nverts > 0) { vec![i] }
                                                else { (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect() };
                        let fichier = impression::fichier_echange();
                        let objs: Vec<(&[f32], Option<&[f32]>)> = choix.iter().map(|&i| (scene[i].verts.as_slice(), if scene[i].has_colors() { Some(scene[i].cols.as_slice()) } else { None })).collect();
                        imp_msg = Some(match threemf::write_objets(&fichier, &objs).map_err(|e| e.to_string()).and_then(|_| impression::envoyer(&l, &fichier)) {
                            Ok(()) => (format!("{} ({} {})", tr(lang, "imp_envoye"), choix.len(), tr(lang, "sauve_objets")), false),
                            Err(e) => (format!("{} : {e}", tr(lang, "imp_echec")), true),
                        });
                    }
                    // TEXTE : incrustation (relief ou gravure) dans l'objet sélectionné
                    if txt_appl && job.is_none() && file_job.is_none() {
                        // l'objet sur lequel le texte est posé, sinon la sélection
                        let cible = txt_cadre.and_then(|c| surface_sous(&scene, c.centre, c.n).map(|x| x.0))
                            .or_else(|| sel.filter(|&i| i < scene.len() && scene[i].nverts > 0))
                            .or_else(|| { let v: Vec<usize> = (0..scene.len()).filter(|&i| scene[i].nverts > 0).collect(); if v.len() == 1 { Some(v[0]) } else { None } });
                        // contours : image vectorisée (INSÈRE IMAGE) ou texte dans la police choisie
                        let polys = if txt_image { img.as_ref().map(|im| im.contours_mm(img_largeur).0) } else {
                            texte::polices().and_then(|liste| txt_police.and_then(|j| liste.get(j))
                                .map(|pol| texte::contours(pol, &txt_saisie, txt_taille, txt_just, 0.0).map(|r| r.0).unwrap_or_default()))
                        };
                        let titre = tr(lang, if txt_image { "img_btn" } else { "texte_btn" });
                        match (cible, txt_cadre, polys) {
                            (Some(i), Some(c), Some(polys)) => {
                                txt_lance = Some(c);
                                let prm = booleen::ParamsTexte { polys, u: c.u, v: c.v, n: c.n, centre: c.centre, angle: c.angle, prof: c.prof, relief: txt_relief, suivre: txt_suivre };
                                let v = scene[i].verts.clone();
                                let nom = tr(lang, if txt_relief { "texte_relief" } else { "texte_grave" });
                                job = Some(spawn_job(i, JobKind::Texte, move || booleen::texte(&v, &prm).map(|w| (w, format!("{titre} : {nom}")))));
                            },
                            (None, _, _) => status_msg = Some((tr(lang, "aucun_obj").into(), true, std::time::Instant::now())),
                            _ => {}
                        }
                    }
                    // MISE EN FORME : opération sur la sélection (objet seul ou groupe, Ctrl+clic)
                    if let Some(op) = forme_req { if job.is_none() && file_job.is_none() {
                        let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else { sel.filter(|&i| i < scene.len() && scene[i].nverts > 0).into_iter().collect() };
                        if choix.is_empty() { status_msg = Some((tr(lang, "aucun_obj").into(), true, std::time::Instant::now())); }
                        else {
                            let (glo, ghi) = choix.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, b), &i| (a.min(scene[i].min), b.max(scene[i].max)));
                            let c = (glo + ghi) * 0.5;
                            let axe = |k: usize| match k { 0 => Vec3::X, 1 => Vec3::Y, _ => Vec3::Z };
                            let mut nouveaux: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
                            match op {
                                FormeOp::Pos(d) => for &i in &choix { forme::deplacer(&mut scene[i].verts, d); },
                                FormeOp::Poser => for &i in &choix { forme::deplacer(&mut scene[i].verts, Vec3::new(0.0, 0.0, -glo.z)); },
                                FormeOp::Rot(k, deg) => {
                                    // rotation du groupe autour de son centre ; le groupe garde son appui (z min)
                                    let m = glam::Mat3::from_axis_angle(axe(k), deg.to_radians());
                                    for &i in &choix { forme::tourner(&mut scene[i].verts, c, m); }
                                    let zmin = choix.iter().map(|&i| forme::bornes(&scene[i].verts).0.z).fold(f32::INFINITY, f32::min);
                                    for &i in &choix { forme::deplacer(&mut scene[i].verts, Vec3::new(0.0, 0.0, glo.z - zmin)); }
                                }
                                FormeOp::Miroir(k) => for &i in &choix { let o = &mut scene[i]; forme::miroir(&mut o.verts, &mut o.cols, k, c); },
                                FormeOp::Echelle(f) => {
                                    // autour du centre au sol : l'appui ne bouge pas
                                    let base = Vec3::new(c.x, c.y, glo.z);
                                    for &i in &choix { forme::echelle(&mut scene[i].verts, base, f); }
                                }
                                FormeOp::Aligner(k, m) => if choix.len() >= 2 {
                                    for &i in &choix {
                                        let (lo, hi) = (scene[i].min[k], scene[i].max[k]);
                                        let dk = match m { 0 => glo[k] - lo, 1 => c[k] - (lo + hi) * 0.5, _ => ghi[k] - hi };
                                        forme::deplacer(&mut scene[i].verts, axe(k) * dk);
                                    }
                                },
                                FormeOp::Repartir(k) => if choix.len() >= 3 {
                                    let objets: Vec<(f32, f32)> = choix.iter().map(|&i| (scene[i].min[k], scene[i].max[k] - scene[i].min[k])).collect();
                                    let x = forme::repartir(&objets);
                                    for (j, &i) in choix.iter().enumerate() { forme::deplacer(&mut scene[i].verts, axe(k) * (x[j] - objets[j].0)); }
                                },
                                FormeOp::Lineaire(n, pas, k) => for r in 1..n {
                                    for &i in &choix { let mut v = scene[i].verts.clone(); forme::deplacer(&mut v, axe(k) * pas * r as f32); nouveaux.push((v, scene[i].cols.clone())); }
                                },
                                FormeOp::Circulaire(n, rayon) => {
                                    // copies réparties autour d'un axe vertical placé à `rayon` du centre (côté −X)
                                    let pivot = Vec3::new(c.x - rayon, c.y, 0.0);
                                    for r in 1..n {
                                        let m = glam::Mat3::from_rotation_z(std::f32::consts::TAU * r as f32 / n as f32);
                                        for &i in &choix { let mut v = scene[i].verts.clone(); forme::tourner(&mut v, Vec3::new(pivot.x, pivot.y, c.z), m); nouveaux.push((v, scene[i].cols.clone())); }
                                    }
                                }
                            }
                            for &i in &choix { scene[i].refresh(); unsafe { scene[i].upload(&gl); } }
                            if !nouveaux.is_empty() {
                                scene.retain(|o| o.nverts > 0);
                                let debut = scene.len();
                                for (v, cols) in nouveaux { scene.push(unsafe { Obj::with_colors(&gl, v, cols) }); }
                                // originaux + copies sélectionnés ensemble
                                let mut tous = choix.clone(); tous.extend(debut..scene.len());
                                multi = tous; sel = multi.last().copied();
                            }
                            let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                            // copies créées : la vue recadre toute la scène
                            if matches!(op, FormeOp::Lineaire(..) | FormeOp::Circulaire(..)) {
                                orbit.target = base; orbit.dist = orbit.dist.max(r * 3.2);
                                ortho = [Ortho { pan: Vec2::ZERO, scale: r * 1.4 }; 3];
                            }
                            selection = None; gizmo = None;
                            defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                            // glisser une valeur de position : une seule étape d'annulation pour tout le geste
                            let suite = matches!(op, FormeOp::Pos(_)) && forme_pos_t.map_or(false, |t| t.elapsed().as_secs_f32() < 0.8) && hist_idx + 1 == history.len();
                            forme_pos_t = if matches!(op, FormeOp::Pos(_)) { Some(std::time::Instant::now()) } else { None };
                            if suite { history[hist_idx] = snapshot(&scene); }
                            else {
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                            }
                        }
                        window.request_redraw();
                    }}
                    if let Some(op) = bool_req { if job.is_none() && file_job.is_none() {
                        let choix: Vec<usize> = if multi.len() >= 2 { multi.clone() } else { sel.into_iter().collect() };
                        if choix.len() >= 2 && choix.iter().all(|&i| i < scene.len()) {
                            let vs: Vec<Vec<f32>> = choix.iter().map(|&i| scene[i].verts.clone()).collect();
                            // tous les objets sauf le premier disparaissent ; l'outil de SOUSTRAIRE peut rester
                            bool_suppr = choix[1..].to_vec();
                            if op == booleen::Op::Soustraction && bool_garder_b { bool_suppr.pop(); }
                            let nom = tr(lang, match op { booleen::Op::Union => "bool_union", booleen::Op::Soustraction => "bool_soustr", booleen::Op::Intersection => "bool_inter" });
                            job = Some(spawn_job(choix[0], JobKind::Bool, move || {
                                let refs: Vec<&[f32]> = vs.iter().map(|v| v.as_slice()).collect();
                                booleen::calculer_n(&refs, op).map(|v| { let n = v.len() / 18; (v, format!("{nom} : {n} {}", tr(lang, "triangles"))) })
                            }));
                        } else { status_msg = Some((tr(lang, "bool_choisir").into(), true, std::time::Instant::now())); }
                    }}
                    // DÉGROUPER : la pièce (ou chaque pièce) devient un objet à part, sélectionné, déplaçable
                    if let Some((i, c, tout)) = degroup_req.take() {
                        if i < scene.len() && job.is_none() {
                            let (comp, n) = pieces(&scene[i].verts);
                            if n >= 2 {
                                let mut nouveaux: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
                                if tout { for k in 0..n as u32 { if k != c { nouveaux.push(tris_piece(&scene[i], &comp, k, true)); } } }
                                let (reste, reste_c) = if tout { tris_piece(&scene[i], &comp, c, true) } else { tris_piece(&scene[i], &comp, c, false) };
                                if !tout { nouveaux.push(tris_piece(&scene[i], &comp, c, true)); }
                                let has = scene[i].has_colors();
                                scene[i].verts = reste; scene[i].cols = if has { reste_c } else { Vec::new() }; scene[i].refresh(); unsafe { scene[i].upload(&gl); }
                                for (v, col) in nouveaux { scene.push(unsafe { Obj::with_colors(&gl, v, col) }); }
                                // sélection : la pièce dégroupée (ou la pièce cliquée quand tout est dégroupé)
                                sel = Some(if tout { i } else { scene.len() - 1 }); multi.clear(); selection = None; gizmo = None;
                                piece = None;
                                defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                                history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                                if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                                status_msg = Some((tr(lang, "degroup_ok").replace("{n}", &(if tout { n } else { 2 }).to_string()), false, std::time::Instant::now()));
                                window.request_redraw();
                            }
                        }
                    }
                    // GROUPER : les objets choisis (Maj / Ctrl+clic) deviennent un seul objet (positions gardées)
                    if let Some(mut g) = grouper_req.take() {
                        g.sort_unstable(); g.dedup(); g.retain(|&k| k < scene.len());
                        if g.len() >= 2 && job.is_none() {
                            let couleurs = g.iter().any(|&k| scene[k].has_colors());
                            let (mut v, mut col) = (Vec::new(), Vec::new());
                            for &k in &g {
                                v.extend_from_slice(&scene[k].verts);
                                if couleurs {
                                    // objet sans couleurs propres dans un groupe coloré : jaune de 3D-Patch
                                    if scene[k].has_colors() { col.extend_from_slice(&scene[k].cols); }
                                    else { for _ in 0..scene[k].verts.len() / 6 { col.extend([0.93f32, 0.80, 0.33]); } }
                                }
                            }
                            let i0 = g[0];
                            for &k in g.iter().skip(1).rev() { unsafe { scene[k].delete(&gl); } scene.remove(k); }
                            scene[i0].verts = v; scene[i0].cols = col; scene[i0].refresh(); unsafe { scene[i0].upload(&gl); }
                            sel = Some(i0); multi.clear(); selection = None; gizmo = None; piece = None;
                            defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                            history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                            if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                            status_msg = Some((tr(lang, "grouper_ok").replace("{n}", &g.len().to_string()), false, std::time::Instant::now()));
                            window.request_redraw();
                        }
                    }
                    // pièce choisie : surlignage (refait si l'objet a bougé) ; abandonnée si l'objet change de sélection ou de forme
                    if let Some((i, _, _, _, sig)) = piece.as_ref() {
                        let i = *i;
                        if sel != Some(i) || i >= scene.len() || scene[i].verts.len() != sig.2 { piece = None; }
                    }
                    let voulu = piece.as_ref().map(|(i, c, _, _, _)| (*i, *c, (scene[*i].min, scene[*i].max, scene[*i].verts.len())));
                    let a_jour = match (&piece_vue, &voulu) { (Some((_, s1)), Some((_, _, s2))) => s1 == s2, (None, None) => true, _ => false };
                    if !a_jour || (voulu.is_some() && PIECE_SEL.lock().unwrap().map(|p| p.0) != voulu.map(|v| v.0)) {
                        if let Some((o, _)) = piece_vue.take() { unsafe { o.delete(&gl); } }
                        if let (Some((i, c, sig)), Some((_, _, comp, _, _))) = (voulu, piece.as_ref()) {
                            let (v, _) = tris_piece(&scene[i], comp, c, true);
                            piece_vue = Some((unsafe { Obj::new(&gl, v) }, sig));
                        }
                        window.request_redraw();
                    }
                    *PIECE_SEL.lock().unwrap() = piece_vue.as_ref().zip(voulu).map(|((o, _), (i, _, _))| (i, o.vao, o.nverts));
                    // objet actif recalculé : la fin d'un calcul (booléen) a pu retirer un objet dans cette image
                    let active = sel.filter(|&i| i < scene.len()).or(if scene.len() == 1 && scene[0].nverts > 0 { Some(0) } else { None });
                    // MODE TEXTURE : actions du panneau, aperçu en direct (dessiné à la place de l'objet), application
                    {
                        let cible = active.filter(|_| tex.actif).map(|i| (i, &scene[i].verts[..]));
                        let (pret, retirer) = tex.tick(&egui_ctx, cible);
                        // réveil demandé par l'aperçu (attente des réglages, calcul en cours) : programmé ici, car
                        // le délai d'egui de cette image est déjà relevé (sinon plus rien ne bouge sans la souris)
                        if let Some(d) = tex.reveil {
                            let t = std::time::Instant::now() + d;
                            prochain_repaint = Some(prochain_repaint.map_or(t, |p| p.min(t)));
                            elwt.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(prochain_repaint.unwrap()));
                        }
                        if retirer { if let Some((_, o)) = tx_vue.take() { unsafe { o.delete(&gl); } } window.request_redraw(); }
                        if let Some((i, v, cols)) = pret {
                            if v.is_empty() {
                                if let Some((_, o)) = tx_vue.as_mut() { if cols.len() / 3 == o.verts.len() / 6 { o.cols = cols; unsafe { o.upload_colors(&gl); } } }
                            } else {
                                if let Some((_, o)) = tx_vue.take() { unsafe { o.delete(&gl); } }
                                tx_vue = Some((i, unsafe { Obj::with_colors(&gl, v, cols) }));
                            }
                            window.request_redraw();
                        }
                        *APERCU_TX.lock().unwrap() = tx_vue.as_ref().filter(|_| tex.actif).map(|(i, o)| (*i, o.vao, o.nverts, o.has_colors()));
                        if tx_act.quitter { tex.fermer(); }
                        // APPLIQUER : calcul complet dans un fil (annulable comme les autres opérations)
                        if let Some(i) = active {
                            if tx_act.appliquer && job.is_none() && file_job.is_none() {
                                if let Some(travail) = tex.preparer(&scene[i].verts) {
                                    let v = scene[i].verts.clone();
                                    // défauts de l'objet avant texture : déjà connus (analyse de l'objet actif)
                                    let d0 = (defects.holes, defects.face_errors);
                                    tex.msg = None;
                                    job = Some(spawn_job(i, JobKind::Texture, move || {
                                        let (mut out, avant) = travail.executer(&v)?;
                                        // points quasi confondus au 1/1000 mm (zones très serrées) : réparation
                                        // seulement si la texture a ajouté des défauts à l'objet
                                        for _ in 0..5 {
                                            let d = defects::analyze(&out);
                                            if d.holes <= d0.0 && d.face_errors <= d0.1 { break; }
                                            out = defects::repair(&out);
                                        }
                                        let n = out.len() / 18;
                                        Ok((out, format!("{} : {avant} > {n} {}", tr(lang, "tx_titre"), tr(lang, "triangles"))))
                                    }));
                                }
                            }
                        }
                    }
                    if job.is_none() && file_job.is_none() { if let Some(i) = active {
                        // copie des sommets seulement si un calcul est demandé (sinon, à chaque image, un gros
                        // maillage serait recopié entièrement : des dizaines de ms perdues par image)
                        let demande = lissage_req || repair_req || simp_req || relief_req || creux_req || wire_req;
                        let v = if demande { scene[i].verts.clone() } else { Vec::new() };
                        if lissage_req {
                            job = Some(spawn_job(i, JobKind::Lissage, move || {
                                let (out, n) = defects::remove_internal(&v, perf::liss_res());
                                Ok((if n > 0 && out.len() >= 18 { out } else { v }, format!("{} : {n} {}", tr(lang, "lissage"), tr(lang, "int_suppr"))))
                            }));
                        } else if repair_req {
                            job = Some(spawn_job(i, JobKind::Repair, move || {
                                // Réparation RÉPÉTÉE jusqu'à 0 défaut ou plateau ; jauge : 1re passe 0-90 %.
                                let (mut w, mut prev, mut passes) = (v, (usize::MAX, usize::MAX), 0);
                                for k in 0..25 {
                                    if k == 0 { progress::span(0.0, 0.9) } else { progress::span(1.0 - 0.1 / k as f32, 1.0 - 0.1 / (k + 1) as f32) }
                                    w = defects::repair(&w); passes += 1;
                                    let d = defects::analyze(&w); let cur = (d.holes, d.face_errors);
                                    if cur == (0, 0) || cur == prev { break; }
                                    prev = cur;
                                }
                                Ok((w, format!("{} ({passes} {})", tr(lang, "repair"), tr(lang, "passes"))))
                            }));
                        } else if simp_req {
                            let (target, env) = (simp_target, simp_env);
                            job = Some(spawn_job(i, JobKind::Simp, move || {
                                let before = v.len() / 18;
                                // enveloppe unique : les pièces séparées / imbriquées (mèches, parois internes)
                                // deviennent une seule surface fermée → plus de jours ni d'éclats après réduction
                                let v = if env { progress::span(0.0, 0.25); let e = remesh::envelope(&v, perf::env_res()); if e.len() >= 18 { e } else { v } } else { v };
                                let out = simplify::resample(&v, target);
                                let msg = format!("{} : {before} → {} {}", tr(lang, "simplif"), out.len() / 18, tr(lang, "triangles"));
                                Ok((out, msg))
                            }));
                        } else if relief_req {
                            let mask = if relief_face_only { face_normal } else { None };
                            let pr = relief::Params { pattern: relief_pattern, mapping: relief_mapping, height_mm: relief_height, scale_mm: relief_scale, invert: relief_invert, mask_normal: mask };
                            job = Some(spawn_job(i, JobKind::Relief, move || {
                                let (nv, before, after) = relief::apply(&v, &pr);
                                Ok((nv, format!("{} : {before} → {after} {}", tr(lang, "relief_ok"), tr(lang, "triangles"))))
                            }));
                        } else if creux_req {
                            let (e, trou, res) = (creux_ep, if creux_trou { Some(creux_diam) } else { None }, perf::liss_res());
                            creux_msg = None;
                            job = Some(spawn_job(i, JobKind::Creux, move || {
                                booleen::creuser(&v, e, trou, res).map(|(w, eco)| (w, format!("{} : {} {:.0} %", tr(lang, "creux_btn"), tr(lang, "creux_ok"), eco * 100.0)))
                            }));
                        } else if wire_req {
                            // Épaisseur réelle du fil, jamais sous le Ø buse (mini imprimable).
                            if wire_thick < wire_nozzle { wire_thick = wire_nozzle; }
                            let (d, res, found, sides) = (wire_thick, wire_mesh_res as usize, wire_found, wire_sides as usize);
                            job = Some(spawn_job(i, JobKind::Wire, move || {
                                // On réduit d'abord le maillage à la résolution choisie (évite le « trop d'arêtes »).
                                progress::span(0.0, 0.3);
                                let dec = remesh::decimate(&v, res);
                                let mut src = if dec.len() >= 18 { dec } else { v };
                                remesh::floor_foundation(&mut src, found); // pose sur le lit + base au sol
                                progress::span(0.0, 1.0);
                                remesh::wireframe_solid(&src, d * 0.5, sides).map(|w| (w, format!("Ø {d:.1} mm")))
                            }));
                        }
                    } else if wire_req { wire_msg = Some(format!("! {}", tr(lang, "aucun_obj"))); }
                      else if creux_req { creux_msg = Some(format!("! {}", tr(lang, "aucun_obj"))); } }
                    if job.is_some() || job_done.map_or(false, |t| t.elapsed().as_secs_f32() < 1.3) { window.request_redraw(); }
                    if resize_final {
                        defects = unsafe { recompute_defects(&gl, &scene, active, &mut bnd_lines, &mut err_lines) };
                        history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                        if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                        window.request_redraw();
                    }
                    // "Centrer" de l'overlay : recadre la vue survolée sur toute la scène.
                    if let Some(k) = fit_view {
                        let (lo, hi) = scene_bounds(&scene);
                        base = (lo + hi) * 0.5;
                        let rad = ((hi - lo) * 0.5).length().max(1e-3);
                        if k == ViewKind::ThreeD {
                            orbit.target = base;
                            orbit.dist = (rad * 3.2).clamp(0.05, 50000.0);
                        } else {
                            let o = &mut ortho[view::oidx(k)];
                            o.pan = Vec2::ZERO;
                            o.scale = (rad * 1.2).clamp(0.05, 50000.0);
                        }
                        window.request_redraw();
                    }
                    if center_req { if let Some(i) = active {
                        let d = -scene[i].min; // colle le coin mini à l'origine (base X Y Z)
                        scene[i].translate(d); unsafe { scene[i].upload(&gl); }
                        if let Some(s) = selection.as_mut() { *s += d; }
                        history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                        if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                        window.request_redraw();
                    }}
                    if clear_req && job.is_none() && file_job.is_none() {
                        // Effacer = tout vider : objets ET outils (cadre de coupe, sélection, panneau Booléens).
                        // Pour un seul objet (ou le cadre sélectionné) : touche Suppr.
                        for o in &scene { unsafe { o.delete(&gl); } }
                        scene.clear(); sel = None; multi.clear(); selection = None; gizmo = None;
                        coupe = None; coupe_arme = false; coupe_etape = 0; coupe_tir = None; coupe_sel = false; panneau_bool = false;
                        defects = unsafe { recompute_defects(&gl, &scene, None, &mut bnd_lines, &mut err_lines) };
                        history.truncate(hist_idx + 1); history.push(snapshot(&scene));
                        if history.len() > 40 { history.remove(0); } hist_idx = history.len() - 1;
                        window.request_redraw();
                    }
                    // ANNULER pendant un outil : on revient d'une étape de l'outil, pas de la géométrie
                    if undo_req && (coupe.is_some() || coupe_arme) { coupe = None; coupe_arme = false; coupe_tir = None; coupe_etape = 0; undo_req = false; window.request_redraw(); }
                    if undo_req && surf.is_some() { surf = None; surf_survol = None; undo_req = false; window.request_redraw(); }
                    let retablir = if undo_req { outil_ann.filter(|(h, _)| *h == hist_idx).map(|(_, o)| o) } else { None };
                    if job.is_none() && file_job.is_none() && ((undo_req && hist_idx > 0) || (redo_req && hist_idx + 1 < history.len())) {
                        hist_idx = if undo_req { hist_idx - 1 } else { hist_idx + 1 };
                        for o in &scene { unsafe { o.delete(&gl); } }
                        scene = history[hist_idx].iter().map(|(v, c)| unsafe { Obj::with_colors(&gl, v.clone(), c.clone()) }).collect();
                        sel = sel.filter(|&i| i < scene.len());
                        defects = unsafe { recompute_defects(&gl, &scene, sel, &mut bnd_lines, &mut err_lines) };
                        let (lo, hi) = scene_bounds(&scene); base = (lo + hi) * 0.5; r = ((hi - lo) * 0.5).length().max(1e-3);
                        selection = None;
                        // on annule l'opération d'un outil : l'outil revient tel qu'il était (cadre, texte, réglages)
                        if let Some(o) = retablir {
                            outil_ann = None;
                            match o {
                                OutilAnn::Texte(c) => { panneau_bool = true; panneau_texte = true; txt_cadre = Some(c); txt_sel = true; }
                                OutilAnn::Coupe(c) => { panneau_bool = true; coupe = Some(c); }
                            }
                            panneau_prec = true; // pas de nouveau basculement de vue
                            if mode != ViewMode::Quad { mode_avant_coupe = mode_avant_coupe.or(Some(mode)); mode = ViewMode::Quad; }
                        }
                        window.request_redraw();
                    }
                    surface.swap_buffers(&gl_context).unwrap();
                    {
                        let b = |v: bool| if v { "1" } else { "0" };
                        let l = lumiere_courante();
                        let mut t = format!("langue={}\nunite={}\nsombre={}\ngrille={}\n", lang.code(), unit.code(), b(dark), grille_mode());
                        if !viewer { t += &format!("regles={}\nvue={}\n", b(show_ruler), code_de_vue(mode)); }
                        t += &format!("simp_env={}\ngarder_proportions={}\nfil_buse={wire_nozzle}\nfil_epaisseur={wire_thick}\nfil_fondation={wire_found}\nfil_facettes={wire_sides}\nfil_resolution={wire_mesh_res}\n", b(simp_env), b(resize_lock));
                        t += &format!("relief_motif={}\nrelief_projection={}\nrelief_hauteur={relief_height}\nrelief_echelle={relief_scale}\nrelief_inverser={}\nrelief_face_seule={}\n",
                            relief_pattern.key(), match relief_mapping { relief::Mapping::PlanarZ => "planz", relief::Mapping::Cylindrical => "cyl", relief::Mapping::Triplanar => "tri" }, b(relief_invert), b(relief_face_only));
                        t += &format!("taille_snap={snap_size}\ntaille_primitive={prim_size}\nlumiere={},{},{}\n", l.x, l.y, l.z);
                        t += &format!("imp_angle={imp_angle}\nimp_buse={imp_buse}\nimp_matiere={imp_mat}\n");
                        if imp_tr != usize::MAX { t += &format!("imp_trancheur={imp_tr}\n"); }
                        if let Some(a) = &imp_autre { t += &format!("imp_autre={}\n", a.display()); }
                        if t != prefs_ecrites {
                            // en visionneuse, on garde les réglages d'affichage du modeleur déjà mémorisés
                            if viewer { for k in ["regles", "vue"] { if let Some(v) = paths::prefs().get(k) { t += &format!("{k}={v}\n"); } } }
                            paths::save_prefs(&t);
                            prefs_ecrites = t;
                        }
                    }
                    // immédiat seulement si egui le demande sans délai ; sinon réveil programmé
                    // (la scène a changé depuis son dessin : nouvelle image tout de suite)
                    // panneau Booléens ouvert / fermé pendant cette image : la bascule en 4 vues (ou le retour à la vue
                    // d'avant) se fait au début de l'image suivante, à demander tout de suite
                    if panneau_bool != panneau_prec || cle_outils!() != outils_avant { window.request_redraw(); }
                    if delai_repaint.is_zero() || cle_scene!() != cle_dessinee { window.request_redraw(); }
                    else if delai_repaint < std::time::Duration::from_secs(3600) {
                        let t = std::time::Instant::now() + delai_repaint;
                        let t = prochain_repaint.map_or(t, |p| p.min(t));
                        prochain_repaint = Some(t);
                        // posé ici aussi : sous Windows le dessin arrive APRÈS « AboutToWait », la boucle
                        // dormirait sinon jusqu'au prochain événement (engrenage, animations figés)
                        elwt.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(t));
                    }
                }
                _ => {}
            }
        }
    }).unwrap();
}

#[cfg(test)]
mod tests_traduction {
    use super::*;
    /// Code de l'appli seul (sans ce module de test, qui cite des exemples).
    fn code_appli() -> &'static str {
        let s = include_str!("main.rs");
        &s[..s.find(&["#[cfg(test)]\nmod tests_", "traduction"].concat()).unwrap_or(s.len())]
    }
    /// Chaque clé passée à tr() dans le code existe et est remplie dans les 5 langues.
    #[test]
    fn toutes_les_cles_sont_traduites() {
        let src = code_appli();
        let mut cles = std::collections::BTreeSet::new();
        for (i, _) in src.match_indices("tr(") {
            let reste = &src[i + 3..];
            // tr(lang, "cle") ou tr(Lang::Fr, "cle") ; ignore les autres fonctions finissant par « tr(»
            if i > 0 && (src.as_bytes()[i - 1].is_ascii_alphanumeric() || src.as_bytes()[i - 1] == b'_') { continue; }
            let Some(v) = reste.find(',') else { continue };
            let apres = reste[v + 1..].trim_start();
            if let Some(q) = apres.strip_prefix('"') { if let Some(f) = q.find('"') { cles.insert(q[..f].to_string()); } }
        }
        // clés des modules : crate::trc(clé)
        for f in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap().flatten() {
            let t = std::fs::read_to_string(f.path()).unwrap_or_default();
            for (i, _) in t.match_indices("trc(\"") {
                if let Some(e) = t[i + 5..].find('"') { cles.insert(t[i + 5..i + 5 + e].to_string()); }
            }
        }
        // clés construites dynamiquement (états de la grille)
        for k in ["g_etat0", "g_etat1", "g_etat2", "g_etat3"] { cles.insert(k.into()); }
        assert!(cles.len() > 100, "extraction des clés suspecte : {}", cles.len());
        let mut manquantes = Vec::new();
        for k in &cles { for l in LANGS { let t = tr(l, k); if t == "?" || t.trim().is_empty() { manquantes.push(format!("{k} [{}]", l.name())); } } }
        assert!(manquantes.is_empty(), "clés sans traduction : {manquantes:?}");
    }
    /// Aucune infobulle écrite en dur (texte littéral) : elles doivent toutes passer par tr().
    #[test]
    fn aucune_infobulle_en_dur() {
        let src = code_appli();
        let fautives: Vec<_> = src.lines().enumerate()
            .filter(|(_, l)| l.contains(&["on_hover_text(", "\""].concat()) || l.contains(&["on_hover_text(", "format!(\"", "A"].concat()))
            .map(|(n, l)| format!("{}: {}", n + 1, l.trim())).collect();
        assert!(fautives.is_empty(), "infobulles non traduites : {fautives:#?}");
    }
}
