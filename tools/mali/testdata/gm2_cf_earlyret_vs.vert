#version 100
/* glmark2 22c527cb ideas, vertex shader 51b217c5 (the logo and the lit
 * lamp): a vec3 function of two vec4s with three `if`s that return early
 * (a compare of each w with 0.0, the first two joined by &&) and a last
 * return of a difference of divisions, called once with a constant second
 * argument; the result is normalized into a varying. */
attribute vec3 a_pos;
attribute vec3 a_nrm;
uniform mat4 u_proj;
uniform mat4 u_mv;
uniform mat3 u_nm;
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
    vec4 p = vec4(a_pos, 1.0);
    gl_Position = u_proj * u_mv * p;
    v_nrm = u_nm * a_nrm;
    v_eye = u_mv * p;
    v_dir = normalize(towards(v_eye, vec4(0.0, 0.0, 0.0, 1.0)));
}
