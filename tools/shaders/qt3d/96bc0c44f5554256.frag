#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    float adjustmentBrightness;
    float adjustmentContrast;
    float adjustmentSaturation;
    uint applyFXAA;
    uint colorAdjustmentsEnabled;
    uint ditheringEnabled;
    uint enableLut;
    float exposure;
    int glowBlendMode;
    float glowBloom;
    float glowHDRMaximumValue;
    float glowHDRMinimumValue;
    float glowHDRScale;
    float glowIntensity;
    int glowLevel;
    uint glowQualityHigh;
    float glowStrength;
    uint glowUseBicubicUpscale;
    uint isFirstPass;
    uint isGlowEnabled;
    uint lensFlareApplyDirtTexture;
    uint lensFlareApplyStarburstTexture;
    float lensFlareBloomBias;
    float lensFlareBloomScale;
    float lensFlareBlurAmount;
    vec3 lensFlareCameraDirection;
    uint lensFlareDebug;
    float lensFlareDistortion;
    uint lensFlareEnabled;
    int lensFlareGhostCount;
    float lensFlareGhostDispersal;
    float lensFlareHaloWidth;
    float lensFlareStretchToAspect;
    float lutFilterAlpha;
    float lutSize;
    vec2 qt_cameraProperties;
    float qt_fps;
    float qt_frame_num;
    vec2 qt_inputSize;
    mat4 qt_modelViewProjection;
    float qt_nearClipValue;
    float qt_normalAdjustViewportFactor;
    vec2 qt_outputSize;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    float sharpnessAmount;
    int tonemapMode;
    vec4 vignetteColor;
    uint vignetteEnabled;
    float vignetteRadius;
    float vignetteStrength;
    float white;
};

uniform cbMain _661;

uniform highp sampler2D qt_inputTexture;
uniform highp sampler2D lut;
uniform highp sampler2D lensDirtTexture;
uniform highp sampler2D lensFlareTexture;
uniform highp sampler2D starburstTexture;
uniform highp sampler2D glowBuffer1;
uniform highp sampler2D glowBuffer2;
uniform highp sampler2D glowBuffer3;
uniform highp sampler2D glowBuffer4;
uniform highp sampler2D glowBuffer5;
uniform highp sampler2D glowBuffer6;
uniform highp sampler2D glowBuffer7;
uniform highp sampler2D lensColorTexture;
uniform highp sampler2D lensFlareDownsampleBuffer;
uniform highp sampler2D lensFlareFeaturesBuffer;

in vec2 qt_inputUV;
layout(location = 0) out vec4 fragOutput;
in vec2 qt_textureUV;
flat in uint qt_viewIndex;

vec3 apply_fxaa(vec3 color, float exposure, vec2 pixelSize)
{
    vec3 rgbNW = textureLod(qt_inputTexture, qt_inputUV + (vec2(-1.0) * pixelSize), 0.0).xyz * exposure;
    vec3 rgbNE = textureLod(qt_inputTexture, qt_inputUV + (vec2(1.0, -1.0) * pixelSize), 0.0).xyz * exposure;
    vec3 rgbSW = textureLod(qt_inputTexture, qt_inputUV + (vec2(-1.0, 1.0) * pixelSize), 0.0).xyz * exposure;
    vec3 rgbSE = textureLod(qt_inputTexture, qt_inputUV + (vec2(1.0) * pixelSize), 0.0).xyz * exposure;
    vec3 rgbM = color;
    vec3 luma = vec3(0.29899999499320984, 0.5870000123977661, 0.11400000005960464);
    float lumaNW = dot(rgbNW, luma);
    float lumaNE = dot(rgbNE, luma);
    float lumaSW = dot(rgbSW, luma);
    float lumaSE = dot(rgbSE, luma);
    float lumaM = dot(rgbM, luma);
    float lumaMin = min(lumaM, min(min(lumaNW, lumaNE), min(lumaSW, lumaSE)));
    float lumaMax = max(lumaM, max(max(lumaNW, lumaNE), max(lumaSW, lumaSE)));
    vec2 dir;
    dir.x = -((lumaNW + lumaNE) - (lumaSW + lumaSE));
    dir.y = (lumaNW + lumaSW) - (lumaNE + lumaSE);
    float dirReduce = max((((lumaNW + lumaNE) + lumaSW) + lumaSE) * 0.03125, 0.0078125);
    float rcpDirMin = 1.0 / (min(abs(dir.x), abs(dir.y)) + dirReduce);
    dir = min(vec2(8.0), max(vec2(-8.0), dir * rcpDirMin)) * pixelSize;
    vec3 rgbA = (textureLod(qt_inputTexture, qt_inputUV + (dir * (-0.1666666716337204)), 0.0).xyz + textureLod(qt_inputTexture, qt_inputUV + (dir * 0.1666666716337204), 0.0).xyz) * (0.5 * exposure);
    vec3 rgbB = (rgbA * 0.5) + ((textureLod(qt_inputTexture, qt_inputUV + (dir * (-0.5)), 0.0).xyz + textureLod(qt_inputTexture, qt_inputUV + (dir * 0.5), 0.0).xyz) * (0.25 * exposure));
    float lumaB = dot(rgbB, luma);
    if ((lumaB < lumaMin) || (lumaB > lumaMax))
    {
        return rgbA;
    }
    else
    {
        return rgbB;
    }
}

