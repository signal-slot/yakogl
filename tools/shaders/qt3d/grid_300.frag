#version 300 es
precision highp float;
precision highp int;

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

in vec3 v_nearPoint;
in vec3 v_farPoint;
layout(location = 0) out vec4 fragColor;
flat in uint v_viewIndex;

vec4 grid(vec2 fragPos2D, float scale, bool drawAxis, float lineAlpha)
{
    vec2 coord = fragPos2D * scale;
    vec2 derivative = fwidth(coord);
    vec2 grid_1 = abs(fract(coord - vec2(0.5)) - vec2(0.5)) / derivative;
    float line = min(grid_1.x, grid_1.y);
    float alpha = smoothstep(0.0, 1.0, 1.0 - line) * lineAlpha;
    vec4 color = vec4(0.5, 0.5, 0.5, alpha);
    float minimumx = min(derivative.x, 1.0);
    float minimumz = min(derivative.y, 1.0);
    if (drawAxis)
    {
        vec2 axisDistance = abs(coord / derivative);
        if (axisDistance.y < 1.0)
        {
            color = vec4(1.0, 0.0, 0.0, alpha);
        }
        if (axisDistance.x < 1.0)
        {
            color = vec4(0.0, 0.0, 1.0, alpha);
        }
    }
    float d2 = length(derivative);
    if (d2 > 0.100000001490116119384765625)
    {
        color.w *= max(0.0, 1.0 - (1.5 * (d2 - 0.100000001490116119384765625)));
    }
    return color;
}

float computeLinearDepth(float fragDepth, float near, float far)
{
    if (near <= 0.0)
    {
        return fragDepth;
    }
    float linearDepth = ((2.0 * near) * far) / ((near + far) - (fragDepth * (far - near)));
    return linearDepth / far;
}

void main()
{
    float nearFloor = v_nearPoint.y;
    float farFloor = v_farPoint.y;
    float t = (-nearFloor) / (farFloor - nearFloor);
    vec3 fragPos3D = v_nearPoint + ((v_farPoint - v_nearPoint) * t);
    vec2 fragPos2D = fragPos3D.xz;
    vec4 clipSpacePos = u_buf.viewProj * vec4(fragPos3D, 1.0);
    bool drawAxis = (u_buf.gridFlags & 1u) != 0u;
    vec2 param = fragPos2D;
    float param_1 = 10.0 * u_buf.scale;
    bool param_2 = drawAxis;
    float param_3 = 0.5;
    vec2 param_4 = fragPos2D;
    float param_5 = 1.0 * u_buf.scale;
    bool param_6 = drawAxis;
    float param_7 = 1.0;
    vec4 outColor = max(grid(param, param_1, param_2, param_3), grid(param_4, param_5, param_6, param_7));
    float fragDepth = clipSpacePos.z / clipSpacePos.w;
    float param_8 = fragDepth;
    float param_9 = u_buf.near;
    float param_10 = u_buf.far;
    float linearDepth = computeLinearDepth(param_8, param_9, param_10);
    float fading = float((fragDepth > (-1.0)) && (fragDepth < 1.0)) * (1.0 - max((linearDepth - 0.699999988079071044921875) / 0.300000011920928955078125, 0.0));
    fragColor = vec4(outColor.xyz, outColor.w * fading);
    gl_FragDepth = (0.5 * fragDepth) + 0.5;
}

