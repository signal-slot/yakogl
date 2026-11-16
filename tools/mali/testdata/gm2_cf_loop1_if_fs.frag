#version 100
/* glmark2 22c527cb terrain, fragment shader 4d266cda (the terrain itself):
 * a for loop with an int counter from 0 to the constant 1 over uniform
 * arrays of one element, whose body has a one-armed `if` on an array
 * element compared with 0.0 that reassigns a local (1.0 less a min of a
 * length over that element and 1.0), and sums a lit term. */
precision mediump float;
uniform vec3 u_lcol[1];
uniform vec3 u_lpos[1];
uniform float u_lrange[1];
uniform mat4 u_view;
varying vec3 v_nrm;
varying vec3 v_view;
void main() {
    vec3 n = normalize(v_nrm);
    vec3 sum = vec3(0.0);
    for (int k = 0; k < 1; k++) {
        vec4 lp = u_view * vec4(u_lpos[k], 1.0);
        highp vec3 lv = lp.xyz + v_view;
        float att = 1.0;
        if (u_lrange[k] > 0.0)
            att = 1.0 - min(length(lv) / u_lrange[k], 1.0);
        lv = normalize(lv);
        sum += att * u_lcol[k] * max(dot(n, lv), 0.0);
    }
    gl_FragColor = vec4(sum, 1.0);
}