vec3 apply_cas(vec3 color, float exposure, float sharpen_intensity)
{
    vec3 a = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(-1)).xyz * exposure;
    vec3 b = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(0, -1)).xyz * exposure;
    vec3 c = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(1, -1)).xyz * exposure;
    vec3 d = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(-1, 0)).xyz * exposure;
    vec3 e = color;
    vec3 f = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(1, 0)).xyz * exposure;
    vec3 g = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(-1, 1)).xyz * exposure;
    vec3 h = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(0, 1)).xyz * exposure;
    vec3 i = textureLodOffset(qt_inputTexture, qt_inputUV, 0.0, ivec2(1)).xyz * exposure;
    vec3 min_rgb = min(min(min(d, e), min(f, b)), h);
    vec3 min_rgb2 = min(min(min(min_rgb, a), min(g, c)), i);
    min_rgb += min_rgb2;
    vec3 max_rgb = max(max(max(d, e), max(f, b)), h);
    vec3 max_rgb2 = max(max(max(max_rgb, a), max(g, c)), i);
    max_rgb += max_rgb2;
    vec3 rcp_max_rgb = vec3(1.0) / max_rgb;
    vec3 amp_rgb = clamp(min(min_rgb, vec3(2.0) - max_rgb) * rcp_max_rgb, vec3(0.0), vec3(1.0));
    amp_rgb = inversesqrt(amp_rgb);
    float peak = 8.0 - (3.0 * sharpen_intensity);
    vec3 w_rgb = vec3(-1.0) / (amp_rgb * peak);
    vec3 rcp_weight_rgb = vec3(1.0) / (vec3(1.0) + (w_rgb * 4.0));
    vec3 window = ((b + d) + f) + h;
    return max(vec3(0.0), ((window * w_rgb) + e) * rcp_weight_rgb);
}

vec3 screen_space_dither(vec2 frag_coord)
{
    vec3 dither = vec3(dot(vec2(171.0, 231.0), frag_coord));
    dither = fract(dither / vec3(103.0, 71.0, 97.0));
    return (dither - vec3(0.5)) / vec3(255.0);
}

vec3 linear_to_srgb(vec3 color)
{
    vec3 _238 = (vec3(1.0549999475479126) * pow(color, vec3(0.4166666567325592))) - vec3(0.054999999701976776);
    vec3 _241 = color * 12.920000076293945;
    bvec3 _247 = lessThan(color, vec3(0.0031308000907301903));
    return vec3(_247.x ? _241.x : _238.x, _247.y ? _241.y : _238.y, _247.z ? _241.z : _238.z);
}

vec3 tonemap_aces(vec3 color, float white)
{
    vec3 color_tonemapped = (color * ((color * 1.8134750127792358) + vec3(0.025499999523162842))) / ((color * ((color * 1.755674958229065) + vec3(0.5015000104904175))) + vec3(0.14000000059604645));
    float white_tonemapped = (white * ((1.8134750127792358 * white) + 0.025499999523162842)) / ((white * ((1.755674958229065 * white) + 0.5015000104904175)) + 0.14000000059604645);
    return clamp(color_tonemapped / vec3(white_tonemapped), vec3(0.0), vec3(1.0));
}

vec3 tonemap_reinhard(vec3 color, float white)
{
    return clamp(((color * white) + color) / ((color * white) + vec3(white)), vec3(0.0), vec3(1.0));
}

