//! Image 3D mémorisée : la scène est dessinée dans une image hors écran, puis recopiée à chaque
//! image affichée. Quand rien de ce qui compose la scène n'a changé (seule une animation de
//! l'interface tourne, comme l'engrenage), on recopie l'image gardée au lieu de tout redessiner.

use glow::HasContext;
use std::sync::atomic::{AtomicU64, Ordering};

/// Compteur de modifications des tampons GL de la scène (objets, grille, règle, défauts).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// À appeler dès qu'un tampon dessiné dans la scène 3D change : l'image gardée n'est plus valable.
pub fn touche() { GENERATION.fetch_add(1, Ordering::Relaxed); }
pub fn generation() -> u64 { GENERATION.load(Ordering::Relaxed) }

const VS: &str = r#"#version 330 core
out vec2 vUv;
void main() {
    // triangle couvrant tout l'écran, sans tampon de sommets
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    vUv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}"#;
const FS: &str = r#"#version 330 core
in vec2 vUv;
uniform sampler2D uTex;
out vec4 frag;
void main() { frag = texture(uTex, vUv); }"#;

pub struct Cache3d {
    prog: glow::NativeProgram,
    vao: glow::NativeVertexArray,
    samples: i32,
    w: i32,
    h: i32,
    // rendu (multi-échantillonné si l'écran l'est) puis résolu dans `tex`
    ms_fbo: Option<glow::NativeFramebuffer>,
    ms_rb: Vec<glow::NativeRenderbuffer>,
    fbo: Option<glow::NativeFramebuffer>,
    dep: Option<glow::NativeRenderbuffer>,
    tex: Option<glow::NativeTexture>,
    cle: Option<String>,
    /// caméra de chaque vue (ordre de view::ALL_VIEWS) lors de son dernier dessin
    cles_vues: [Option<String>; 4],
}

impl Cache3d {
    /// À créer contexte GL courant, framebuffer de la fenêtre lié (pour lire son anticrénelage).
    pub unsafe fn new(gl: &glow::Context) -> Cache3d {
        let prog = gl.create_program().unwrap();
        for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let s = gl.create_shader(ty).unwrap();
            gl.shader_source(s, src); gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) { panic!("shader image 3D : {}", gl.get_shader_info_log(s)); }
            gl.attach_shader(prog, s);
        }
        gl.link_program(prog);
        let vao = gl.create_vertex_array().unwrap();
        let samples = gl.get_parameter_i32(glow::SAMPLES).max(0).min(gl.get_parameter_i32(glow::MAX_SAMPLES).max(0));
        Cache3d { prog, vao, samples, w: 0, h: 0, ms_fbo: None, ms_rb: Vec::new(), fbo: None, dep: None, tex: None, cle: None, cles_vues: Default::default() }
    }

    unsafe fn liberer(&mut self, gl: &glow::Context) {
        if let Some(f) = self.ms_fbo.take() { gl.delete_framebuffer(f); }
        if let Some(f) = self.fbo.take() { gl.delete_framebuffer(f); }
        for rb in self.ms_rb.drain(..) { gl.delete_renderbuffer(rb); }
        if let Some(rb) = self.dep.take() { gl.delete_renderbuffer(rb); }
        if let Some(t) = self.tex.take() { gl.delete_texture(t); }
    }

    unsafe fn preparer(&mut self, gl: &glow::Context, w: i32, h: i32) {
        if self.tex.is_some() && w == self.w && h == self.h { return; }
        self.liberer(gl);
        self.w = w; self.h = h; self.cle = None; self.cles_vues = Default::default();
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, None);
        for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32); }
        for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32); }
        gl.bind_texture(glow::TEXTURE_2D, None);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        if self.samples > 1 {
            let ms = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(ms));
            for (fmt, att) in [(glow::RGBA8, glow::COLOR_ATTACHMENT0), (glow::DEPTH_COMPONENT24, glow::DEPTH_ATTACHMENT)] {
                let rb = gl.create_renderbuffer().unwrap();
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
                gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, self.samples, fmt, w, h);
                gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, att, glow::RENDERBUFFER, Some(rb));
                self.ms_rb.push(rb);
            }
            self.ms_fbo = Some(ms);
        } else {
            let rb = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, w, h);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(rb));
            self.dep = Some(rb);
        }
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        self.fbo = Some(fbo); self.tex = Some(tex);
    }

    /// Quelles vues redessiner ? `cle` résume ce qui compose la scène hors caméras, `vues` la caméra de
    /// chaque vue ; `forcer` = tout redessiner. Une vue dont seule la caméra d'une AUTRE vue a changé
    /// garde son image (zoomer dans une fenêtre ne redessine pas les trois autres).
    /// Si une vue est à refaire, le framebuffer hors écran est lié : la dessiner, puis appeler `fin`.
    pub unsafe fn debut(&mut self, gl: &glow::Context, w: i32, h: i32, cle: String, vues: [String; 4], forcer: bool) -> Option<[bool; 4]> {
        self.preparer(gl, w.max(1), h.max(1));
        let tout = forcer || self.cle.as_deref() != Some(cle.as_str());
        let a_faire: [bool; 4] = std::array::from_fn(|i| tout || self.cles_vues[i].as_deref() != Some(vues[i].as_str()));
        if !a_faire.contains(&true) { return None; }
        self.cle = Some(cle);
        self.cles_vues = vues.map(Some);
        gl.bind_framebuffer(glow::FRAMEBUFFER, self.ms_fbo.or(self.fbo));
        Some(a_faire)
    }

    pub unsafe fn fin(&self, gl: &glow::Context) {
        if let (Some(ms), Some(f)) = (self.ms_fbo, self.fbo) {
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(ms));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(f));
            gl.blit_framebuffer(0, 0, self.w, self.h, 0, 0, self.w, self.h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }

    /// Recopie l'image gardée dans la fenêtre (avant le dessin de l'interface).
    pub unsafe fn afficher(&self, gl: &glow::Context) {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(0, 0, self.w, self.h);
        gl.disable(glow::DEPTH_TEST); gl.disable(glow::BLEND); gl.disable(glow::SCISSOR_TEST); gl.disable(glow::CULL_FACE);
        gl.use_program(Some(self.prog));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, self.tex);
        gl.uniform_1_i32(gl.get_uniform_location(self.prog, "uTex").as_ref(), 0);
        gl.bind_vertex_array(Some(self.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }
}
