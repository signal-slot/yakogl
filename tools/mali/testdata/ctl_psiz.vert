/* Control: gl_PointSize from a uniform (the position program and its no_psiz copy). */
attribute vec4 a_pos;
uniform float u_size;
void main() { gl_PointSize = u_size; gl_Position = a_pos; }
