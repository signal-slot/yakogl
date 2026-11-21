#version 100
/* Two constant directions and a specular power: the blinn-phong light and
 * half vectors with a pow(max(., 0.0), 64.0) (the shading 994f tail, bump
 * 487d, 77c5 and d442). */
precision mediump float;
varying vec3 Normal;
const vec3 LightPos = vec3(12.0, 9.0, 8.0);
const vec3 HalfVec = vec3(0.3, 0.6, 0.9);
void main(void)
{
    vec3 N = normalize(Normal);
    vec3 L = normalize(LightPos);
    vec3 H = normalize(HalfVec);
    float diff = max(dot(N, L), 0.0);
    float spec = pow(max(dot(N, H), 0.0), 64.0);
    gl_FragColor = vec4(vec3(0.1) + vec3(0.7, 0.7, 0.2) * diff + vec3(0.5) * spec, 1.0);
}
