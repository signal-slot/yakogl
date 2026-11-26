#version 300 es
/* glGenerateMipmap on the GPU, three levels a render (the PowerVR's
 * src/pvr/backend.rs `record_mip_levels`, behind
 * `pvr_backend_set_mip_three_levels`, off by default;
 * docs/notes/pvr-mip-three-levels.md): levels l+1, l+2 and l+3 of a 2D RGBA8
 * chain drawn from level l -- bound alone, one level, linear filters,
 * CLAMP_TO_EDGE -- into three colour targets by one full-screen quad over
 * level l+1's extent. The block holds (1/W, 1/H, W, H) of each of the three
 * levels, 1/W and 1/H made as `record_mip_level` makes its u_scale; a render
 * of two levels has u_l3 zero.
 *
 * o0, level l+1: mipgen.frag's statement, the same sample at the same
 * coordinate bits, so the level is the one a level kick draws.
 * o1, level l+2: the bilinear sample a level kick would take of level l+1,
 * at the same point (u = (p + 0.5) / W2 * W1 - 0.5, its floor, fraction and
 * clamp), over level l+1's texels as o0 makes them (the leaves, packed to
 * 0..255 by Q), in the ALU, packed by Q again: the 2x box on an even step.
 * o2, level l+3: the same over four level-l+2 texels made as o1 makes them.
 * The pixels past a level's extent compute nothing for it; the PBE's clip
 * stores only the extent.
 *
 * Q is round to nearest, ties up: floor(v * 255 + 0.5) (E1's choice,
 * docs/notes/pvr-mip-tail-e1.md; the board's pack may round ties otherwise,
 * and only the ALU levels can show it). The leaves sit in non-uniform
 * control flow, so they are textureLod at 0, which a one-level sampler with
 * linear min and mag filters answers as texture() does. */
precision highp float;
uniform highp sampler2D u_src;
uniform vec4 u_l1;
uniform vec4 u_l2;
uniform vec4 u_l3;
layout(location = 0) out vec4 o0;
layout(location = 1) out vec4 o1;
layout(location = 2) out vec4 o2;

/* level l+1's texel q, as 0..255 */
vec4 leaf(vec2 q) {
    return floor(textureLod(u_src, (q + 0.5) * u_l1.xy, 0.0) * 255.0 + 0.5);
}

/* level l+2's texel q, as 0..255: the bilinear of level l+1's texels */
vec4 level2(vec2 q) {
    vec2 u = (q + 0.5) * u_l2.xy * u_l1.zw - 0.5;
    vec2 i0 = floor(u);
    vec2 f = u - i0;
    vec2 a = clamp(i0, vec2(0.0), u_l1.zw - 1.0);
    vec2 b = clamp(i0 + 1.0, vec2(0.0), u_l1.zw - 1.0);
    vec4 t00 = leaf(a);
    vec4 t10 = leaf(vec2(b.x, a.y));
    vec4 t01 = leaf(vec2(a.x, b.y));
    vec4 t11 = leaf(b);
    return floor(mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y) + 0.5);
}

void main() {
    o0 = texture(u_src, gl_FragCoord.xy * u_l1.xy);
    vec2 p = floor(gl_FragCoord.xy);
    o1 = vec4(0.0);
    o2 = vec4(0.0);
    if (p.x < u_l2.z && p.y < u_l2.w) {
        o1 = level2(p) * (1.0 / 255.0);
    }
    if (p.x < u_l3.z && p.y < u_l3.w) {
        vec2 u = (p + 0.5) * u_l3.xy * u_l2.zw - 0.5;
        vec2 i0 = floor(u);
        vec2 f = u - i0;
        vec2 a = clamp(i0, vec2(0.0), u_l2.zw - 1.0);
        vec2 b = clamp(i0 + 1.0, vec2(0.0), u_l2.zw - 1.0);
        vec4 t00 = level2(a);
        vec4 t10 = level2(vec2(b.x, a.y));
        vec4 t01 = level2(vec2(a.x, b.y));
        vec4 t11 = level2(b);
        o2 = floor(mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y) + 0.5) * (1.0 / 255.0);
    }
}
