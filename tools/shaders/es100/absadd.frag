#version 100
precision highp float;
uniform vec4 u_a;
uniform vec4 u_b;
void main()
{
    gl_FragColor = vec4(u_a.x + abs(u_b.y), u_a.y - abs(u_b.x), abs(u_a.z) + u_b.z, 1.0);
}
