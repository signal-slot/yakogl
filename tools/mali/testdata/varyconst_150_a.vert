#version 100
// gl_test 150 shprobe, arm A: the vertex shader shp_src(64) generates
// (driver/tools/pvrlab/gl_test.c), verbatim apart from this comment. The
// first term reads u_p[58].y alone, so v_c's first lane is computed from a
// uniform and a constant with no attribute.
attribute vec4 a_pos;
uniform vec4 u_p[64];
varying vec4 v_c;
void main() {
    vec4 r = u_p[58].y * vec4(1.0, 0.0, 1.0, 1.0);
    r += u_p[56].x * u_p[0];
    r += u_p[56].y * u_p[32];
    r += u_p[56].z * u_p[60];
    r += u_p[56].w * u_p[63];
    v_c = r;
    gl_Position = a_pos;
}
