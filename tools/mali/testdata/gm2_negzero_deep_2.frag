#version 100
/* #33 plan #367 lower-const-fold: a discard on a value 11 products deep under a texel (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family NEG_ZERO). */
precision highp float;
uniform highp sampler2D u_s0;
varying vec2 v_uv;
void main() {
    float h = (((((((((((texture2D(u_s0, v_uv).x * 2.5 + -0.5) * 0.75 + 0.125) * 1.5 + 0.25)
        * 2.5 + -0.5) * 0.75 + 0.125) * 1.5 + 0.25) * 2.5 + -0.5) * 0.75 + 0.125) * 1.5 + 0.25)
        * 2.5 + -0.5) * 0.75 + 0.125);
    if (h < 0.0)
        discard;
    gl_FragColor = vec4(1.0);
}
