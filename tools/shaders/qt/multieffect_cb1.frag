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

varying vec2 texCoord;

void main()
{
    vec4 color = texture2D(src, texCoord) * _23.blurWeight1.x;
    color += (texture2D(blurSrc1, texCoord) * _23.blurWeight1.y);
    color += (texture2D(blurSrc2, texCoord) * _23.blurWeight1.z);
    color += (texture2D(blurSrc3, texCoord) * _23.blurWeight1.w);
    vec4 _63 = color;
    float _68 = color.w;
    float _79 = color.w;
    vec3 _82 = ((_63.xyz - vec3(0.5 * _68)) * (1.0 + _23.contrast)) + vec3(0.5 * _79);
    color.x = _82.x;
    color.y = _82.y;
    color.z = _82.z;
    float _93 = color.w;
    vec4 _95 = color;
    vec3 _98 = _95.xyz + vec3(_23.brightness * _93);
    color.x = _98.x;
    color.y = _98.y;
    color.z = _98.z;
    float gray = dot(color.xyz, vec3(0.2989999949932098388671875, 0.58700001239776611328125, 0.114000000059604644775390625));
    vec4 _123 = color;
    vec3 _129 = ((_23.colorizationColor.xyz * gray) * _23.colorizationColor.w) + (_123.xyz * (1.0 - _23.colorizationColor.w));
    color.x = _129.x;
    color.y = _129.y;
    color.z = _129.z;
    vec4 _138 = color;
    vec3 _145 = mix(vec3(gray), _138.xyz, vec3(1.0 + _23.saturation));
    color.x = _145.x;
    color.y = _145.y;
    color.z = _145.z;
    gl_FragData[0] = color * _23.qt_Opacity;
}

