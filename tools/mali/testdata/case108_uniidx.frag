/* #1385 (b), plan #153 study-uniarray: gl_test case 108 uniidx program A,
 * tools/shaders/es100/uniidx_a.frag verbatim (dEQP-GLES2 float_dynamic_read:
 * four reads of a float uniform array at uniform ints; tools/mali/drm_shim_ref.sh
 * run (no --uniform-values: gl_ref.py refuses them for an int uniform), Mesa 26.2.1; the pair's vertex shader is
 * case108_uniidx.vert). */
precision mediump float;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
uniform mediump float u_arr[4];
void main() {
    mediump float res = float(0.0);
    res += u_arr[ui_zero];
    res += u_arr[ui_one];
    res += u_arr[ui_two];
    res += u_arr[ui_three];
    gl_FragColor = vec4(res, 0.0, 0.0, 1.0);
}
