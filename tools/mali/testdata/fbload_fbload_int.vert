#version 300 es
// The ES 3.00 twin of internal/es/quad.vert (a #version 300 es fragment
// shader does not link with an ES 1.00 vertex shader): the back ends'
// full-screen quad, the pair of fbload_*.frag.
layout(location = 0) in vec4 a_pos;
void main() {
    gl_Position = a_pos;
}
