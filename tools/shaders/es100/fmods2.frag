#version 100
precision highp float;
uniform vec4 u_a;
varying vec4 v_t;
void main()
{
    vec4 t = v_t * u_a;
    gl_FragColor = vec4(t.z * abs(t.w), -(t.x * t.y) - t.z, t.w * abs(t.x), -(t.y * t.w) - t.x);
}
