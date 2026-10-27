/* Compaction control: a vec4 varying, for the fragment shaders that read part of it. */
attribute vec4 a_pos; attribute vec4 a_color; varying vec4 v_color;
void main() { gl_Position = a_pos; v_color = a_color; }
