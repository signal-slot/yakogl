#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 aoProperties;
    vec4 aoProperties2;
    vec4 aoScreenConst;
    vec4 uvToEyeConst;
    vec2 cameraProperties;
};

uniform buf ubuf;

uniform highp sampler2D depthTexture;

layout(location = 0) out vec4 fragOutput;

float getDepthValue(vec4 depth_texture_sample, vec2 cameraProperties)
{
    float zNear = cameraProperties.x;
    float zFar = cameraProperties.y;
    float zRange = zFar - zNear;
    float z_b = depth_texture_sample.x;
    float z_n = (2.0 * z_b) - 1.0;
    float z_e = ((2.0 * zNear) * zFar) / ((zFar + zNear) - (z_n * zRange));
    return 1.0 - ((z_e - cameraProperties.x) / zRange);
}

float depthValueToLinearDistance(float depth_value, vec2 cameraProperties)
{
    float FarClipDistance = cameraProperties.y;
    float NearClipDistance = cameraProperties.x;
    float DepthRange = FarClipDistance - NearClipDistance;
    float linearDepth = NearClipDistance + (DepthRange * (1.0 - depth_value));
    return linearDepth;
}

vec3 getViewSpacePos(highp sampler2D depthSampler, vec2 camProps, vec2 UV, vec4 UvToEye)
{
    vec4 param = texture(depthSampler, UV);
    vec2 param_1 = camProps;
    float sampleDepth = getDepthValue(param, param_1);
    float param_2 = sampleDepth;
    vec2 param_3 = camProps;
    sampleDepth = depthValueToLinearDistance(param_2, param_3);
    vec2 scaledUV = (UV * UvToEye.xy) + UvToEye.zw;
    return vec3(scaledUV * sampleDepth, sampleDepth);
}

float hashRot(vec2 pos)
{
    float px = 2.0 * fract(floor(pos.x) * 0.5);
    float py = fract(floor(pos.y) * 0.5);
    return px + py;
}

vec2 computeDir(vec2 baseDir, int v)
{
    vec2 param = gl_FragCoord.xy;
    float ang = (3.1415927410125732421875 * hashRot(param)) + float(v - 1);
    vec2 vX = vec2(cos(ang), sin(ang));
    vec2 vY = vec2(-sin(ang), cos(ang));
    return vec2(dot(baseDir, vX), dot(baseDir, vY));
}

vec2 offsetDir(vec2 baseDir, int v)
{
    float ang = float(v - 1);
    vec2 vX = vec2(cos(ang), sin(ang));
    vec2 vY = vec2(-sin(ang), cos(ang));
    return vec2(dot(baseDir, vX), dot(baseDir, vY));
}

