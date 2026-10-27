/* #1385 (b), plan #153 uniarray: the colour of uniarray_vs.vert and
 * uniarray_vsconst.vert, read whole (tools/mali/drm_shim_ref.sh run, Mesa
 * 26.2.1). */
precision mediump float;
varying vec4 v_color;
void main() {
    gl_FragColor = v_color;
}
