#version 300 es
layout(location = 0) in vec4 pos;
out vec4 v_col;
void main()
{
    // the vertex index names the colour: 0 red, 1 green, 2 blue, 3 white ...
    int i = gl_VertexID % 4;
    v_col = vec4(i == 0 || i == 3 ? 1.0 : 0.0, i == 1 || i == 3 ? 1.0 : 0.0, i == 2 || i == 3 ? 1.0 : 0.0, 1.0);
    gl_Position = pos;
}
