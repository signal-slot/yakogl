#version 450
/* The clear quad: the backend writes the four corners of the clear rectangle
 * into a vertex buffer in clip space, with z already at the clear depth. */
layout(location = 0) in vec4 a_pos;
void main() {
    gl_Position = a_pos;
}
