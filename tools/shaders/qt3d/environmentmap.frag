#version 100
precision highp float;
precision highp int;

struct buf
{
    int colorSpace;
};

uniform buf ubuf2;

uniform highp sampler2D environmentMap;

varying vec3 localPos;

vec2 sampleSphericalMap(vec3 v)
{
    vec2 uv = vec2(atan(v.z, v.x), asin(v.y));
    uv *= vec2(0.159099996089935302734375, 0.3183000087738037109375);
    uv += vec2(0.5);
    return uv;
}

void main()
{
    vec3 param = normalize(localPos);
    vec2 uv = sampleSphericalMap(param);
    vec4 textureColor = texture2D(environmentMap, uv);
    if (ubuf2.colorSpace == 1)
    {
        vec4 _72 = textureColor;
        vec4 _74 = textureColor;
        vec4 _76 = textureColor;
        vec3 _87 = _72.xyz * ((_74.xyz * ((_76.xyz * 0.305306017398834228515625) + vec3(0.6821711063385009765625))) + vec3(0.01252287812530994415283203125));
        textureColor.x = _87.x;
        textureColor.y = _87.y;
        textureColor.z = _87.z;
    }
    gl_FragData[0] = textureColor;
}

