#version 100
precision highp float;
uniform float u_cut;
uniform float u_w;
varying vec2 v_uv;
void main() {
    if (v_uv.x < u_cut)
        discard;
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 4; i++)
        acc += vec3(1.0, 2.0, 3.0) * u_w * float(i + 1);
    if (v_uv.x + v_uv.y > 1.75)
        discard;
    gl_FragColor = vec4(acc, 1.0);
}
