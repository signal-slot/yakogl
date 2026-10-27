/* #1385 (b), plan #153 uniarray: gl_test case 108 uniidx program C,
 * tools/shaders/es100/uniidx_c.frag verbatim (program B's read with
 * unrelated ALU work between the movar and the read; tools/mali/
 * drm_shim_ref.sh run, Mesa 26.2.1; the pair's vertex shader is
 * case108_uniidx.vert). */
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
