// SPDX-License-Identifier: GPL-3.0-only
// glmark2 22c527cb: the text glmark2 passed to glShaderSource (README.md)
#if defined(GL_ES)
#define HIGHP_OR_DEFAULT highp
#else
#define HIGHP_OR_DEFAULT
#endif
#if defined(GL_ES)
#define MEDIUMP_OR_DEFAULT mediump
#else
#define MEDIUMP_OR_DEFAULT
#endif
attribute vec3 position;

uniform mat4 ModelViewProjectionMatrix;

// Removing this varying causes an inexplicable performance regression
// with r600g... Keeping it for now.
varying vec4 dummy;

void main(void)
{
    dummy = vec4(1.0);

    float d = fract(position.x);

#if defined(GL_ES)
#define HIGHP_OR_DEFAULT highp
#else
#define HIGHP_OR_DEFAULT
#endif
#if defined(GL_ES)
#define MEDIUMP_OR_DEFAULT mediump
#else
#define MEDIUMP_OR_DEFAULT
#endif
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);


    vec4 pos = vec4(position.x,
                    position.y + 0.1 * d * fract(position.x),
                    position.z, 1.0);

    // Transform the position to clip coordinates
    gl_Position = ModelViewProjectionMatrix * pos;
}


