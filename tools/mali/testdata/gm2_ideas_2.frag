#version 100
/* study-ideas: glmark2 22c527cb ideas, fragment shader c5652010 (the logo),
 * the whole body: the early-return vec3 function of two vec4s (`&&` of two
 * w compares, three returns) called with a varying and a uniform, the
 * normalized results in a reflection, pow(max(0.0, dot), 30.0) and a lit
 * colour from constant vec4 products. gm2_cf_earlyret_fs has no pow and
 * folds fewer constants. */
precision mediump float;
uniform vec4 u_light;
varying vec3 v_nrm;
varying vec4 v_eye;
varying vec3 v_dir;
vec3 towards(vec4 a, vec4 b) {
    if (a.w == 0.0 && b.w == 0.0)
        return vec3(b - a);
    if (a.w == 0.0)
        return vec3(-a);
    if (b.w == 0.0)
        return vec3(b);
    return b.xyz / b.w - a.xyz / a.w;
}
void main() {
    vec4 amb = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 dif = vec4(1.0, 1.0, 1.0, 1.0);
    vec4 spc = vec4(1.0, 1.0, 1.0, 1.0);
    vec4 mamb = vec4(0.1, 0.1, 0.1, 1.0);
    vec4 mdif = vec4(0.5, 0.4, 0.7, 1.0);
    vec4 mspc = vec4(1.0, 1.0, 1.0, 1.0);
    float shin = 30.0;
    vec3 l = normalize(towards(v_eye, u_light));
    vec3 n = normalize(v_nrm);
    vec3 r = reflect(-l, n);
    float st = pow(max(0.0, dot(r, v_dir)), shin);
    float dt = max(0.0, dot(n, l));
    vec4 s = spc * mspc;
    vec4 a = amb * mamb;
    vec4 d = dif * mdif;
    gl_FragColor = s * st + a + d * dt;
}
