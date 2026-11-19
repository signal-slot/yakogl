#version 100
/* #33 plan #367 lower-const-fold: step() of a vec4 built from a value 11 products deep, against
 * vec4(0.0) (original reduction) (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family NEG_ZERO,
 * the vector form of terrain fs 1637's `step(h, vec4(0.0))`). */
precision highp float;
uniform highp sampler2D u_s0;
varying vec2 v_uv;
void main() {
    float h = (((((((((((texture2D(u_s0, v_uv).x * 2.5 + -0.5) * 0.75 + 0.125) * 1.5 + 0.25)
        * 2.5 + -0.5) * 0.75 + 0.125) * 1.5 + 0.25) * 2.5 + -0.5) * 0.75 + 0.125) * 1.5 + 0.25)
        * 2.5 + -0.5) * 0.75 + 0.125);
    vec4 hh = vec4(h, h * 2.0, h + 1.0, h - 1.0);
    gl_FragColor = step(hh, vec4(0.0));
}
