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
attribute vec3 normal;

uniform mat4 ModelViewProjectionMatrix;
uniform mat4 NormalMatrix;

varying vec3 Normal;

void main(void)
{
    // Transform the normal to eye coordinates
    Normal = normalize(vec3(NormalMatrix * vec4(normal, 1.0)));

    // Transform the position to clip coordinates
    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);
}
