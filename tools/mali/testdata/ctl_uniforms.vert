/* Control: a vec4 and a float uniform in the default block (packed layout, FAU push). */
attribute vec4 a_pos;
uniform vec4 u_off;
uniform float u_s;
void main() { gl_Position = vec4(a_pos.xy * u_s, 0.0, 1.0) + u_off; }
