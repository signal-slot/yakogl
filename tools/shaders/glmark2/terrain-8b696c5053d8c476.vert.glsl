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
uniform vec2 uvOffset;
uniform MEDIUMP_OR_DEFAULT vec2 uvScale;

attribute vec3 position;

varying vec2 vUv;

void main()
{
    vUv = uvScale * (position.xy * 0.5 + 0.5) + uvOffset;
    gl_Position = vec4( position, 1.0);
}
