/* loop: an int-counter for loop of fract(3d); VertexLoops declared and unused as in glmark2's: the Mesa 26.2.1
 * drm-shim reference. */
attribute vec3 position;
uniform mat4 mvp;
uniform int VertexLoops;
varying vec4 dummy;
void main(void)
{
    dummy = vec4(1.0);
    float d = fract(position.x);
    for (int i = 0; i < 5; i++)
        d = fract(3.0 * d);

    vec4 pos = vec4(position.x, position.y + 0.1 * d * fract(position.x), position.z, 1.0);
    gl_Position = mvp * pos;
}
