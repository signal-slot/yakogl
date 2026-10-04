#version 300 es

struct buf
{
    mat4 viewProj;
    mat4 invViewProj;
    float near;
    float far;
    float scale;
    float yFactor;
    uint gridFlags;
};

uniform buf u_buf;

layout(location = 0) in vec3 attr_pos;
out vec3 v_nearPoint;
out vec3 v_farPoint;
flat out uint v_viewIndex;

vec3 unprojectPoint(float x, float y, float z, mat4 invViewProj)
{
    vec4 unprojectedPoint = invViewProj * vec4(x, y, z, 1.0);
    return unprojectedPoint.xyz / vec3(unprojectedPoint.w);
}

void main()
{
    vec3 p = attr_pos;
    float param = p.x;
    float param_1 = u_buf.yFactor * p.y;
    float param_2 = 0.0;
    mat4 param_3 = u_buf.invViewProj;
    v_nearPoint = unprojectPoint(param, param_1, param_2, param_3);
    float param_4 = p.x;
    float param_5 = u_buf.yFactor * p.y;
    float param_6 = 1.0;
    mat4 param_7 = u_buf.invViewProj;
    v_farPoint = unprojectPoint(param_4, param_5, param_6, param_7);
    v_viewIndex = 0u;
    gl_Position = vec4(p, 1.0);
}

