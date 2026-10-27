#version 300 es
precision highp float;
uniform sampler2D hdr;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    // a quarter of the HDR texel: values above 1.0 stay visible in an 8-bit target
    fragColor = texture(hdr, v_xy * 0.5 + 0.5) * 0.25;
}
