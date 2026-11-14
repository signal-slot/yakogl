#version 100
/* A position-only vertex shader for the fragment shaders that read no varying. */
attribute vec4 a_pos;
void main() {
    gl_Position = a_pos;
}
