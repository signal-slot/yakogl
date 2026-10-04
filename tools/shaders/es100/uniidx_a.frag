/* gl_test 108 uniidx, program A: dEQP-GLES2 indexing.uniform_array.
 * float_dynamic_read_fragment's shape verbatim (four reads of u_arr at
 * uniform ints, each right after the movar that indexes it). */
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
