#version 300 es
/* gl_test 199: es3trunc_fs.frag's arithmetic in the vertex stage. */
in vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
out vec4 v;
void main() {
    int i = int(u_q);
    uint u = uint(u_a);
    gl_Position = a_pos;
    v = vec4((trunc(u_x) + 2.0) * 0.125, (trunc(-u_x) + 4.0) * 0.125,
             float((i >> 1) - (i << 3)) / 64.0, float(u >> 2u) / 64.0);
}
