#version 100
precision highp float;
uniform vec4 u_a;
uniform vec4 u_b;
uniform vec4 u_c;
varying vec4 v_t;
void main()
{
    vec4 t = v_t * u_a;
    float s0 = abs(t.x) + abs(t.y);
    float s1 = t.z * abs(t.w);
    float s2 = t.x * abs(t.y) + t.z;
    float s3 = t.y * t.z - t.w;
    float s4 = -(t.z * t.w) - t.x;
    float s5 = t.w * t.x + floor(t.y);
    float s6 = exp2(t.x) + exp2(-t.y);
    float s7 = log2(t.z * t.z + 1.0) + log2(abs(t.w));
    float s8 = inversesqrt(-min(t.x, -1.0));
    gl_FragColor = vec4(s0 + s1, s2 + s3, s4 + s5, s6 + s7 + s8) * u_b + u_c;
}
