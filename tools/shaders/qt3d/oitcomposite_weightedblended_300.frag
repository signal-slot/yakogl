#version 300 es
precision highp float;
precision highp int;

uniform highp sampler2D accumTexture;
uniform highp sampler2D revealageTexture;

in vec2 uv_coord;
layout(location = 0) out vec4 fragOutput;

void main()
{
    vec2 uv = uv_coord;
    vec4 accum = texture(accumTexture, uv);
    float a = 1.0 - texture(revealageTexture, uv).x;
    vec4 color = vec4(accum.xyz / vec3(clamp(accum.w, 9.9999997473787516355514526367188e-05, 5000000.0)), a);
    fragOutput = color;
}

