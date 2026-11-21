#version 100
/* Cel fea0's shape in the large: the constant light position's length and
 * a normalize, a threshold on the product and a mix of two constants. */
precision mediump float;
varying vec3 Normal;
const vec4 LightPos = vec4(4.0, 3.0, 1.0, 1.0);
void main(void)
{
    vec3 vertex_light = LightPos.xyz - Normal;
    vec3 N = normalize(Normal);
    vec3 L = normalize(vertex_light);
    float maxNdotL = max(dot(N, L), 0.0);
    float attenuation = length(LightPos) / length(vertex_light);
    vec4 c = vec4(0.0, 0.3, 0.0, 1.0);
    if (attenuation * maxNdotL >= 0.1)
        c = vec4(0.0, 0.6, 0.0, 1.0);
    if (dot(L, N) < mix(0.1, 0.4, maxNdotL))
        c = vec4(0.0, 0.0, 0.0, 1.0);
    gl_FragColor = vec4(c.xyz, 1.0);
}
