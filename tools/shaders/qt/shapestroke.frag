#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float matrixScale;
    float opacity;
    float devicePixelRatio;
    float strokeWidth;
    vec4 strokeColor;
    float debug;
    float reserved5;
    float reserved6;
    float reserved7;
};

uniform buf ubuf;

varying vec4 HGOW;
varying vec2 A;
varying vec2 B;
varying vec2 C;
varying vec4 P;

float cuberoot(float x)
{
    return sign(x) * pow(abs(x), 0.3333333432674407958984375);
}

vec3 solveDepressedCubic(float H, float G)
{
    float D = ((G * G) / 4.0) + (((H * H) * H) / 27.0);
    float param = ((-G) / 2.0) - sqrt(D);
    float u1 = cuberoot(param);
    float param_1 = ((-G) / 2.0) + sqrt(D);
    float u2 = cuberoot(param_1);
    vec3 rootsD1 = vec3(u1 - (H / (3.0 * u1)), u2 - (H / (3.0 * u2)), 0.0);
    float v = 2.0 * sqrt((-H) / 3.0);
    float t = acos(((3.0 * G) / H) / v) / 3.0;
    float k = 2.094395160675048828125;
    vec3 rootsD2 = vec3(v * cos(t), v * cos(t - k), v * cos(t - (2.0 * k)));
    bvec3 _119 = bvec3(D > 0.0);
    return vec3(_119.x ? rootsD1.x : rootsD2.x, _119.y ? rootsD1.y : rootsD2.y, _119.z ? rootsD1.z : rootsD2.z);
}

void main()
{
    float offset = HGOW.z;
    float param = HGOW.x;
    float param_1 = HGOW.y;
    vec3 s = solveDepressedCubic(param, param_1) - vec3(offset);
    vec2 Qmin = vec2(10000000000.0);
    float dmin = 10000.0;
    for (int i = 0; i < 3; i++)
    {
        float t = clamp(s[i], 0.0, 1.0);
        vec2 Q = (((A * t) * t) + (B * t)) + C;
        float d = length(Q - P.xy);
        float foundNewMin = step(d, dmin);
        dmin = min(d, dmin);
        Qmin = (Q * foundNewMin) + (Qmin * (1.0 - foundNewMin));
    }
    vec2 n = (P.xy - Qmin) / vec2(dmin);
    vec2 Q1 = Qmin + (n * (HGOW.w / 2.0));
    vec2 Q2 = Qmin - (n * (HGOW.w / 2.0));
    vec2 Q1_s = Q1 * ubuf.matrixScale;
    vec2 Q2_s = Q2 * ubuf.matrixScale;
    vec2 P_s = P.xy * ubuf.matrixScale;
    vec2 n_s = n;
    float dist1 = dot(P_s - Q1_s, n_s);
    float dist2 = dot(P_s - Q2_s, n_s);
    float fillCoverageDia = clamp(step(0.0, -dist1) + (sign(dist1) * pow(max(0.0, 0.707106769084930419921875 - abs(dist1)), 2.0)), 0.0, 1.0) - clamp(step(0.0, -dist2) + (sign(dist2) * pow(max(0.0, 0.707106769084930419921875 - abs(dist2)), 2.0)), 0.0, 1.0);
    float fillCoverage = fillCoverageDia;
    float centerline = step(HGOW.w * 0.00999999977648258209228515625, dmin);
    fillCoverage = (fillCoverage * centerline) + (min(1.0, HGOW.w * ubuf.matrixScale) * (1.0 - centerline));
    gl_FragData[0] = (((vec4(ubuf.strokeColor.xyz, 1.0) * ubuf.strokeColor.w) * fillCoverage) * ubuf.opacity) + (((vec4(0.0, 0.5, 1.0, 1.0) * ubuf.debug) * (1.0 - fillCoverage)) * ubuf.opacity);
}

