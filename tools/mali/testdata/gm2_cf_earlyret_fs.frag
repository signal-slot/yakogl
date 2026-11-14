#version 100
/* glmark2 22c527cb ideas, fragment shader c5652010 (the logo): the vec3
 * function of two vec4s with three early returns (w compared with 0.0, the
 * first two joined by &&), called once with a varying and a uniform, then
 * a lit colour from its normalized result. */
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
    vec3 l = normalize(towards(v_eye, u_light));
    vec3 n = normalize(v_nrm);
    float s = max(0.0, dot(reflect(-l, n), v_dir));
    float k = max(0.0, dot(n, l));
    gl_FragColor = vec4(0.5, 0.4, 0.7, 1.0) * k + vec4(s);
}
