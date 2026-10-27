#version 300 es
/* glGenerateMipmap on the GPU (the PowerVR's per-level quad,
 * src/pvr/backend.rs pvr_generate_mipmap): level n of a 2D RGBA8 chain is
 * drawn as a full-screen quad into that level, and every fragment samples
 * level n-1 -- bound alone, one level, linear filters, CLAMP_TO_EDGE -- at
 * the centre of its own texel: gl_FragCoord.xy * u_scale, where u_scale is
 * 1 / the level's size. When the level is half the one above (every level
 * of a power-of-two chain) that point is the middle of the 2x2 box of
 * texels under it, and the bilinear sample is the box's average, the filter
 * the GL core's CPU path makes (gl/texture.rs glGenerateMipmap); on a
 * non-power-of-two chain's odd steps (25 -> 12) it is the bilinear sample
 * between the nearest four, which ES 3.0.6 3.8.9 allows ("box filter, or
 * other").
 *
 * The quad is drawn unflipped, so the pair the backend writes for
 * gl_FragCoord.y (fbload.frag's u_flip) is (1, 0) and the register holds the
 * row in memory order: nothing to undo. */
precision highp float;
uniform highp sampler2D u_src;
uniform vec2 u_scale;
layout(location = 0) out vec4 o_color;
void main() {
    o_color = texture(u_src, gl_FragCoord.xy * u_scale);
}
