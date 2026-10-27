/* #1385 (b), plan #153 uniarray: gl_test case 108 uniidx program B,
 * tools/shaders/es100/uniidx_b.frag verbatim (one run-time read of a
 * mediump float array, right after its movar; tools/mali/drm_shim_ref.sh
 * run, Mesa 26.2.1; the pair's vertex shader is case108_uniidx.vert). */
precision mediump float;
uniform mediump int ui_two;
uniform mediump float u_arr[4];
void main() {
    gl_FragColor = vec4(u_arr[ui_two], 0.0, 0.0, 1.0);
}