float calculateAo(int j, vec3 kernel[9], vec4 aoParams, vec4 aoParams2, vec2 camProps, vec2 centerUV, vec4 aoScreen, vec4 UvToEye, vec3 viewPos, vec3 viewNorm, float radStep, highp sampler2D depthSampler)
{
    float ret = 0.0;
    int i = 0;
    float curRange = (aoParams.y * radStep) * float(j);
    float curRadius = curRange * kernel[i].z;
    vec2 param = kernel[i].xy;
    int param_1 = j;
    vec2 param_2 = kernel[i].xy;
    int param_3 = j;
    vec2 _295 = (computeDir(param, param_1) * aoParams2.y) + (offsetDir(param_2, param_3) * (1.0 - aoParams2.y));
    vec3 smpDir;
    smpDir.x = _295.x;
    smpDir.y = _295.y;
    smpDir.z = kernel[i].z;
    smpDir *= curRange;
    vec2 smpUV = centerUV + (smpDir.xy * aoScreen.zw);
    vec2 param_4 = camProps;
    vec2 param_5 = smpUV;
    vec4 param_6 = UvToEye;
    vec3 samplePos = getViewSpacePos(depthSampler, param_4, param_5, param_6);
    vec3 smpVec = samplePos - viewPos;
    float lenRad = dot(smpVec, smpVec);
    smpVec = normalize(smpVec);
    float lenDot = dot(smpVec, viewNorm);
    lenRad /= (aoParams.y * aoParams.y);
    float falloff = smoothstep(8.0, 0.0, (lenRad - 1.0) * 0.125);
    float occl = 1.0 - clamp(lenDot * falloff, 0.0, 1.0);
    ret += (occl * occl);
    int i_1 = 1;
    float curRange_1 = (aoParams.y * radStep) * float(j);
    float curRadius_1 = curRange_1 * kernel[i_1].z;
    vec2 param_7 = kernel[i_1].xy;
    int param_8 = j;
    vec2 param_9 = kernel[i_1].xy;
    int param_10 = j;
    vec2 _401 = (computeDir(param_7, param_8) * aoParams2.y) + (offsetDir(param_9, param_10) * (1.0 - aoParams2.y));
    vec3 smpDir_1;
    smpDir_1.x = _401.x;
    smpDir_1.y = _401.y;
    smpDir_1.z = kernel[i_1].z;
    smpDir_1 *= curRange_1;
    vec2 smpUV_1 = centerUV + (smpDir_1.xy * aoScreen.zw);
    vec2 param_11 = camProps;
    vec2 param_12 = smpUV_1;
    vec4 param_13 = UvToEye;
    vec3 samplePos_1 = getViewSpacePos(depthSampler, param_11, param_12, param_13);
    vec3 smpVec_1 = samplePos_1 - viewPos;
    float lenRad_1 = dot(smpVec_1, smpVec_1);
    smpVec_1 = normalize(smpVec_1);
    float lenDot_1 = dot(smpVec_1, viewNorm);
    lenRad_1 /= (aoParams.y * aoParams.y);
    float falloff_1 = smoothstep(8.0, 0.0, (lenRad_1 - 1.0) * 0.125);
    float occl_1 = 1.0 - clamp(lenDot_1 * falloff_1, 0.0, 1.0);
    ret += (occl_1 * occl_1);
    int i_2 = 2;
    float curRange_2 = (aoParams.y * radStep) * float(j);
    float curRadius_2 = curRange_2 * kernel[i_2].z;
    vec2 param_14 = kernel[i_2].xy;
    int param_15 = j;
    vec2 param_16 = kernel[i_2].xy;
    int param_17 = j;
    vec2 _506 = (computeDir(param_14, param_15) * aoParams2.y) + (offsetDir(param_16, param_17) * (1.0 - aoParams2.y));
    vec3 smpDir_2;
    smpDir_2.x = _506.x;
    smpDir_2.y = _506.y;
    smpDir_2.z = kernel[i_2].z;
    smpDir_2 *= curRange_2;
    vec2 smpUV_2 = centerUV + (smpDir_2.xy * aoScreen.zw);
    vec2 param_18 = camProps;
    vec2 param_19 = smpUV_2;
    vec4 param_20 = UvToEye;
    vec3 samplePos_2 = getViewSpacePos(depthSampler, param_18, param_19, param_20);
    vec3 smpVec_2 = samplePos_2 - viewPos;
    float lenRad_2 = dot(smpVec_2, smpVec_2);
    smpVec_2 = normalize(smpVec_2);
    float lenDot_2 = dot(smpVec_2, viewNorm);
    lenRad_2 /= (aoParams.y * aoParams.y);
    float falloff_2 = smoothstep(8.0, 0.0, (lenRad_2 - 1.0) * 0.125);
    float occl_2 = 1.0 - clamp(lenDot_2 * falloff_2, 0.0, 1.0);
    ret += (occl_2 * occl_2);
    int i_3 = 3;
    float curRange_3 = (aoParams.y * radStep) * float(j);
    float curRadius_3 = curRange_3 * kernel[i_3].z;
    vec2 param_21 = kernel[i_3].xy;
    int param_22 = j;
    vec2 param_23 = kernel[i_3].xy;
    int param_24 = j;
    vec2 _611 = (computeDir(param_21, param_22) * aoParams2.y) + (offsetDir(param_23, param_24) * (1.0 - aoParams2.y));
    vec3 smpDir_3;
    smpDir_3.x = _611.x;
    smpDir_3.y = _611.y;
    smpDir_3.z = kernel[i_3].z;
    smpDir_3 *= curRange_3;
    vec2 smpUV_3 = centerUV + (smpDir_3.xy * aoScreen.zw);
    vec2 param_25 = camProps;
    vec2 param_26 = smpUV_3;
    vec4 param_27 = UvToEye;
    vec3 samplePos_3 = getViewSpacePos(depthSampler, param_25, param_26, param_27);
    vec3 smpVec_3 = samplePos_3 - viewPos;
    float lenRad_3 = dot(smpVec_3, smpVec_3);
    smpVec_3 = normalize(smpVec_3);
    float lenDot_3 = dot(smpVec_3, viewNorm);
    lenRad_3 /= (aoParams.y * aoParams.y);
    float falloff_3 = smoothstep(8.0, 0.0, (lenRad_3 - 1.0) * 0.125);
    float occl_3 = 1.0 - clamp(lenDot_3 * falloff_3, 0.0, 1.0);
    ret += (occl_3 * occl_3);
    int i_4 = 4;
    float curRange_4 = (aoParams.y * radStep) * float(j);
    float curRadius_4 = curRange_4 * kernel[i_4].z;
    vec2 param_28 = kernel[i_4].xy;
    int param_29 = j;
    vec2 param_30 = kernel[i_4].xy;
    int param_31 = j;
    vec2 _716 = (computeDir(param_28, param_29) * aoParams2.y) + (offsetDir(param_30, param_31) * (1.0 - aoParams2.y));
    vec3 smpDir_4;
    smpDir_4.x = _716.x;
    smpDir_4.y = _716.y;
    smpDir_4.z = kernel[i_4].z;
    smpDir_4 *= curRange_4;
    vec2 smpUV_4 = centerUV + (smpDir_4.xy * aoScreen.zw);
    vec2 param_32 = camProps;
    vec2 param_33 = smpUV_4;
    vec4 param_34 = UvToEye;
    vec3 samplePos_4 = getViewSpacePos(depthSampler, param_32, param_33, param_34);
    vec3 smpVec_4 = samplePos_4 - viewPos;
    float lenRad_4 = dot(smpVec_4, smpVec_4);
    smpVec_4 = normalize(smpVec_4);
    float lenDot_4 = dot(smpVec_4, viewNorm);
    lenRad_4 /= (aoParams.y * aoParams.y);
    float falloff_4 = smoothstep(8.0, 0.0, (lenRad_4 - 1.0) * 0.125);
    float occl_4 = 1.0 - clamp(lenDot_4 * falloff_4, 0.0, 1.0);
    ret += (occl_4 * occl_4);
    int i_5 = 5;
    float curRange_5 = (aoParams.y * radStep) * float(j);
    float curRadius_5 = curRange_5 * kernel[i_5].z;
    vec2 param_35 = kernel[i_5].xy;
    int param_36 = j;
    vec2 param_37 = kernel[i_5].xy;
    int param_38 = j;
    vec2 _821 = (computeDir(param_35, param_36) * aoParams2.y) + (offsetDir(param_37, param_38) * (1.0 - aoParams2.y));
    vec3 smpDir_5;
    smpDir_5.x = _821.x;
    smpDir_5.y = _821.y;
    smpDir_5.z = kernel[i_5].z;
    smpDir_5 *= curRange_5;
    vec2 smpUV_5 = centerUV + (smpDir_5.xy * aoScreen.zw);
    vec2 param_39 = camProps;
    vec2 param_40 = smpUV_5;
    vec4 param_41 = UvToEye;
    vec3 samplePos_5 = getViewSpacePos(depthSampler, param_39, param_40, param_41);
    vec3 smpVec_5 = samplePos_5 - viewPos;
    float lenRad_5 = dot(smpVec_5, smpVec_5);
    smpVec_5 = normalize(smpVec_5);
    float lenDot_5 = dot(smpVec_5, viewNorm);
    lenRad_5 /= (aoParams.y * aoParams.y);
    float falloff_5 = smoothstep(8.0, 0.0, (lenRad_5 - 1.0) * 0.125);
    float occl_5 = 1.0 - clamp(lenDot_5 * falloff_5, 0.0, 1.0);
    ret += (occl_5 * occl_5);
    int i_6 = 6;
    float curRange_6 = (aoParams.y * radStep) * float(j);
    float curRadius_6 = curRange_6 * kernel[i_6].z;
    vec2 param_42 = kernel[i_6].xy;
    int param_43 = j;
    vec2 param_44 = kernel[i_6].xy;
    int param_45 = j;
    vec2 _926 = (computeDir(param_42, param_43) * aoParams2.y) + (offsetDir(param_44, param_45) * (1.0 - aoParams2.y));
    vec3 smpDir_6;
    smpDir_6.x = _926.x;
    smpDir_6.y = _926.y;
    smpDir_6.z = kernel[i_6].z;
    smpDir_6 *= curRange_6;
    vec2 smpUV_6 = centerUV + (smpDir_6.xy * aoScreen.zw);
    vec2 param_46 = camProps;
    vec2 param_47 = smpUV_6;
    vec4 param_48 = UvToEye;
    vec3 samplePos_6 = getViewSpacePos(depthSampler, param_46, param_47, param_48);
    vec3 smpVec_6 = samplePos_6 - viewPos;
    float lenRad_6 = dot(smpVec_6, smpVec_6);
    smpVec_6 = normalize(smpVec_6);
    float lenDot_6 = dot(smpVec_6, viewNorm);
    lenRad_6 /= (aoParams.y * aoParams.y);
    float falloff_6 = smoothstep(8.0, 0.0, (lenRad_6 - 1.0) * 0.125);
    float occl_6 = 1.0 - clamp(lenDot_6 * falloff_6, 0.0, 1.0);
    ret += (occl_6 * occl_6);
    int i_7 = 7;
    float curRange_7 = (aoParams.y * radStep) * float(j);
    float curRadius_7 = curRange_7 * kernel[i_7].z;
    vec2 param_49 = kernel[i_7].xy;
    int param_50 = j;
    vec2 param_51 = kernel[i_7].xy;
    int param_52 = j;
    vec2 _1031 = (computeDir(param_49, param_50) * aoParams2.y) + (offsetDir(param_51, param_52) * (1.0 - aoParams2.y));
    vec3 smpDir_7;
    smpDir_7.x = _1031.x;
    smpDir_7.y = _1031.y;
    smpDir_7.z = kernel[i_7].z;
    smpDir_7 *= curRange_7;
    vec2 smpUV_7 = centerUV + (smpDir_7.xy * aoScreen.zw);
    vec2 param_53 = camProps;
    vec2 param_54 = smpUV_7;
    vec4 param_55 = UvToEye;
    vec3 samplePos_7 = getViewSpacePos(depthSampler, param_53, param_54, param_55);
    vec3 smpVec_7 = samplePos_7 - viewPos;
    float lenRad_7 = dot(smpVec_7, smpVec_7);
    smpVec_7 = normalize(smpVec_7);
    float lenDot_7 = dot(smpVec_7, viewNorm);
    lenRad_7 /= (aoParams.y * aoParams.y);
    float falloff_7 = smoothstep(8.0, 0.0, (lenRad_7 - 1.0) * 0.125);
    float occl_7 = 1.0 - clamp(lenDot_7 * falloff_7, 0.0, 1.0);
    ret += (occl_7 * occl_7);
    int i_8 = 8;
    float curRange_8 = (aoParams.y * radStep) * float(j);
    float curRadius_8 = curRange_8 * kernel[i_8].z;
    vec2 param_56 = kernel[i_8].xy;
    int param_57 = j;
    vec2 param_58 = kernel[i_8].xy;
    int param_59 = j;
    vec2 _1136 = (computeDir(param_56, param_57) * aoParams2.y) + (offsetDir(param_58, param_59) * (1.0 - aoParams2.y));
    vec3 smpDir_8;
    smpDir_8.x = _1136.x;
    smpDir_8.y = _1136.y;
    smpDir_8.z = kernel[i_8].z;
    smpDir_8 *= curRange_8;
    vec2 smpUV_8 = centerUV + (smpDir_8.xy * aoScreen.zw);
    vec2 param_60 = camProps;
    vec2 param_61 = smpUV_8;
    vec4 param_62 = UvToEye;
    vec3 samplePos_8 = getViewSpacePos(depthSampler, param_60, param_61, param_62);
    vec3 smpVec_8 = samplePos_8 - viewPos;
    float lenRad_8 = dot(smpVec_8, smpVec_8);
    smpVec_8 = normalize(smpVec_8);
    float lenDot_8 = dot(smpVec_8, viewNorm);
    lenRad_8 /= (aoParams.y * aoParams.y);
    float falloff_8 = smoothstep(8.0, 0.0, (lenRad_8 - 1.0) * 0.125);
    float occl_8 = 1.0 - clamp(lenDot_8 * falloff_8, 0.0, 1.0);
    ret += (occl_8 * occl_8);
    return ret;
}

