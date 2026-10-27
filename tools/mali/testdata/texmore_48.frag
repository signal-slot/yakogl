#version 300 es
precision highp float;
precision highp int;
uniform highp sampler3D vol;
uniform float zz;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    vec3 p = vec3(v_xy * 0.5 + 0.5, zz);
    ivec3 sz = textureSize(vol, 0);
    float a = textureLod(vol, p, 0.0).r;
    vec4 f = texelFetch(vol, ivec3(0, 0, int(zz * float(sz.z))), 0);
    fragColor = vec4(texture(vol, p).r, a, f.b, float(sz.z) / 255.0);
}
