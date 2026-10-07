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
uniform sampler2D MaterialTexture0;

varying vec4 Color;
varying vec2 TextureCoord;

void main(void)
{
    vec4 texel = texture2D(MaterialTexture0, TextureCoord);
    gl_FragColor = texel * Color;
}