vec3 tonemap_filmic(vec3 color, float white)
{
    vec3 color_tonemapped = (((color * ((color * 0.8799999952316284) + vec3(0.05999999865889549))) + vec3(0.0020000000949949026)) / ((color * ((color * 0.8799999952316284) + vec3(0.6000000238418579))) + vec3(0.05999999865889549))) - vec3(0.03333333507180214);
    float white_tonemapped = (((white * ((0.8799999952316284 * white) + 0.05999999865889549)) + 0.0020000000949949026) / ((white * ((0.8799999952316284 * white) + 0.6000000238418579)) + 0.05999999865889549)) - 0.03333333507180214;
    return clamp(color_tonemapped / vec3(white_tonemapped), vec3(0.0), vec3(1.0));
}

vec3 apply_tonemapping(inout vec3 color, float white)
{
    if (_661.tonemapMode == 0)
    {
    }
    else
    {
        if (_661.tonemapMode == 1)
        {
            color = clamp(color, vec3(0.0), vec3(1.0));
            vec3 param = color;
            color = linear_to_srgb(param);
        }
        else
        {
            if (_661.tonemapMode == 2)
            {
                color = max(vec3(0.0), color);
                vec3 param_1 = color;
                float param_2 = white;
                color = tonemap_aces(param_1, param_2);
                vec3 param_3 = color;
                color = linear_to_srgb(param_3);
            }
            else
            {
                if (_661.tonemapMode == 3)
                {
                    color = max(vec3(0.0), color);
                    vec3 param_4 = color;
                    float param_5 = white;
                    color = tonemap_reinhard(param_4, param_5);
                    vec3 param_6 = color;
                    color = linear_to_srgb(param_6);
                }
                else
                {
                    color = max(vec3(0.0), color);
                    vec3 param_7 = color;
                    float param_8 = white;
                    color = tonemap_filmic(param_7, param_8);
                    vec3 param_9 = color;
                    color = linear_to_srgb(param_9);
                }
            }
        }
    }
    return color;
}

float w0(float a)
{
    return 0.1666666716337204 * ((a * ((a * ((-a) + 3.0)) - 3.0)) + 1.0);
}

float w1(float a)
{
    return 0.1666666716337204 * (((a * a) * ((3.0 * a) - 6.0)) + 4.0);
}

float g0(float a)
{
    float param = a;
    float param_1 = a;
    return w0(param) + w1(param_1);
}

float w2(float a)
{
    return 0.1666666716337204 * ((a * ((a * (((-3.0) * a) + 3.0)) + 3.0)) + 1.0);
}

float w3(float a)
{
    return 0.1666666716337204 * ((a * a) * a);
}

float g1(float a)
{
    float param = a;
    float param_1 = a;
    return w2(param) + w3(param_1);
}

float h0(float a)
{
    float param = a;
    float param_1 = a;
    float param_2 = a;
    return (-1.0) + (w1(param) / (w0(param_1) + w1(param_2)));
}

float h1(float a)
{
    float param = a;
    float param_1 = a;
    float param_2 = a;
    return 1.0 + (w3(param) / (w2(param_1) + w3(param_2)));
}

vec3 sampleGlowBuffer(highp sampler2D glowBuffer, int level)
{
    if (_661.glowUseBicubicUpscale != 0u)
    {
        vec2 tex_size = vec2(ivec2(_661.qt_inputSize) >> ivec2(level));
        vec2 texel_size = vec2(1.0) / tex_size;
        vec2 uv = (qt_inputUV * tex_size) + vec2(0.5);
        vec2 iuv = floor(uv);
        vec2 fuv = fract(uv);
        float param = fuv.x;
        float g0x = g0(param);
        float param_1 = fuv.x;
        float g1x = g1(param_1);
        float param_2 = fuv.x;
        float h0x = h0(param_2);
        float param_3 = fuv.x;
        float h1x = h1(param_3);
        float param_4 = fuv.y;
        float h0y = h0(param_4);
        float param_5 = fuv.y;
        float h1y = h1(param_5);
        vec2 p0 = (vec2(iuv.x + h0x, iuv.y + h0y) - vec2(0.5)) * texel_size;
        vec2 p1 = (vec2(iuv.x + h1x, iuv.y + h0y) - vec2(0.5)) * texel_size;
        vec2 p2 = (vec2(iuv.x + h0x, iuv.y + h1y) - vec2(0.5)) * texel_size;
        vec2 p3 = (vec2(iuv.x + h1x, iuv.y + h1y) - vec2(0.5)) * texel_size;
        float param_6 = fuv.y;
        float param_7 = fuv.y;
        return ((((textureLod(glowBuffer, p0, 0.0) * g0x) + (textureLod(glowBuffer, p1, 0.0) * g1x)) * g0(param_6)) + (((textureLod(glowBuffer, p2, 0.0) * g0x) + (textureLod(glowBuffer, p3, 0.0) * g1x)) * g1(param_7))).xyz;
    }
    else
    {
        return textureLod(glowBuffer, qt_inputUV, 0.0).xyz;
    }
}

