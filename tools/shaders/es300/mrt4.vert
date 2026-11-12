#version 300 es
// gl_test 243 mrt4: a full-screen position, nothing else.
layout(location = 0) in vec4 a_pos;
void main()
{
    gl_Position = a_pos;
}
