/* A multiply by a constant: Mesa's algebraic rules on it (m3w). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.xyz * 3.0, a_pos.w); }
