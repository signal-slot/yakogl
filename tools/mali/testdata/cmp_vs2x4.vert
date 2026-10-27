/* Compaction control: two vec4 varyings, for the fragment shaders that read part of each. */
attribute vec4 a_pos; attribute vec4 a_color; attribute vec4 a_uv; varying vec4 v_a; varying vec4 v_b;
void main() { gl_Position = a_pos; v_a = a_color; v_b = a_uv; }
