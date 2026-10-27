/* #1385 (b): a mediump vec4 array read at a run-time index alone,
 * tools/shaders/es100/uniidx_d.frag (gl_test case 108 D) verbatim: the
 * LD_PKA row without the look-up (tools/mali/drm_shim_ref.sh run, Mesa
 * 26.2.1, --uniform-values; the pair's vertex shader is uniidx_i.vert). */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
void main() {
    gl_FragColor = fcols[fidx];
}
