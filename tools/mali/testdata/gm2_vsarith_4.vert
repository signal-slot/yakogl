/* function: d = process(d), process holding a fract(3d) step and a product of two: the Mesa 26.2.1
 * drm-shim reference. */
attribute vec3 position;
uniform mat4 mvp;
varying vec4 dummy;
float process(float d)
{
    d = fract(3.0 * d);
    d = fract(3.0 * d) * fract(3.0 * d);
    return d;
}

void main(void)
{
    dummy = vec4(1.0);
    float d = fract(position.x);
    d = process(d);

    vec4 pos = vec4(position.x, position.y + 0.1 * d * fract(position.x), position.z, 1.0);
    gl_Position = mvp * pos;
}
