#version 300 es
precision highp float;
precision highp int;

struct buf
{
    int hemisphere;
};

uniform buf ubuf;

uniform highp samplerCube shadowMapCube;

in vec2 uv_coord;
layout(location = 0) out vec4 fragOutput;

void main()
{
    vec2 uv = (uv_coord * 2.0) - vec2(1.0);
    float r2 = dot(uv, uv);
    float z = 0.0;
    if (ubuf.hemisphere == 0)
    {
        z = (1.0 - r2) / (1.0 + r2);
    }
    else
    {
        z = (r2 - 1.0) / (1.0 + r2);
    }
    float x = (2.0 * uv.x) / (1.0 + r2);
    float y = (2.0 * uv.y) / (1.0 + r2);
    float depthVal = texture(shadowMapCube, vec3(x, y, z)).x;
    fragOutput = vec4(depthVal);
}

