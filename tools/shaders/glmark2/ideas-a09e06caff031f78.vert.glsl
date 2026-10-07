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
uniform mat4 projection;
uniform mat4 modelview;
uniform float currentTime;
attribute vec2 vertex;
varying vec4 color;

void main()
{
    vec4 curVertex = vec4(vertex.x, vertex.y, 0.0, 1.0);
    gl_Position = projection * modelview * curVertex;
    float referenceTime = 15.0;
    float c = 0.0;
    if (currentTime > referenceTime * 1.0 - 5.0)
    {
        c = (currentTime - (referenceTime * 1.0 - 5.0)) / 2.0;
    }
    color = vec4(c, c, c, 1.0);
}
