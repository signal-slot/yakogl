#version 100
/* glmark2 22c527cb ideas, vertex shader a09e06ca (the text): a one-armed
 * `if` on a float uniform compared with a constant that assigns a value
 * from that uniform to a local, which starts at 0.0 and goes into a
 * varying. */
attribute vec2 a_xy;
uniform mat4 u_proj;
uniform mat4 u_mv;
uniform float u_time;
varying vec4 v_col;
void main() {
    gl_Position = u_proj * u_mv * vec4(a_xy, 0.0, 1.0);
    float c = 0.0;
    if (u_time > 10.0) {
        c = (u_time - 10.0) / 2.0;
    }
    v_col = vec4(c, c, c, 1.0);
}
