//! Règles graduées transparentes sur les axes +X / +Y / +Z (traits).
//! Les chiffres (en cm) sont dessinés par-dessus via egui (voir main.rs).
//! `step` = graduation en unités monde (10 = 1 cm si 1 unité = 1 mm).

use glow::HasContext;

const VS: &str = r#"#version 330 core
layout(location=0) in vec3 aPos;
uniform mat4 uMVP;
void main(){ gl_Position = uMVP * vec4(aPos,1.0); }"#;

const FS: &str = r#"#version 330 core
uniform vec4 uColor;
out vec4 frag;
void main(){ frag = uColor; }"#;

pub struct Ruler {
    program: glow::NativeProgram,
    vao: glow::NativeVertexArray,
    u_mvp: Option<glow::NativeUniformLocation>,
    u_color: Option<glow::NativeUniformLocation>,
    count: i32,
    pub step: f32,
    pub majors: i32, // nombre de graduations sur chaque axe
}

impl Ruler {
    pub unsafe fn new(gl: &glow::Context, step: f32, majors: i32) -> Ruler {
        let mut v: Vec<f32> = Vec::new();
        let mut seg = |a: [f32; 3], b: [f32; 3]| v.extend_from_slice(&[a[0], a[1], a[2], b[0], b[1], b[2]]);
        let tick_m = step * 0.15;  // longueur trait mm
        let tick_c = step * 0.45;  // longueur trait cm
        // Traits : tous les mm (petit), renforcés tous les cm (grand), sur chaque axe +X,+Y,+Z.
        let nmm = majors * 10;
        for i in 1..=nmm {
            let p = i as f32 * (step / 10.0);
            let t = if i % 10 == 0 { tick_c } else { tick_m };
            // axe X : traits dans le plan, vers +Y et vers +Z
            seg([p, 0.0, 0.0], [p, -t, 0.0]);
            // axe Y : traits vers +X et vers +Z
            seg([0.0, p, 0.0], [-t, p, 0.0]);
            // axe Z : traits vers +X
            seg([0.0, 0.0, p], [-t, 0.0, p]);
        }

        let program = gl.create_program().unwrap();
        for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let s = gl.create_shader(ty).unwrap();
            gl.shader_source(s, src);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) { panic!("ruler shader: {}", gl.get_shader_info_log(s)); }
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
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 3 * 4, 0);

        Ruler {
            program, vao,
            u_mvp: gl.get_uniform_location(program, "uMVP"),
            u_color: gl.get_uniform_location(program, "uColor"),
            count: (v.len() / 3) as i32,
            step, majors,
        }
    }

    pub unsafe fn draw(&self, gl: &glow::Context, mvp: &[f32; 16]) {
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.use_program(Some(self.program));
        gl.uniform_matrix_4_f32_slice(self.u_mvp.as_ref(), false, mvp);
        gl.uniform_4_f32(self.u_color.as_ref(), 0.92, 0.95, 1.0, 0.55); // blanc translucide, lisible
        gl.bind_vertex_array(Some(self.vao));
        gl.draw_arrays(glow::LINES, 0, self.count);
        gl.disable(glow::BLEND);
    }
}
