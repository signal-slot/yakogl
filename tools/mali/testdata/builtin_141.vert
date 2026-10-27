#version 300 es
precision highp float;
layout(location = 0) in vec4 pos;
layout(location = 1) in vec2 offs;
layout(location = 2) in vec4 icol;
out vec4 v_col;
void main()
{
    gl_Position = vec4(pos.xy * 0.25 + offs, 0.0, 1.0);
    /* the last instance proves gl_InstanceID: half its colour */
    v_col = gl_InstanceID == 3 ? icol * 0.5 : icol;
}