float SSambientOcclusion(highp sampler2D depthSampler, vec3 viewNorm, vec4 aoParams, vec4 aoParams2, vec2 camProps, vec4 aoScreen, vec4 UvToEye)
{
    vec2 centerUV = gl_FragCoord.xy * aoScreen.zw;
    vec2 param = camProps;
    vec2 param_1 = centerUV;
    vec4 param_2 = UvToEye;
    vec3 viewPos = getViewSpacePos(depthSampler, param, param_1, param_2);
    viewPos += (viewNorm * aoParams.w);
    float screenRadius = (aoParams.y * aoScreen.y) / viewPos.z;
    if (screenRadius < 1.0)
    {
        return 1.0;
    }
    vec3 kernel[9];
    kernel[0] = vec3(-0.1376475989818572998046875, 0.2842021882534027099609375, 0.948831975460052490234375);
    kernel[1] = vec3(-0.62661802768707275390625, 0.45941150188446044921875, 0.62951600551605224609375);
    kernel[2] = vec3(-0.890313804149627685546875, -0.058654241263866424560546875, 0.451554000377655029296875);
    kernel[3] = vec3(0.2871418893337249755859375, 0.8511679172515869140625, 0.4393889904022216796875);
    kernel[4] = vec3(-0.1525250971317291259765625, -0.387011706829071044921875, 0.90937197208404541015625);
    kernel[5] = vec3(0.6978704929351806640625, -0.217677295207977294921875, 0.682344019412994384765625);
    kernel[6] = vec3(0.7343006134033203125, 0.3774330914020538330078125, 0.564199984073638916015625);
    kernel[7] = vec3(0.1408804953098297119140625, -0.88915002346038818359375, 0.43529999256134033203125);
    kernel[8] = vec3(-0.6642615795135498046875, -0.543600976467132568359375, 0.513000011444091796875);
    float sampleRate = clamp(aoParams2.x, 2.0, 4.0);
    int radLevels = int(floor(sampleRate));
    float radStep = 1.0 / sampleRate;
    float ret = 0.0;
    int param_3 = 1;
    vec3 param_4[9] = kernel;
    vec4 param_5 = aoParams;
    vec4 param_6 = aoParams2;
    vec2 param_7 = camProps;
    vec2 param_8 = centerUV;
    vec4 param_9 = aoScreen;
    vec4 param_10 = UvToEye;
    vec3 param_11 = viewPos;
    vec3 param_12 = viewNorm;
    float param_13 = radStep;
    ret += calculateAo(param_3, param_4, param_5, param_6, param_7, param_8, param_9, param_10, param_11, param_12, param_13, depthSampler);
    int param_14 = 2;
    vec3 param_15[9] = kernel;
    vec4 param_16 = aoParams;
    vec4 param_17 = aoParams2;
    vec2 param_18 = camProps;
    vec2 param_19 = centerUV;
    vec4 param_20 = aoScreen;
    vec4 param_21 = UvToEye;
    vec3 param_22 = viewPos;
    vec3 param_23 = viewNorm;
    float param_24 = radStep;
    ret += calculateAo(param_14, param_15, param_16, param_17, param_18, param_19, param_20, param_21, param_22, param_23, param_24, depthSampler);
    if (radLevels >= 3)
    {
        int param_25 = 3;
        vec3 param_26[9] = kernel;
        vec4 param_27 = aoParams;
        vec4 param_28 = aoParams2;
        vec2 param_29 = camProps;
        vec2 param_30 = centerUV;
        vec4 param_31 = aoScreen;
        vec4 param_32 = UvToEye;
        vec3 param_33 = viewPos;
        vec3 param_34 = viewNorm;
        float param_35 = radStep;
        ret += calculateAo(param_25, param_26, param_27, param_28, param_29, param_30, param_31, param_32, param_33, param_34, param_35, depthSampler);
    }
    if (radLevels >= 4)
    {
        int param_36 = 4;
        vec3 param_37[9] = kernel;
        vec4 param_38 = aoParams;
        vec4 param_39 = aoParams2;
        vec2 param_40 = camProps;
        vec2 param_41 = centerUV;
        vec4 param_42 = aoScreen;
        vec4 param_43 = UvToEye;
        vec3 param_44 = viewPos;
        vec3 param_45 = viewNorm;
        float param_46 = radStep;
        ret += calculateAo(param_36, param_37, param_38, param_39, param_40, param_41, param_42, param_43, param_44, param_45, param_46, depthSampler);
    }
    ret /= (9.0 * float(radLevels));
    float hardCut = (1.0 - aoParams.z) * 0.72974002361297607421875;
    ret = smoothstep(0.0, 1.0, (ret - hardCut) / (1.0 - hardCut));
    ret = (aoParams.x * ret) + (1.0 - aoParams.x);
    return ret;
}

