#version 100
/* A pass-through for es100/dynidx*.frag (dEQP shaders.indexing's vertex
 * side for the fragment cases). */
attribute vec4 a_pos;
attribute vec4 a_coords;
varying mediump vec4 v_coords;
void main() {
    gl_Position = a_pos;
    v_coords = a_coords;
}
