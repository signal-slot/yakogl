/* A multiply by a constant stored as a varying (v075). */
attribute vec4 a_pos; attribute vec4 a_c; varying vec4 v;
void main() { gl_Position = a_pos; v = a_c * 0.75; }
