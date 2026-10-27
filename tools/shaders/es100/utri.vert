#version 100
/* The passthrough VS with a uniform offset on the position: the board sample
 * the removed v3d_triangle sample's uniform variant (TRI_UNIFORMS). A
 * uniform the vertex and coordinate shaders both read, from one storage.
 * Attribute order is the passthrough's (a_pos 0, a_color 1). */
attribute vec4 a_pos;
attribute vec4 a_color;
uniform vec4 u_offset;
varying vec4 v_color;
void main() {
    gl_Position = a_pos + u_offset;
    v_color = a_color;
}
