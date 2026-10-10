#version 300 es
precision highp float;
uniform highp isampler2D u_tex;
layout(location = 0) out vec4 fragColor;
void main()
{
    // a signed integer texel fetched as integers, sign extended, shown
    // offset by 128 (gl_test case 201, usamp): 2x2 texels over the 64x64 target
    fragColor = vec4(texelFetch(u_tex, ivec2(gl_FragCoord.xy) / 32, 0) + 128) / 255.0;
}
