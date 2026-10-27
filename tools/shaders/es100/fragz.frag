#version 100
precision highp float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color * gl_FragCoord.z;
}
