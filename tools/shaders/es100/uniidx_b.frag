/* gl_test 108 uniidx, program B: one run-time read of a float array,
 * right after its movar. */
precision mediump float;
uniform mediump int ui_two;
uniform mediump float u_arr[4];
void main() {
    gl_FragColor = vec4(u_arr[ui_two], 0.0, 0.0, 1.0);
}
