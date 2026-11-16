#version 100
/* glmark2 22c527cb ideas, fragment shader 1acaaccc (the lit lamp): a global
 * array of three structs of vec4s, filled with constant constructors, one
 * member then set from three uniforms, and a for loop with an int counter
 * from 0 to the constant 3 that indexes the array and calls the
 * early-return function of gm2_cf_earlyret_fs in its body. */
precision mediump float;
struct Lamp {
    vec4 amb;
    vec4 dif;
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
    lamps[0] = Lamp(vec4(0.0, 0.0, 0.0, 1.0), vec4(1.0), vec4(0.0, 1.0, 0.0, 0.0));
    lamps[1] = Lamp(vec4(0.0, 0.0, 0.0, 1.0), vec4(0.3, 0.3, 0.5, 1.0), vec4(-1.0, 0.0, 0.0, 0.0));
    lamps[2] = Lamp(vec4(0.2, 0.2, 0.2, 1.0), vec4(0.2, 0.2, 0.2, 1.0), vec4(0.0, -1.0, 0.0, 0.0));
    lamps[0].pos = u_l0;
    lamps[1].pos = u_l1;
    lamps[2].pos = u_l2;
    vec3 n = normalize(v_nrm);
    vec4 dsum = vec4(0.0);
    vec4 asum = vec4(0.0);
    for (int k = 0; k < 3; k++) {
        vec3 l = normalize(towards(v_eye, lamps[k].pos));
        dsum += max(0.0, dot(n, l)) * lamps[k].dif;
        asum += lamps[k].amb;
    }
    gl_FragColor = vec4(1.0, 0.2, 0.2, 1.0) * dsum + vec4(0.1) * asum + vec4(v_dir, 0.0);
}
