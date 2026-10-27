#version 300 es
/* Background load of an integer target: copy the previous contents of this
 * pixel (the PowerVR's frame quad, when a frame does not clear its target),
 * fbload.frag for a target whose pixel is integer bits. The back end binds
 * the target as an unsigned texel of the pixel's size (8, 16, 32, 64 or 128
 * bits) and compiles this for the 32-bit layout of as many dwords
 * (replace@r32ui, @rg32ui, @rgba32ui): the bits go back to the pixel output
 * registers as they came, and the PBE packs them for the target as it packs
 * any draw's.
 *
 * u_flip undoes the mirror this compiler applies to gl_FragCoord.y, as in
 * fbload.frag. */
precision highp float;
uniform highp usampler2D u_fb;
uniform vec2 u_flip;
layout(location = 0) out highp uvec4 o_color;
void main() {
    float y = gl_FragCoord.y * u_flip.x + u_flip.y;
    o_color = texelFetch(u_fb, ivec2(int(gl_FragCoord.x), int(y)), 0);
}
