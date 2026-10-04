#version 300 es
precision highp float;
precision highp int;

struct DistributionSample
{
    float pdf;
    float cosTheta;
    float sinTheta;
    float phi;
};

struct buf
{
    float roughness;
    float resolution;
    float lodBias;
    int sampleCount;
    int distribution;
};

uniform buf ubuf2;

uniform highp samplerCube environmentMap;

in vec3 localPos;
layout(location = 0) out vec4 FragColor;

float radicalInverse_VdC(inout uint bits)
{
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 1431655765u) << 1u) | ((bits & 2863311530u) >> 1u);
    bits = ((bits & 858993459u) << 2u) | ((bits & 3435973836u) >> 2u);
    bits = ((bits & 252645135u) << 4u) | ((bits & 4042322160u) >> 4u);
    bits = ((bits & 16711935u) << 8u) | ((bits & 4278255360u) >> 8u);
    return float(bits) * 2.3283064365386962890625e-10;
}

vec2 hammersley(uint i, uint N)
{
    uint param = i;
    float _181 = radicalInverse_VdC(param);
    return vec2(float(i) / float(N), _181);
}

DistributionSample Lambertian(vec2 xi)
{
    DistributionSample lambertian;
    lambertian.cosTheta = sqrt(1.0 - xi.y);
    lambertian.sinTheta = sqrt(xi.y);
    lambertian.phi = 6.283185482025146484375 * xi.x;
    lambertian.pdf = lambertian.cosTheta / 3.1415927410125732421875;
    return lambertian;
}

float saturate(float v)
{
    return clamp(v, 0.0, 1.0);
}

float D_GGX(float NdotH, float roughness)
{
    float a = NdotH * roughness;
    float k = roughness / ((1.0 - (NdotH * NdotH)) + (a * a));
    return (k * k) * 0.3183098733425140380859375;
}

DistributionSample GGX(vec2 xi, float roughness)
{
    float alpha = roughness * roughness;
    float param = sqrt((1.0 - xi.y) / (1.0 + (((alpha * alpha) - 1.0) * xi.y)));
    DistributionSample ggx;
    ggx.cosTheta = saturate(param);
    ggx.sinTheta = sqrt(1.0 - (ggx.cosTheta * ggx.cosTheta));
    ggx.phi = 6.283185482025146484375 * xi.x;
    float param_1 = ggx.cosTheta;
    float param_2 = alpha;
    ggx.pdf = D_GGX(param_1, param_2);
    ggx.pdf /= 4.0;
    return ggx;
}

mat3 generateTBN(vec3 normal)
{
    vec3 bitangent = vec3(0.0, 1.0, 0.0);
    float NdotUp = dot(normal, vec3(0.0, 1.0, 0.0));
    float epsilon = 1.0000000116860974230803549289703e-07;
    if ((1.0 - abs(NdotUp)) <= epsilon)
    {
        if (NdotUp > 0.0)
        {
            bitangent = vec3(0.0, 0.0, 1.0);
        }
        else
        {
            bitangent = vec3(0.0, 0.0, -1.0);
        }
    }
    vec3 tangent = normalize(cross(bitangent, normal));
    bitangent = cross(normal, tangent);
    return mat3(vec3(tangent), vec3(bitangent), vec3(normal));
}

vec4 getImportanceSample(int sampleIndex, vec3 N, float roughness)
{
    uint param = uint(sampleIndex);
    uint param_1 = uint(ubuf2.sampleCount);
    vec2 xi = hammersley(param, param_1);
    DistributionSample importanceSample;
    if (ubuf2.distribution == 0)
    {
        vec2 param_2 = xi;
        importanceSample = Lambertian(param_2);
    }
    else
    {
        if (ubuf2.distribution == 1)
        {
            vec2 param_3 = xi;
            float param_4 = roughness;
            importanceSample = GGX(param_3, param_4);
        }
    }
    vec3 localSpaceDirection = normalize(vec3(importanceSample.sinTheta * cos(importanceSample.phi), importanceSample.sinTheta * sin(importanceSample.phi), importanceSample.cosTheta));
    vec3 param_5 = N;
    mat3 TBN = generateTBN(param_5);
    vec3 direction = TBN * localSpaceDirection;
    return vec4(direction, importanceSample.pdf);
}

float computeLod(float pdf)
{
    float lod = 0.5 * log2(((6.0 * ubuf2.resolution) * ubuf2.resolution) / (float(ubuf2.sampleCount) * pdf));
    return lod;
}

vec4 decodeRGBE(vec4 rgbe)
{
    float f = pow(2.0, (255.0 * rgbe.w) - 128.0);
    return vec4(rgbe.xyz * f, 1.0);
}

vec3 filterColor(vec3 N)
{
    vec3 color = vec3(0.0);
    float weight = 0.0;
    for (int i = 0; i < ubuf2.sampleCount; i++)
    {
        int param = i;
        vec3 param_1 = N;
        float param_2 = ubuf2.roughness;
        vec4 importanceSample = getImportanceSample(param, param_1, param_2);
        vec3 H = importanceSample.xyz;
        float pdf = importanceSample.w;
        float param_3 = pdf;
        float lod = computeLod(param_3);
        lod += ubuf2.lodBias;
        if (ubuf2.distribution == 0)
        {
            vec4 param_4 = texture(environmentMap, H);
            vec3 lambertian = decodeRGBE(param_4).xyz;
            color += lambertian;
        }
        else
        {
            if (ubuf2.distribution == 1)
            {
                vec3 V = N;
                vec3 L = normalize(reflect(-V, H));
                float NdotL = dot(N, L);
                if (NdotL > 0.0)
                {
                    if (ubuf2.roughness == 0.0)
                    {
                        lod = ubuf2.lodBias;
                    }
                    vec4 param_5 = texture(environmentMap, L);
                    vec3 sampleColor = decodeRGBE(param_5).xyz;
                    color += (sampleColor * NdotL);
                    weight += NdotL;
                }
            }
        }
    }
    if (weight != 0.0)
    {
        color /= vec3(weight);
    }
    else
    {
        color /= vec3(float(ubuf2.sampleCount));
    }
    return color;
}

vec4 encodeRGBE(vec4 rgba)
{
    float maxMan = max(rgba.x, max(rgba.y, rgba.z));
    float maxExp = 1.0 + floor(log2(maxMan));
    return vec4(rgba.xyz / vec3(pow(2.0, maxExp)), (maxExp + 128.0) / 255.0);
}

void main()
{
    vec3 direction = normalize(localPos);
    vec3 param = direction;
    vec3 color = filterColor(param);
    vec4 param_1 = vec4(color, 1.0);
    FragColor = encodeRGBE(param_1);
}

