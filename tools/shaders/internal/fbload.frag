#version 450
/* Background load: copy the previous framebuffer contents of this pixel. */
layout(set = 0, binding = 0) uniform sampler2D u_fb;
layout(location = 0) out vec4 o_color;
void main() {
    o_color = texelFetch(u_fb, ivec2(gl_FragCoord.xy), 0);
}
