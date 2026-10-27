/* #1385 (b), plan #153 uniarray: case 108 A's four run-time reads of a
 * float array (case108_uniidx.frag) at highp: Mesa keeps the block fp32,
 * so the element is a 32-bit load (tools/mali/drm_shim_ref.sh run, Mesa
 * 26.2.1; the pair's vertex shader is case108_uniidx.vert). */
precision highp float;
uniform highp int ui_zero, ui_one, ui_two, ui_three;
uniform highp float u_arr[4];
void main() {
    highp float res = float(0.0);
    res += u_arr[ui_zero];
    res += u_arr[ui_one];
    res += u_arr[ui_two];
    res += u_arr[ui_three];
    gl_FragColor = vec4(res, 0.0, 0.0, 1.0);
}
