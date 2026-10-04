#version 300 es

struct buf
{
    mat4 matrix;
    float opacity;
    float entry;
    float timestamp;
    float dpr;
    float sizetable[64];
    float opacitytable[64];
};

uniform buf ubuf;

layout(location = 1) in vec4 vData;
layout(location = 0) in vec4 vPosRot;
out vec2 fTex;
layout(location = 5) in vec3 vTex;
layout(location = 2) in vec4 vVec;
layout(location = 4) in vec4 vDeformVec;
out vec4 fColor;
layout(location = 3) in vec4 vColor;

void main()
{
    float t = (ubuf.timestamp - vData.x) / vData.y;
    mat4 matrix = ubuf.matrix;
    if ((t < 0.0) || (t > 1.0))
    {
        gl_Position = matrix * vec4(vPosRot.x, vPosRot.y, 0.0, 1.0);
    }
    else
    {
        fTex = vTex.xy;
        float currentSize = mix(vData.z, vData.w, t * t);
        float fade = 1.0;
        float fadeIn = min(t * 10.0, 1.0);
        float fadeOut = 1.0 - clamp((t - 0.75) * 4.0, 0.0, 1.0);
        if (ubuf.entry == 1.0)
        {
            fade = (fade * fadeIn) * fadeOut;
        }
        else
        {
            if (ubuf.entry == 2.0)
            {
                currentSize = (currentSize * fadeIn) * fadeOut;
            }
        }
        if (currentSize <= 0.0)
        {
            gl_Position = matrix * vec4(vPosRot.x, vPosRot.y, 0.0, 1.0);
        }
        else
        {
            if (currentSize < 3.0)
            {
                currentSize = 3.0;
            }
            float rotation = vPosRot.z + ((vPosRot.w * t) * vData.y);
            if (vTex.z > 0.0)
            {
                vec2 curVel = ((vVec.zw * t) * vData.y) + vVec.xy;
                if (length(curVel) > 0.0)
                {
                    rotation += atan(curVel.y, curVel.x);
                }
            }
            vec2 trigCalcs = vec2(cos(rotation), sin(rotation));
            vec4 deform = (vDeformVec * currentSize) * (vTex.xxyy - vec4(0.5));
            vec4 rotatedDeform = deform.xxzz * trigCalcs.xyxy;
            rotatedDeform += ((deform.yyww * trigCalcs.yxyx) * vec4(-1.0, 1.0, -1.0, 1.0));
            vec2 pos = (((vPosRot.xy + rotatedDeform.xy) + rotatedDeform.zw) + ((vVec.xy * t) * vData.y)) + ((vVec.zw * 0.5) * pow(t * vData.y, 2.0));
            gl_Position = matrix * vec4(pos.x, pos.y, 0.0, 1.0);
            fColor = vColor * fade;
        }
    }
}

