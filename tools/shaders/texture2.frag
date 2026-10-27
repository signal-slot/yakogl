#version 450
/* Two samplers: the unit-0 texture weighted 0.5 plus the unit-1 texture weighted 0.25. */
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;
layout(set = 0, binding = 0) uniform sampler2D u_tex0;
layout(set = 0, binding = 1) uniform sampler2D u_tex1;
void main() {
    o_color = vec4(texture(u_tex0, v_uv).rgb * 0.5 + texture(u_tex1, v_uv).rgb * 0.25, 1.0);
}