vec3 apply_bcs(inout vec3 color)
{
    color = mix(vec3(0.0), color, vec3(_661.adjustmentBrightness));
    color = mix(vec3(0.5), color, vec3(_661.adjustmentContrast));
    color = mix(vec3(dot(vec3(1.0), color) * 0.3333300054073334), color, vec3(_661.adjustmentSaturation));
    return color;
}

float get_interp_percent_channel(float channel_value, float floor_value, float diff_value)
{
    float div_sign = abs(sign(diff_value));
    return ((channel_value - floor_value) * div_sign) / (diff_value + (div_sign - 1.0));
}

vec3 get_interp_percent_color(vec3 color, vec3 floorc, vec3 diff)
{
    vec3 res = vec3(0.0);
    float param = color.x;
    float param_1 = floorc.x;
    float param_2 = diff.x;
    res.x = get_interp_percent_channel(param, param_1, param_2);
    float param_3 = color.y;
    float param_4 = floorc.y;
    float param_5 = diff.y;
    res.y = get_interp_percent_channel(param_3, param_4, param_5);
    float param_6 = color.z;
    float param_7 = floorc.z;
    float param_8 = diff.z;
    res.z = get_interp_percent_channel(param_6, param_7, param_8);
    return res;
}

float fixY(float y)
{
    if (_661.qt_normalAdjustViewportFactor > 0.0)
    {
        return y;
    }
    float lut_div = _661.lutSize - 1.0;
    return lut_div - y;
}

vec3 get_interpolated_color(vec3 floorc, vec3 diff, float perc)
{
    return floorc + (diff * perc);
}

