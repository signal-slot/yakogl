#version 100
// gl_test 150 shprobe, arm B: the vertex shader shp_src(SHP_MAX_VEC4 = 70)
// generates (driver/tools/pvrlab/gl_test.c), verbatim apart from this
// comment: arm A's reads plus u_p[64..69] (sh256 and up).
attribute vec4 a_pos;
uniform vec4 u_p[70];
varying vec4 v_c;
void main() {
    vec4 r = u_p[58].y * vec4(1.0, 0.0, 1.0, 1.0);
    r += u_p[56].x * u_p[0];
    r += u_p[56].y * u_p[32];
    r += u_p[56].z * u_p[60];
    r += u_p[56].w * u_p[63];
    r += u_p[57].x * u_p[64];
    r += u_p[57].y * u_p[65];
    r += u_p[57].z * u_p[66];
    r += u_p[57].w * u_p[67];
    r += u_p[58].x * u_p[69];
    v_c = r;
    gl_Position = a_pos;
}
