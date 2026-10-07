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
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
#endif
uniform sampler2D uSampler;
uniform sampler2D uSampler1;
uniform float uCurrentTime;
  
varying vec2 vTextureCoord;
varying vec4 vWorld;
varying vec3 vDiffuse;
varying vec3 vAmbient;
varying vec3 vFresnel;

void main(void)
{
    vec4 caustics = texture2D(uSampler1, vec2(vWorld.x / 24.0 + uCurrentTime / 20.0, (vWorld.z - vWorld.y)/48.0 + uCurrentTime / 40.0));
    vec4 colorMap = texture2D(uSampler, vTextureCoord);
    float transparency = colorMap.a + pow(vFresnel.r, 2.0) - 0.3;
    gl_FragColor = vec4(((vAmbient + vDiffuse + caustics.rgb) * colorMap.rgb), transparency);
}