vec3 get_lut_mapping_trilinear(vec3 old_color)
{
    float lut_div = _661.lutSize - 1.0;
    vec3 old_color_lut_base = old_color * lut_div;
    vec3 old_color_floor_vec = floor(old_color_lut_base);
    vec3 old_color_ceil_vec = ceil(old_color_lut_base);
    vec3 old_color_diff = (old_color_floor_vec - old_color_ceil_vec) / vec3(lut_div);
    vec3 param = old_color;
    vec3 param_1 = old_color_floor_vec / vec3(lut_div);
    vec3 param_2 = old_color_diff;
    vec3 old_color_percentages = get_interp_percent_color(param, param_1, param_2);
    vec3 lut_color_fff_vec = vec3(old_color_floor_vec.x, old_color_floor_vec.y, old_color_floor_vec.z);
    vec3 lut_color_ffc_vec = vec3(old_color_floor_vec.x, old_color_floor_vec.y, old_color_ceil_vec.z);
    vec3 lut_color_fcf_vec = vec3(old_color_floor_vec.x, old_color_ceil_vec.y, old_color_floor_vec.z);
    vec3 lut_color_fcc_vec = vec3(old_color_floor_vec.x, old_color_ceil_vec.y, old_color_ceil_vec.z);
    vec3 lut_color_cff_vec = vec3(old_color_ceil_vec.x, old_color_floor_vec.y, old_color_floor_vec.z);
    vec3 lut_color_cfc_vec = vec3(old_color_ceil_vec.x, old_color_floor_vec.y, old_color_ceil_vec.z);
    vec3 lut_color_ccf_vec = vec3(old_color_ceil_vec.x, old_color_ceil_vec.y, old_color_floor_vec.z);
    vec3 lut_color_ccc_vec = vec3(old_color_ceil_vec.x, old_color_ceil_vec.y, old_color_ceil_vec.z);
    float param_3 = lut_color_fff_vec.y;
    ivec2 lut_color_fff_pos = ivec2(int((_661.lutSize * lut_color_fff_vec.z) + lut_color_fff_vec.x), int(fixY(param_3)));
    float param_4 = lut_color_ffc_vec.y;
    ivec2 lut_color_ffc_pos = ivec2(int((_661.lutSize * lut_color_ffc_vec.z) + lut_color_ffc_vec.x), int(fixY(param_4)));
    float param_5 = lut_color_fcf_vec.y;
    ivec2 lut_color_fcf_pos = ivec2(int((_661.lutSize * lut_color_fcf_vec.z) + lut_color_fcf_vec.x), int(fixY(param_5)));
    float param_6 = lut_color_fcc_vec.y;
    ivec2 lut_color_fcc_pos = ivec2(int((_661.lutSize * lut_color_fcc_vec.z) + lut_color_fcc_vec.x), int(fixY(param_6)));
    float param_7 = lut_color_cff_vec.y;
    ivec2 lut_color_cff_pos = ivec2(int((_661.lutSize * lut_color_cff_vec.z) + lut_color_cff_vec.x), int(fixY(param_7)));
    float param_8 = lut_color_cfc_vec.y;
    ivec2 lut_color_cfc_pos = ivec2(int((_661.lutSize * lut_color_cfc_vec.z) + lut_color_cfc_vec.x), int(fixY(param_8)));
    float param_9 = lut_color_ccf_vec.y;
    ivec2 lut_color_ccf_pos = ivec2(int((_661.lutSize * lut_color_ccf_vec.z) + lut_color_ccf_vec.x), int(fixY(param_9)));
    float param_10 = lut_color_ccc_vec.y;
    ivec2 lut_color_ccc_pos = ivec2(int((_661.lutSize * lut_color_ccc_vec.z) + lut_color_ccc_vec.x), int(fixY(param_10)));
    vec3 lut_color_fff = texelFetch(lut, lut_color_fff_pos, 0).xyz;
    vec3 lut_color_ffc = texelFetch(lut, lut_color_ffc_pos, 0).xyz;
    vec3 lut_color_fcf = texelFetch(lut, lut_color_fcf_pos, 0).xyz;
    vec3 lut_color_fcc = texelFetch(lut, lut_color_fcc_pos, 0).xyz;
    vec3 lut_color_cff = texelFetch(lut, lut_color_cff_pos, 0).xyz;
    vec3 lut_color_cfc = texelFetch(lut, lut_color_cfc_pos, 0).xyz;
    vec3 lut_color_ccf = texelFetch(lut, lut_color_ccf_pos, 0).xyz;
    vec3 lut_color_ccc = texelFetch(lut, lut_color_ccc_pos, 0).xyz;
    vec3 param_11 = lut_color_fff;
    vec3 param_12 = lut_color_fff - lut_color_cff;
    float param_13 = old_color_percentages.x;
    vec3 lut_color_iff = get_interpolated_color(param_11, param_12, param_13);
    vec3 param_14 = lut_color_ffc;
    vec3 param_15 = lut_color_ffc - lut_color_cfc;
    float param_16 = old_color_percentages.x;
    vec3 lut_color_ifc = get_interpolated_color(param_14, param_15, param_16);
    vec3 param_17 = lut_color_fcf;
    vec3 param_18 = lut_color_fcf - lut_color_ccf;
    float param_19 = old_color_percentages.x;
    vec3 lut_color_icf = get_interpolated_color(param_17, param_18, param_19);
    vec3 param_20 = lut_color_fcc;
    vec3 param_21 = lut_color_fcc - lut_color_ccc;
    float param_22 = old_color_percentages.x;
    vec3 lut_color_icc = get_interpolated_color(param_20, param_21, param_22);
    vec3 param_23 = lut_color_iff;
    vec3 param_24 = lut_color_iff - lut_color_icf;
    float param_25 = old_color_percentages.y;
    vec3 lut_color_iif = get_interpolated_color(param_23, param_24, param_25);
    vec3 param_26 = lut_color_ifc;
    vec3 param_27 = lut_color_ifc - lut_color_icc;
    float param_28 = old_color_percentages.y;
    vec3 lut_color_iic = get_interpolated_color(param_26, param_27, param_28);
    vec3 param_29 = lut_color_iif;
    vec3 param_30 = lut_color_iif - lut_color_iic;
    float param_31 = old_color_percentages.z;
    vec3 lut_color_iii = get_interpolated_color(param_29, param_30, param_31);
    return lut_color_iii;
}

