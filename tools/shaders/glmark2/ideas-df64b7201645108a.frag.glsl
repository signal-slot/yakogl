// SPDX-License-Identifier: GPL-3.0-only
// glmark2 22c527cb: the text glmark2 passed to glShaderSource (README.md)
#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)
#define HIGHP_OR_DEFAULT highp
#else
#define HIGHP_OR_DEFAULT
#endif
#if defined(GL_ES)
#define MEDIUMP_OR_DEFAULT mediump
#else
#define MEDIUMP_OR_DEFAULT
#endif
#ifdef GL_ES
precision mediump float;
#endif
uniform sampler2D tex;

void main()
{
    vec2 curPos = gl_FragCoord.xy / 32.0;
    vec4 color = texture2D(tex, curPos);
    if (color.w < 0.5)
        discard;
    gl_FragColor = color;
}
