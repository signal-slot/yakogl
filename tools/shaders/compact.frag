#version 450
/* Two samplers are declared but only the second (binding 1) is sampled: the
 * compiler drops binding 0 and compacts binding 1 to sh0. The backend must
 * place the used sampler's descriptor there, not at binding*20. (gl_test
 * case 24 verifies this on silicon.) */
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;
layout(set = 0, binding = 0) uniform sampler2D u_tex0;
layout(set = 0, binding = 1) uniform sampler2D u_tex1;
void main() {
    o_color = texture(u_tex1, v_uv);
}
