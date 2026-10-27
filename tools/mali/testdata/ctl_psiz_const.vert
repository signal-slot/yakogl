/* Control: a constant gl_PointSize (folded to its fp16 value, the position program and its no_psiz copy). */
attribute vec4 a_pos;
void main() { gl_PointSize = 4.0; gl_Position = a_pos; }
