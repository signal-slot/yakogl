#version 450
/* Uniform/constant colour FS. u_color -> sh0..3 via DOUTD (push-const). */
layout(push_constant) uniform U { vec4 u_color; };
layout(location = 0) out vec4 o_color;
void main() { o_color = u_color; }
