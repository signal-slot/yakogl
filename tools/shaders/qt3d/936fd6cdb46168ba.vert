#version 300 es

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

uniform cbMain _69;

uniform highp sampler2D glowBuffer1;
uniform highp sampler2D glowBuffer2;
uniform highp sampler2D glowBuffer3;
uniform highp sampler2D glowBuffer4;
uniform highp sampler2D glowBuffer5;
uniform highp sampler2D glowBuffer6;
uniform highp sampler2D glowBuffer7;
uniform highp sampler2D lensColorTexture;
uniform highp sampler2D lensDirtTexture;
uniform highp sampler2D lensFlareDownsampleBuffer;
uniform highp sampler2D lensFlareFeaturesBuffer;
uniform highp sampler2D lensFlareTexture;
uniform highp sampler2D lut;
uniform highp sampler2D qt_inputTexture;
uniform highp sampler2D starburstTexture;

out vec2 qt_inputUV;
layout(location = 1) in vec2 attr_uv;
out vec2 qt_textureUV;
layout(location = 0) in vec3 attr_pos;
flat out uint qt_viewIndex;

vec2 qt_effectTextureMapUV(vec2 uv)
{
    return uv;
}

void qt_customMain(vec3 VERTEX)
{
}

void main()
{
    qt_inputUV = attr_uv;
    vec2 param = attr_uv;
    qt_textureUV = qt_effectTextureMapUV(param);
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    qt_viewIndex = 0u;
    vec3 param_1 = qt_vertPosition.xyz;
    qt_customMain(param_1);
    qt_vertPosition.x = param_1.x;
    qt_vertPosition.y = param_1.y;
    qt_vertPosition.z = param_1.z;
    gl_Position = _69.qt_modelViewProjection * qt_vertPosition;
}

