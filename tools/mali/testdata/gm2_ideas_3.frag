#version 100
/* study-ideas: glmark2 22c527cb ideas, fragment shader 1acaaccc (the lit
 * lamp), the whole body: a global array of three structs of four vec4s
 * filled with constant constructors, the position member set from three
 * uniforms, and a for loop with an int counter from 0 to the constant 3
 * that indexes the array, calls the early-return function and sums specular,
 * diffuse and ambient terms (pow by 20.0, reflect, normalize).
 * gm2_cf_constloop_struct_fs has three members and no specular term. */
precision mediump float;
struct Lamp {
    vec4 amb;
    vec4 dif;
    vec4 spc;
    vec4 pos;
};
Lamp lamps[3];
uniform vec4 u_l0;
uniform vec4 u_l1;
uniform vec4 u_l2;
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
    lamps[0] = Lamp(vec4(0.0, 0.0, 0.0, 1.0), vec4(1.0, 1.0, 1.0, 1.0), vec4(1.0, 1.0, 1.0, 1.0), vec4(0.0, 1.0, 0.0, 0.0));
    lamps[1] = Lamp(vec4(0.0, 0.0, 0.0, 1.0), vec4(0.3, 0.3, 0.5, 1.0), vec4(0.3, 0.3, 0.5, 1.0), vec4(-1.0, 0.0, 0.0, 0.0));
    lamps[2] = Lamp(vec4(0.2, 0.2, 0.2, 1.0), vec4(0.2, 0.2, 0.2, 1.0), vec4(0.2, 0.2, 0.2, 1.0), vec4(0.0, -1.0, 0.0, 0.0));
    vec4 mamb = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 mdif = vec4(1.0, 0.2, 0.2, 1.0);
    vec4 mspc = vec4(0.5, 0.5, 0.5, 1.0);
    float shin = 20.0;
    vec4 dsum = vec4(0.0);
    vec4 ssum = vec4(0.0);
    vec4 asum = vec4(0.0);
    vec3 n = normalize(v_nrm);
    lamps[0].pos = u_l0;
    lamps[1].pos = u_l1;
    lamps[2].pos = u_l2;
    for (int k = 0; k < 3; k++) {
        vec4 lp = lamps[k].pos;
        vec3 l = normalize(towards(v_eye, lp));
        vec3 r = reflect(-l, n);
        ssum += pow(max(0.0, dot(r, v_dir)), shin) * lamps[k].spc;
        dsum += max(0.0, dot(n, l)) * lamps[k].dif;
        asum += lamps[k].amb;
    }
    gl_FragColor = (mspc * ssum) + (mamb * asum) + (mdif * dsum);
}
