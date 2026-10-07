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

varying vec2 TextureCoord;

void main(void)
{
    gl_Position = vec4(position, 1.0);

    TextureCoord = position.xy * 0.5 + 0.5;
}
