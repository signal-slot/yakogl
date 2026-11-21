#version 100
/* A mediump normalize of a constant vec3 beside one of the varying
 * (glmark2's shading blinn-phong-inf 994f and bump normals/high-poly shape,
 * with other constants): Mesa folds normalize(const) at fp32 in the GLSL
 * front end and lowers the vector to fp16 constants. */
precision mediump float;
varying vec3 Normal;
void main(void)
{
    vec3 N = normalize(Normal);
    vec3 L = normalize(vec3(3.0, 4.0, 12.0));
    float d = max(dot(N, L), 0.0);
    gl_FragColor = vec4(vec3(d), 1.0);
}
