#version 100
precision highp float;
precision highp int;

struct buf
{
    vec2 invSize;
};

uniform buf ubuf;

uniform highp sampler2D depthTexture;
uniform highp sampler2D currentTexture;
uniform highp sampler2D velocityTexture;
uniform highp sampler2D prevTexture;

varying vec2 uv_coord;

vec2 getDepthClosestUVOffset(vec2 uv)
{
    float closestDepth = 100.0;
    vec2 depthClosestUVOffset = vec2(0.0);
    for (int x = -1; x <= 1; x++)
    {
        for (int y = -1; y <= 1; y++)
        {
            vec2 uvOffset = vec2(float(x), float(y)) * ubuf.invSize;
            float neighborDepth = texture2D(depthTexture, uv + uvOffset).x;
            if (neighborDepth < closestDepth)
            {
                depthClosestUVOffset = uvOffset;
                closestDepth = neighborDepth;
            }
        }
    }
    return depthClosestUVOffset;
}

void main()
{
    vec2 uv = uv_coord;
    vec4 currentColor = texture2D(currentTexture, uv);
    vec2 param = uv;
    vec2 depthClosestUVOffset = getDepthClosestUVOffset(param);
    vec2 velocity = texture2D(velocityTexture, uv + depthClosestUVOffset).zw;
    vec4 prevColor = texture2D(prevTexture, uv + velocity);
    vec4 minColor = vec4(9999.0);
    vec4 maxColor = vec4(-9999.0);
    for (int x = -1; x <= 1; x++)
    {
        for (int y = -1; y <= 1; y++)
        {
            vec2 uvoffset = vec2(float(x), float(y)) * ubuf.invSize;
            vec4 color = texture2D(currentTexture, uv + uvoffset);
            minColor = min(minColor, color);
            maxColor = max(maxColor, color);
        }
    }
    vec4 clampPrevColor = clamp(prevColor, minColor, maxColor);
    gl_FragData[0] = mix(currentColor, clampPrevColor, vec4(0.89999997615814208984375));
}

