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
attribute vec2 vPos;
attribute vec4 vVec;
varying vec4 fColor;
attribute vec4 vColor;
attribute float _qt_order;

void main()
{
    float t = (ubuf.timestamp - vData.x) / vData.y;
    mat4 matrix = ubuf.matrix;
    if ((t < 0.0) || (t > 1.0))
    {
        gl_PointSize = 0.0;
    }
    else
    {
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
            gl_PointSize = 0.0;
        }
        else
        {
            if (currentSize < 3.0)
            {
                currentSize = 3.0;
            }
            vec2 pos = (vPos + ((vVec.xy * t) * vData.y)) + ((vVec.zw * 0.5) * pow(t * vData.y, 2.0));
            gl_PointSize = currentSize * ubuf.dpr;
            gl_Position = matrix * vec4(pos.x, pos.y, 0.0, 1.0);
            fColor = vColor * fade;
        }
    }
    gl_Position.z = _qt_order * gl_Position.w;
}

