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

uniform buf _36;

uniform highp sampler2D _qt_texture;

varying vec3 sampleNearLeft;
varying vec3 sampleNearRight;

void main()
{
    vec2 n;
    n.x = texture2DProj(_qt_texture, sampleNearLeft).x;
    n.y = texture2DProj(_qt_texture, sampleNearRight).x;
    n = smoothstep(vec2(_36.alphaMin), vec2(_36.alphaMax), n);
    float c = 0.5 * (n.x + n.y);
    gl_FragData[0] = vec4(n.x, c, n.y, c) * _36.color.w;
}