void qt_customMain()
{
    vec4 sourceColor = texture(qt_inputTexture, qt_inputUV);
    vec3 color = sourceColor.xyz;
    float fullExposure = _661.exposure;
    color *= fullExposure;
    if (_661.applyFXAA != 0u)
    {
        vec3 param = color;
        float param_1 = fullExposure;
        vec2 param_2 = vec2(1.0 / _661.qt_outputSize.x, 1.0 / _661.qt_outputSize.y);
        color = apply_fxaa(param, param_1, param_2);
    }
    if (_661.sharpnessAmount >= 0.0010000000474974513)
    {
        vec3 param_3 = color;
        float param_4 = fullExposure;
        float param_5 = _661.sharpnessAmount;
        color = apply_cas(param_3, param_4, param_5);
    }
    if (_661.ditheringEnabled != 0u)
    {
        vec2 param_6 = gl_FragCoord.xy;
        color += screen_space_dither(param_6);
    }
    if (_661.lensFlareEnabled != 0u)
    {
        vec3 lensMod = vec3(1.0);
        if (_661.lensFlareApplyDirtTexture != 0u)
        {
            lensMod = texture(lensDirtTexture, qt_inputUV).xyz;
        }
        vec3 lensFlare = texture(lensFlareTexture, qt_inputUV).xyz * lensMod;
        if (_661.lensFlareApplyStarburstTexture != 0u)
        {
            vec2 centerVec = qt_inputUV - vec2(0.5);
            float d = length(centerVec);
            float radial = acos(centerVec.x / d);
            float starOffset = dot(_661.lensFlareCameraDirection, vec3(1.0)) * 10.0;
            float star = texture(starburstTexture, vec2(radial + starOffset)).x * texture(starburstTexture, vec2(radial + (starOffset * 0.5))).x;
            star = clamp(star + (1.0 - smoothstep(0.0, 0.30000001192092896, d)), 0.0, 1.0);
            lensFlare *= star;
        }
        if (!(_661.lensFlareDebug != 0u))
        {
            color += lensFlare;
        }
        else
        {
            vec3 param_7 = lensFlare;
            float param_8 = _661.white;
            vec3 _1593 = apply_tonemapping(param_7, param_8);
            fragOutput = vec4(_1593, 1.0);
            return;
        }
    }
    vec3 param_9 = color;
    float param_10 = _661.white;
    vec3 _1604 = apply_tonemapping(param_9, param_10);
    color = _1604;
    if (_661.isGlowEnabled != 0u)
    {
        vec3 glow = vec3(0.0);
        if ((_661.glowLevel & 1) != 0)
        {
            int param_11 = 0;
            glow += sampleGlowBuffer(glowBuffer1, param_11);
        }
        if ((_661.glowLevel & 2) != 0)
        {
            int param_12 = 1;
            glow += sampleGlowBuffer(glowBuffer2, param_12);
        }
        if ((_661.glowLevel & 4) != 0)
        {
            int param_13 = 2;
            glow += sampleGlowBuffer(glowBuffer3, param_13);
        }
        if ((_661.glowLevel & 8) != 0)
        {
            int param_14 = 3;
            glow += sampleGlowBuffer(glowBuffer4, param_14);
        }
        if ((_661.glowLevel & 16) != 0)
        {
            int param_15 = 4;
            glow += sampleGlowBuffer(glowBuffer5, param_15);
        }
        if ((_661.glowLevel & 32) != 0)
        {
            int param_16 = 5;
            glow += sampleGlowBuffer(glowBuffer6, param_16);
        }
        if ((_661.glowLevel & 64) != 0)
        {
            int param_17 = 6;
            glow += sampleGlowBuffer(glowBuffer7, param_17);
        }
        glow *= _661.glowIntensity;
        vec3 param_18 = glow;
        float param_19 = _661.white;
        vec3 _1706 = apply_tonemapping(param_18, param_19);
        glow = _1706;
        if (_661.glowBlendMode == 0)
        {
            color += glow;
        }
        else
        {
            if (_661.glowBlendMode == 1)
            {
                color = clamp(color, vec3(0.0), vec3(1.0));
                color = max((color + glow) - (color * glow), vec3(0.0));
            }
            else
            {
                if (_661.glowBlendMode == 2)
                {
                    color = clamp(color, vec3(0.0), vec3(1.0));
                    glow = (glow * vec3(0.5)) + vec3(0.5);
                    float _1745;
                    if (glow.x <= 0.5)
                    {
                        _1745 = color.x - (((1.0 - (2.0 * glow.x)) * color.x) * (1.0 - color.x));
                    }
                    else
                    {
                        bool _1765 = glow.x > 0.5;
                        bool _1771;
                        if (_1765)
                        {
                            _1771 = color.x <= 0.25;
                        }
                        else
                        {
                            _1771 = _1765;
                        }
                        float _1772;
                        if (_1771)
                        {
                            _1772 = color.x + (((2.0 * glow.x) - 1.0) * ((((4.0 * color.x) * ((4.0 * color.x) + 1.0)) * (color.x - 1.0)) + (7.0 * color.x)));
                        }
                        else
                        {
                            _1772 = color.x + (((2.0 * glow.x) - 1.0) * (sqrt(color.x) - color.x));
                        }
                        _1745 = _1772;
                    }
                    color.x = _1745;
                    float _1821;
                    if (glow.y <= 0.5)
                    {
                        _1821 = color.y - (((1.0 - (2.0 * glow.y)) * color.y) * (1.0 - color.y));
                    }
                    else
                    {
                        bool _1841 = glow.y > 0.5;
                        bool _1847;
                        if (_1841)
                        {
                            _1847 = color.y <= 0.25;
                        }
                        else
                        {
                            _1847 = _1841;
                        }
                        float _1848;
                        if (_1847)
                        {
                            _1848 = color.y + (((2.0 * glow.y) - 1.0) * ((((4.0 * color.y) * ((4.0 * color.y) + 1.0)) * (color.y - 1.0)) + (7.0 * color.y)));
                        }
                        else
                        {
                            _1848 = color.y + (((2.0 * glow.y) - 1.0) * (sqrt(color.y) - color.y));
                        }
                        _1821 = _1848;
                    }
                    color.y = _1821;
                    float _1896;
                    if (glow.z <= 0.5)
                    {
                        _1896 = color.z - (((1.0 - (2.0 * glow.z)) * color.z) * (1.0 - color.z));
                    }
                    else
                    {
                        bool _1916 = glow.z > 0.5;
                        bool _1922;
                        if (_1916)
                        {
                            _1922 = color.z <= 0.25;
                        }
                        else
                        {
                            _1922 = _1916;
                        }
                        float _1923;
                        if (_1922)
                        {
                            _1923 = color.z + (((2.0 * glow.z) - 1.0) * ((((4.0 * color.z) * ((4.0 * color.z) + 1.0)) * (color.z - 1.0)) + (7.0 * color.z)));
                        }
                        else
                        {
                            _1923 = color.z + (((2.0 * glow.z) - 1.0) * (sqrt(color.z) - color.z));
                        }
                        _1896 = _1923;
                    }
                    color.z = _1896;
                }
                else
                {
                    color = glow;
                }
            }
        }
    }
    if (_661.colorAdjustmentsEnabled != 0u)
    {
        vec3 param_20 = color;
        vec3 _1977 = apply_bcs(param_20);
        color = _1977;
    }
    if (_661.enableLut != 0u)
    {
        vec3 param_21 = color;
        vec3 filteredColor = get_lut_mapping_trilinear(param_21);
        vec3 diffColor = filteredColor - color;
        vec3 param_22 = color;
        vec3 param_23 = diffColor;
        float param_24 = _661.lutFilterAlpha;
        color = get_interpolated_color(param_22, param_23, param_24);
    }
    if (_661.vignetteEnabled != 0u)
    {
        vec2 uv = qt_inputUV;
        vec3 bg = color;
        uv *= (vec2(1.0) - uv.yx);
        float vig = (uv.x * uv.y) * _661.vignetteStrength;
        vig = pow(vig, _661.vignetteRadius);
        vec4 vigColor = vec4(_661.vignetteColor.xyz, vig) * vig;
        color = mix(vigColor, vec4(color, 1.0), vec4(vig)).xyz;
    }
    fragOutput = vec4(color, sourceColor.w);
}

vec3 qt_tonemap(vec3 color)
{
    return color;
}

vec4 qt_tonemap(vec4 color)
{
    vec3 param = color.xyz;
    return vec4(qt_tonemap(param), color.w);
}

void main()
{
    qt_customMain();
    vec4 param = fragOutput;
    fragOutput = qt_tonemap(param);
}

