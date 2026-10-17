//! Grille de construction sur le plan X-Y (Z=0), carrés de 1 cm (10 mm).
//! Traits fins tous les 1 cm, renforcés tous les 10 cm ; axe X rouge, axe Y vert, axe Z bleu.

use glow::HasContext;

const VS: &str = r#"#version 330 core
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aCol;
uniform mat4 uMVP;
out vec3 vCol;
void main(){ vCol=aCol; gl_Position=uMVP*vec4(aPos,1.0); }"#;

const FS: &str = r#"#version 330 core
in vec3 vCol; out vec4 frag;
void main(){ frag=vec4(vCol,1.0); }"#;

pub struct Grid {
    program: glow::NativeProgram,
    vao: glow::NativeVertexArray,
    u_mvp: Option<glow::NativeUniformLocation>,
    count: i32,
    /// nombre de sommets des lignes de grille (les axes suivent)
    n_grille: i32,
}

impl Grid {
    /// `step` = pas de la grille en unités monde (10.0 = 1 cm si 1 unité = 1 mm).
    pub unsafe fn new(gl: &glow::Context, step: f32, lines: i32) -> Grid {
        let ext = lines as f32 * step;
        let minor = [0.28, 0.31, 0.36];
        let major = [0.42, 0.46, 0.52];
        let ax_x = [0.80, 0.30, 0.30];
        let ax_y = [0.35, 0.70, 0.38];
        let ax_z = [0.35, 0.55, 0.85];
        let mut v: Vec<f32> = Vec::new();
        let mut push = |x: f32, y: f32, z: f32, c: [f32; 3]| {
            v.extend_from_slice(&[x, y, z, c[0], c[1], c[2]]);
        };
        // lignes de grille (sans les axes)
        for i in -lines..=lines {
            if i == 0 { continue; }
            let p = i as f32 * step;
            let c = if i % 10 == 0 { major } else { minor };
            push(-ext, p, 0.0, c); push(ext, p, 0.0, c); // parallèles à X
            push(p, -ext, 0.0, c); push(p, ext, 0.0, c); // parallèles à Y
        }
        let n_grille = 8 * lines; // 2 lignes × 2 sommets × 2·lines valeurs de i (0 exclu)
        // axes X (rouge), Y (vert), Z (bleu, vertical)
        push(-ext, 0.0, 0.0, ax_x); push(ext, 0.0, 0.0, ax_x);
        push(0.0, -ext, 0.0, ax_y); push(0.0, ext, 0.0, ax_y);
        push(0.0, 0.0, 0.0, ax_z); push(0.0, 0.0, ext * 0.25, ax_z);

        let program = gl.create_program().unwrap();
        for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let s = gl.create_shader(ty).unwrap();
            gl.shader_source(s, src);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) { panic!("grid shader: {}", gl.get_shader_info_log(s)); }
            gl.attach_shader(program, s);
        }
        gl.link_program(program);

        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(vao));
        let vbo = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let bytes: &[u8] = core::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        crate::cache3d::touche();
        let stride = 6 * 4;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 3 * 4);
        let u_mvp = gl.get_uniform_location(program, "uMVP");

        Grid { program, vao, u_mvp, count: (v.len() / 6) as i32, n_grille }
    }

    /// `grille` = lignes de la grille, `axes` = axes X/Y/Z colorés.
    pub unsafe fn draw(&self, gl: &glow::Context, mvp: &[f32; 16], grille: bool, axes: bool) {
        if !grille && !axes { return; }
        gl.use_program(Some(self.program));
        gl.uniform_matrix_4_f32_slice(self.u_mvp.as_ref(), false, mvp);
        gl.bind_vertex_array(Some(self.vao));
        if grille { gl.draw_arrays(glow::LINES, 0, self.n_grille); }
        if axes { gl.draw_arrays(glow::LINES, self.n_grille, self.count - self.n_grille); }
    }
}
