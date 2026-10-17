//! Plaque translucide dessinée dans la scène 3D (cadre de coupe) : elle passe DERRIÈRE tous les
//! objets (profondeur forcée au fond), donc elle ne masque jamais leur couleur ; on ne la voit
//! que sur le fond, autour des objets.

use glow::HasContext;

const VS: &str = r#"#version 330 core
layout(location = 0) in vec3 aPos;
uniform mat4 uMvp;
uniform float uFond;
void main() {
    vec4 p = uMvp * vec4(aPos, 1.0);
    if (uFond > 0.5) p.z = p.w * 0.99999; // au fond : tout objet dessiné passe devant
    else p.z -= 0.0004 * p.w;             // collé à la surface, juste devant (pas de scintillement)
    gl_Position = p;
}"#;
const FS: &str = r#"#version 330 core
uniform vec4 uCol;
out vec4 frag;
void main() { frag = uCol; }"#;

pub struct Plaque { prog: glow::NativeProgram, vao: glow::NativeVertexArray, vbo: glow::NativeBuffer }

impl Plaque {
    pub unsafe fn new(gl: &glow::Context) -> Plaque {
        let prog = gl.create_program().unwrap();
        for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let s = gl.create_shader(ty).unwrap();
            gl.shader_source(s, src); gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) { panic!("shader plaque : {}", gl.get_shader_info_log(s)); }
            gl.attach_shader(prog, s);
        }
        gl.link_program(prog);
        let vao = gl.create_vertex_array().unwrap();
        let vbo = gl.create_buffer().unwrap();
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
        gl.bind_vertex_array(None);
        Plaque { prog, vao, vbo }
    }

    /// Dessine le quadrilatère `q` (4 coins) avec la couleur `col` (RGBA, alpha = transparence).
    pub unsafe fn dessiner(&self, gl: &glow::Context, mvp: &[f32; 16], q: &[glam::Vec3; 4], col: [f32; 4]) {
        let v: Vec<f32> = [0usize, 1, 2, 0, 2, 3].iter().flat_map(|&i| [q[i].x, q[i].y, q[i].z]).collect();
        self.triangles_gl(gl, mvp, &v, col, true);
    }

    /// Triangles posés SUR les surfaces des objets (profondeur réelle : cachés derrière l'objet).
    pub unsafe fn sur_objet(&self, gl: &glow::Context, mvp: &[f32; 16], tris: &[[glam::Vec3; 3]], col: [f32; 4]) {
        let v: Vec<f32> = tris.iter().flatten().flat_map(|p| [p.x, p.y, p.z]).collect();
        self.triangles_gl(gl, mvp, &v, col, false);
    }

    unsafe fn triangles_gl(&self, gl: &glow::Context, mvp: &[f32; 16], v: &[f32], col: [f32; 4], fond: bool) {
        if v.is_empty() { return; }
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_f32(v), glow::DYNAMIC_DRAW);
        gl.use_program(Some(self.prog));
        gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(self.prog, "uMvp").as_ref(), false, mvp);
        gl.uniform_4_f32(gl.get_uniform_location(self.prog, "uCol").as_ref(), col[0], col[1], col[2], col[3]);
        gl.uniform_1_f32(gl.get_uniform_location(self.prog, "uFond").as_ref(), if fond { 1.0 } else { 0.0 });
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(false);
        gl.bind_vertex_array(Some(self.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, (v.len() / 3) as i32);
        gl.bind_vertex_array(None);
        gl.depth_mask(true);
        gl.depth_func(glow::LESS);
        gl.disable(glow::BLEND);
    }
}

fn bytemuck_f32(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}
