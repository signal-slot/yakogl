#version 100
precision highp float;
uniform vec4 u_a;
uniform vec4 u_b;
uniform vec4 u_c;
void main()
{
    float eq = u_a.x == u_a.y ? u_b.x : 0.0;
    float sg = u_a.z >= 0.0 ? clamp(u_a.w * u_b.y, 0.0, 1.0) : -u_b.z;
    float sh = u_c.x >= 0.0 ? clamp(u_c.y * u_c.z, 0.0, 1.0) : -u_c.y;
    float ab = abs(u_a.x) * u_b.w;
    float am = abs(u_c.w) * u_b.x + u_a.y;
    float t = -u_b.w;
    float st = t >= 0.0 ? clamp(u_c.x * u_a.z, 0.0, 1.0) : t;
    gl_FragColor = vec4(eq + am, sg + st, ab, sh);
}
