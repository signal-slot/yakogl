#version 100
/* glmark2 22c527cb loop (all three scenes), vertex shader 3cb095bb: a for
 * loop with an int counter from 0 whose bound is an int uniform, its body a
 * fract of a constant times the value, in a vertex shader. */
attribute vec3 a_pos;
uniform mat4 u_mvp;
uniform int u_n;
void main() {
    float t = fract(a_pos.x);
    for (int k = 0; k < u_n; k++)
        t = fract(3.0 * t);
    gl_Position = u_mvp * vec4(a_pos.x, a_pos.y + t, a_pos.z, 1.0);
}
