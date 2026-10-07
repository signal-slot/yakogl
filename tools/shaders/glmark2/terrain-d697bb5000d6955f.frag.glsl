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
uniform sampler2D tDiffuse;

varying vec2 vUv;

void main()
{
    vec4 texel = texture2D( tDiffuse, vUv );

    vec3 luma = vec3( 0.299, 0.587, 0.114 );

    float v = dot( texel.xyz, luma );

    gl_FragColor = vec4( v, v, v, texel.w );
}

