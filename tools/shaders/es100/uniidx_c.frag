/* gl_test 108 uniidx, program C: program B's read with unrelated ALU work
 * selected between the movar and the read (the listing is the check:
 * three instructions apart, as gl_test 100 idxtex's are). */
precision mediump float;
uniform mediump int ui_two;
uniform mediump float u_arr[4];
varying vec4 v_color;
void main() {
    float k = u_arr[ui_two];
    vec4 c = v_color * 0.5 + vec4(0.25);
    c = c * c + vec4(0.125);
    gl_FragColor = vec4(k + c.y * 0.0, 0.0, 0.0, 1.0);
}
