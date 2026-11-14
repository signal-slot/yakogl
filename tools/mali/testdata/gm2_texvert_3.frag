#version 100
/* #32 plan #367 ref-tex-vertex: fragment shader of the gm2_texvert_3 pair
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; see
 * docs/notes/mali-vs-texture-lanes.md). */
precision mediump float;
varying vec3 v_n;
void main() {
    gl_FragColor = vec4(v_n, 1.0);
}
