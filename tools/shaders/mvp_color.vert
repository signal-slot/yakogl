#version 450
/* MVP + colour VS: u_mvp (mat4) in the push-constant block, a_pos / a_color
 * attributes, v_color out (pairs with interp_color.frag). */
layout(push_constant) uniform U { mat4 u_mvp; };
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_color;
layout(location = 0) out vec4 v_color;
void main() {
    gl_Position = u_mvp * a_pos;
    v_color = a_color;
}
