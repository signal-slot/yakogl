/* A multiply by a constant: Mesa's algebraic rules on it (m075). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.xy * 0.75, 0.0, 1.0); }
