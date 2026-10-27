#version 300 es
// gl_test case 100 "idxtex": a texture and a dynamically indexed uniform
// array in one stage. The sampler's descriptor comes before the uniform
// block, so the indexed read's address must be past it (the online path
// read 20 dwords too low until 18ae1599). Red and green carry this stage.
precision highp float;
precision highp int;
layout(location = 0) in vec4 pos;
uniform highp sampler2D vtex;
uniform vec4 vcols[4];
uniform int vidx;
out vec4 v_col;
void main()
{
    v_col = 0.5 * (vcols[vidx] + textureLod(vtex, vec2(0.25, 0.25), 0.0));
    gl_Position = pos;
}
