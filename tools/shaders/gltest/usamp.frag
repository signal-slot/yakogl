#version 300 es
precision highp float;
uniform highp usampler2D u_tex;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    // an unsigned integer texel read as integers, shown as bytes (gl_test case 201, usamp)
    fragColor = vec4(texture(u_tex, v_xy * 0.5 + 0.5)) / 255.0;
}
