#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    float shadowScale;
    vec2 shadowOffset;
    vec2 centerOffset;
    float contrast;
    float brightness;
    float saturation;
    vec4 colorizationColor;
    vec4 blurWeight1;
    vec2 blurWeight2;
    vec4 mask;
    float maskInverted;
    vec4 shadowColor;
    vec4 shadowBlurWeight1;
    vec2 shadowBlurWeight2;
};

uniform buf _23;

uniform highp sampler2D src;
uniform highp sampler2D blurSrc1;
uniform highp sampler2D blurSrc2;
uniform highp sampler2D blurSrc3;
uniform highp sampler2D blurSrc4;

varying vec2 texCoord;

void main()
{
    vec4 color = texture2D(src, texCoord) * _23.blurWeight1.x;
    color += (texture2D(blurSrc1, texCoord) * _23.blurWeight1.y);
    color += (texture2D(blurSrc2, texCoord) * _23.blurWeight1.z);
    color += (texture2D(blurSrc3, texCoord) * _23.blurWeight1.w);
    color += (texture2D(blurSrc4, texCoord) * _23.blurWeight2.x);
    vec4 _73 = color;
    float _78 = color.w;
    float _89 = color.w;
    vec3 _92 = ((_73.xyz - vec3(0.5 * _78)) * (1.0 + _23.contrast)) + vec3(0.5 * _89);
    color.x = _92.x;
    color.y = _92.y;
    color.z = _92.z;
    float _103 = color.w;
    vec4 _105 = color;
    vec3 _108 = _105.xyz + vec3(_23.brightness * _103);
    color.x = _108.x;
    color.y = _108.y;
    color.z = _108.z;
    float gray = dot(color.xyz, vec3(0.2989999949932098388671875, 0.58700001239776611328125, 0.114000000059604644775390625));
    vec4 _133 = color;
    vec3 _139 = ((_23.colorizationColor.xyz * gray) * _23.colorizationColor.w) + (_133.xyz * (1.0 - _23.colorizationColor.w));
    color.x = _139.x;
    color.y = _139.y;
    color.z = _139.z;
    vec4 _148 = color;
    vec3 _155 = mix(vec3(gray), _148.xyz, vec3(1.0 + _23.saturation));
    color.x = _155.x;
    color.y = _155.y;
    color.z = _155.z;
    gl_FragData[0] = color * _23.qt_Opacity;
}

