#version 450
/* MVP VS: u_mvp (mat4) -> sh0..15 via DOUTD (push-const). */
layout(push_constant) uniform U { mat4 u_mvp; };
layout(location = 0) in vec4 a_pos;
void main() { gl_Position = u_mvp * a_pos; }
