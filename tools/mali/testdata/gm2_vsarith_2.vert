/* conditionals vertex-steps=5 (step-conditional): if (d >= 0.5) fract(2d) else fract(3d): the Mesa 26.2.1
 * drm-shim reference. */
attribute vec3 position;
uniform mat4 mvp;
varying vec4 dummy;
void main(void)
{
    dummy = vec4(1.0);
    float d = fract(position.x);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);

    vec4 pos = vec4(position.x, position.y + 0.1 * d * fract(position.x), position.z, 1.0);
    gl_Position = mvp * pos;
}
