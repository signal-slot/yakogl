#version 300 es
/* Background load: copy the previous framebuffer contents of this pixel
 * (the PowerVR's frame quad, when a frame does not clear its target). The
 * ES 3.00 twin of ../fbload.frag: texelFetch is ES 3.00.
 *
 * The texel is the pixel's own, in memory order. This compiler hands the
 * shader gl_FragCoord.y in GL's orientation -- mirrored through the pair
 * (scale, offset) the backend writes per draw, (-1, height) on a flipped
 * target and (1, 0) otherwise -- so u_flip is that pair again, which undoes
 * it: y * s + o with the same (s, o) is the register's own y on both. */
precision highp float;
uniform highp sampler2D u_fb;
uniform vec2 u_flip;
layout(location = 0) out vec4 o_color;
void main() {
    float y = gl_FragCoord.y * u_flip.x + u_flip.y;
    o_color = texelFetch(u_fb, ivec2(int(gl_FragCoord.x), int(y)), 0);
}
