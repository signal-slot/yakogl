#version 100

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

attribute vec4 vData;
attribute vec4 vPosRot;
varying vec2 fTex;
attribute vec3 vTex;
attribute vec4 vVec;
attribute vec4 vDeformVec;
varying vec4 fColor;
attribute vec4 vColor;
varying vec2 tt;
attribute float _qt_order;

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
        currentSize *= ubuf.sizetable[int(floor(t * 64.0))];
        fade *= ubuf.opacitytable[int(floor(t * 64.0))];
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
            tt.x = t;
        }
    }
    gl_Position.z = _qt_order * gl_Position.w;
}

