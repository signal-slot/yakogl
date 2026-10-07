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
attribute vec3 vertex;
varying vec4 color;

void main()
{
    vec4 curVertex = vec4(vertex.x, vertex.y, vertex.z, 1.0);
    gl_Position = projection * modelview * curVertex;
    color = vec4(1.0, 1.0, 1.0, 1.0);
}
