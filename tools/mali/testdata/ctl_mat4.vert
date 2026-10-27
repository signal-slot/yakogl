/* Control: a mat4 times a vec4. */
attribute vec4 a_pos;
uniform mat4 u_mvp;
void main() { gl_Position = u_mvp * a_pos; }
