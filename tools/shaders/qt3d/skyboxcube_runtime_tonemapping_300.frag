#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 skyboxProperties;
    mat3 orientation;
    mat4 viewProjection;
    mat4 inverseProjection;
    mat3 viewMatrix;
};

uniform buf ubuf;

uniform highp samplerCube tex;

in vec3 tex_coords;
layout(location = 0) out vec4 fragOutput;

vec3 qt_sRGBToLinear(vec3 srgbIn)
{
    return srgbIn * ((srgbIn * ((srgbIn * 0.305306017398834228515625) + vec3(0.6821711063385009765625))) + vec3(0.01252287812530994415283203125));
}

vec3 qt_linearTosRGB(vec3 color)
{
    vec3 S1 = sqrt(color);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.585122406482696533203125) + (S2 * 0.783140361309051513671875)) - (S3 * 0.3682627379894256591796875);
}

vec3 qt_toneMapACES(vec3 color)
{
    vec3 param = clamp((color * ((color * 2.5099999904632568359375) + vec3(0.02999999932944774627685546875))) / ((color * ((color * 2.4300000667572021484375) + vec3(0.589999973773956298828125))) + vec3(0.14000000059604644775390625)), vec3(0.0), vec3(1.0));
    return qt_linearTosRGB(param);
}

vec3 qt_tonemapHejlDawson(inout vec3 color)
{
    color = max(vec3(0.0), color - vec3(0.0040000001899898052215576171875));
    return (color * ((color * 6.19999980926513671875) + vec3(0.5))) / ((color * ((color * 6.19999980926513671875) + vec3(1.7000000476837158203125))) + vec3(0.0599999986588954925537109375));
}

vec3 qt_toneMapFilmicSub(vec3 color)
{
    return (((color * ((color * 0.1500000059604644775390625) + vec3(0.0500000007450580596923828125))) + vec3(0.0040000001899898052215576171875)) / ((color * ((color * 0.1500000059604644775390625) + vec3(0.5))) + vec3(0.0599999986588954925537109375))) - vec3(0.066666670143604278564453125);
}

vec3 qt_toneMapFilmic(inout vec3 color)
{
    vec3 param = color * 2.0;
    color = qt_toneMapFilmicSub(param);
    vec3 param_1 = vec3(11.19999980926513671875);
    vec3 whiteScale = vec3(1.0) / qt_toneMapFilmicSub(param_1);
    vec3 param_2 = color * whiteScale;
    return qt_linearTosRGB(param_2);
}

vec3 qt_tonemap(vec3 color, bool sRgbsource, uint tonemap_mode)
{
    if (sRgbsource && (tonemap_mode == 1u))
    {
        return color;
    }
    vec3 _193;
    if (sRgbsource)
    {
        vec3 param = color;
        _193 = qt_sRGBToLinear(param);
    }
    else
    {
        _193 = color;
    }
    vec3 linear_color = _193;
    switch (tonemap_mode)
    {
        case 1u:
        {
            vec3 param_1 = linear_color;
            return qt_linearTosRGB(param_1);
        }
        case 2u:
        {
            vec3 param_2 = linear_color;
            return qt_toneMapACES(param_2);
        }
        case 3u:
        {
            vec3 param_3 = linear_color;
            vec3 _219 = qt_tonemapHejlDawson(param_3);
            return _219;
        }
        case 4u:
        {
            vec3 param_4 = linear_color;
            vec3 _223 = qt_toneMapFilmic(param_4);
            return _223;
        }
        default:
        {
            return linear_color;
        }
    }
}

vec4 qt_tonemap(vec4 color, bool sRgbsource, uint tonemap_mode)
{
    vec3 param = color.xyz;
    bool param_1 = sRgbsource;
    uint param_2 = tonemap_mode;
    return vec4(qt_tonemap(param, param_1, param_2), color.w);
}

void main()
{
    vec4 color = texture(tex, tex_coords);
    uint tonemapMode = uint(int(ubuf.skyboxProperties.w));
    bool sRrgb = ubuf.skyboxProperties.z != 0.0;
    vec4 param = color;
    bool param_1 = sRrgb;
    uint param_2 = tonemapMode;
    fragOutput = qt_tonemap(param, param_1, param_2);
}

