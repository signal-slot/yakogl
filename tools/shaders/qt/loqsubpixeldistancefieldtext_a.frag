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
    float fontScale;
    vec4 vecDelta;
};

uniform buf _37;

uniform highp sampler2D _qt_texture;

varying vec3 sampleNearLeft;
varying vec3 sampleNearRight;

void main()
{
    vec2 n;
    n.x = texture2DProj(_qt_texture, sampleNearLeft).w;
    n.y = texture2DProj(_qt_texture, sampleNearRight).w;
    n = smoothstep(vec2(_37.alphaMin), vec2(_37.alphaMax), n);
    float c = 0.5 * (n.x + n.y);
    gl_FragData[0] = vec4(n.x, c, n.y, c) * _37.color.w;
}

