#version 300 es
// tools/shaders/es300/mrt3.vert's full-screen position, for mrt4.frag.
layout(location = 0) in vec4 a_pos;
void main()
{
    gl_Position = a_pos;
}
