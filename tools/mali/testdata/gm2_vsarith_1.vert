/* the base of conditionals, function and loop: d = fract(x), and 0.1 * d * fract(x) in y: the Mesa 26.2.1
 * drm-shim reference. */
attribute vec3 position;
uniform mat4 mvp;
varying vec4 dummy;
void main(void)
{
    dummy = vec4(1.0);
    float d = fract(position.x);

    vec4 pos = vec4(position.x, position.y + 0.1 * d * fract(position.x), position.z, 1.0);
    gl_Position = mvp * pos;
}