void main()
{
    ivec2 iCoords = ivec2(gl_FragCoord.xy);
    vec4 param = texelFetch(depthTexture, iCoords, 0);
    vec2 param_1 = ubuf.cameraProperties;
    float depth = getDepthValue(param, param_1);
    float param_2 = depth;
    vec2 param_3 = ubuf.cameraProperties;
    depth = depthValueToLinearDistance(param_2, param_3);
    depth = (depth - ubuf.cameraProperties.x) / (ubuf.cameraProperties.y - ubuf.cameraProperties.x);
    vec4 param_4 = texelFetch(depthTexture, iCoords + ivec2(1), 0);
    vec2 param_5 = ubuf.cameraProperties;
    float depth2 = getDepthValue(param_4, param_5);
    float param_6 = depth;
    vec2 param_7 = ubuf.cameraProperties;
    depth2 = depthValueToLinearDistance(param_6, param_7);
    vec4 param_8 = texelFetch(depthTexture, iCoords - ivec2(1), 0);
    vec2 param_9 = ubuf.cameraProperties;
    float depth3 = getDepthValue(param_8, param_9);
    float param_10 = depth;
    vec2 param_11 = ubuf.cameraProperties;
    depth3 = depthValueToLinearDistance(param_10, param_11);
    vec3 tanU = vec3(10.0, 0.0, dFdx(depth));
    vec3 tanV = vec3(0.0, 10.0, dFdy(depth));
    vec3 screenNorm = normalize(cross(tanU, tanV));
    tanU = vec3(10.0, 0.0, dFdx(depth2));
    tanV = vec3(0.0, 10.0, dFdy(depth2));
    screenNorm += normalize(cross(tanU, tanV));
    tanU = vec3(10.0, 0.0, dFdx(depth3));
    tanV = vec3(0.0, 10.0, dFdy(depth3));
    screenNorm += normalize(cross(tanU, tanV));
    screenNorm = -normalize(screenNorm);
    vec3 param_12 = screenNorm;
    vec4 param_13 = ubuf.aoProperties;
    vec4 param_14 = ubuf.aoProperties2;
    vec2 param_15 = ubuf.cameraProperties;
    vec4 param_16 = ubuf.aoScreenConst;
    vec4 param_17 = ubuf.uvToEyeConst;
    float aoFactor = SSambientOcclusion(depthTexture, param_12, param_13, param_14, param_15, param_16, param_17);
    fragOutput = vec4(aoFactor, aoFactor, aoFactor, 1.0);
}

