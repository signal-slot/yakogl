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
const vec4 MaterialDiffuse = vec4(0.300000, 0.300000, 0.300000, 1.000000);
attribute vec2 position;

uniform mat4 LightMatrix;
uniform mat4 ModelViewProjectionMatrix;

varying vec4 ShadowCoord;
varying vec4 Color;

void main()
{
    Color = MaterialDiffuse;

    vec4 pos4 = vec4(position, 0.0, 1.0);
    ShadowCoord = LightMatrix * pos4;
    gl_Position = ModelViewProjectionMatrix * pos4;
}
