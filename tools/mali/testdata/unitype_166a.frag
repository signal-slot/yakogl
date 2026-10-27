/* gl_test case 166 fshprobe, arm A (driver/tools/pvrlab/gl_test.c shp_src, n_vec4=64, text regenerated, not hand-written). */
precision mediump float;
uniform vec4 u_p[64];
void main() {
    vec4 r = u_p[58].y * vec4(1.0, 0.0, 1.0, 1.0);
    r += u_p[56].x * u_p[0];
    r += u_p[56].y * u_p[32];
    r += u_p[56].z * u_p[60];
    r += u_p[56].w * u_p[63];
    gl_FragColor = r;
}
