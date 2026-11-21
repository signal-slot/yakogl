#version 100
/* A constant length (cel fea0's `length(LightSourcePosition)`) over a
 * run-time length: the constant's squares are constants multiplied. */
precision mediump float;
varying vec3 Normal;
const vec4 LightPos = vec4(5.0, 2.0, 2.0, 1.0);
void main(void)
{
    vec3 v = LightPos.xyz - Normal;
    float attenuation = length(LightPos) / length(v);
    gl_FragColor = vec4(vec3(attenuation), 1.0);
}
