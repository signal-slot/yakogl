#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    vec2 textureScale;
    vec4 color;
    float alphaMin;
    float alphaMax;
    vec4 styleColor;
    float outlineAlphaMax0;
    float outlineAlphaMax1;
};

uniform buf _27;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;

void main()
{
    float d = texture2D(_qt_texture, sampleCoord).w;
    float a = smoothstep(_27.alphaMin, _27.alphaMax, d);
    gl_FragData[0] = (mix(_27.styleColor, _27.color, vec4(a)) * step(1.0 - a, 1.0)) * smoothstep(_27.outlineAlphaMax0, _27.outlineAlphaMax1, d);
}

