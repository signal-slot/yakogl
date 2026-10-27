/* A multiply by a constant: Mesa's algebraic rules on it (mneg). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.xy * -1.0, 0.0, 1.0); }
