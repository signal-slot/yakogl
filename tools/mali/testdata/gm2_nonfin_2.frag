#version 100
/* The constant alone, the smallest shape: a mediump normalize of a
 * constant vec3 whose squares are 1.0 * 1.0 and 0.5 * 0.5 (cel fea0's
 * constant lane products). */
precision mediump float;
varying vec3 Normal;
void main(void)
{
    vec3 H = normalize(vec3(0.5, 0.5, 1.0));
    gl_FragColor = vec4(H * 0.5 + Normal * 0.25, 1.0);
}
