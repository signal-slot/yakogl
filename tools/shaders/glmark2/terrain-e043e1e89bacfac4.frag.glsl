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
const float TextureStepX = 0.001250;
const float TiltShift = 0.000000;
const float Kernel2 = 0.187627;
const float Kernel1 = 0.206068;
const float Kernel0 = 0.212609;
uniform sampler2D Texture0;

varying vec2 vUv;

void main(void)
{
    vec4 result;
    vec2 TextureCoord = vUv;

    float stepX = TextureStepX * abs(TiltShift - TextureCoord.y) / abs(1.0 - TiltShift);
result = 
texture2D(Texture0, TextureCoord + vec2(-2.0 * stepX, 0.0)) * Kernel2 +
texture2D(Texture0, TextureCoord + vec2(-1.0 * stepX, 0.0)) * Kernel1 +
texture2D(Texture0, TextureCoord + vec2(0.0 * stepX, 0.0)) * Kernel0 +
texture2D(Texture0, TextureCoord + vec2(1.0 * stepX, 0.0)) * Kernel1 +
texture2D(Texture0, TextureCoord + vec2(2.0 * stepX, 0.0)) * Kernel2 +
0.0 ;


    gl_FragColor = vec4(result.xyz, 1.0);
}

