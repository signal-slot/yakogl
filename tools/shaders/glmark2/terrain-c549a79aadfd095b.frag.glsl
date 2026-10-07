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
uniform float height;
uniform vec2 resolution;
uniform sampler2D heightMap;

varying vec2 vUv;

void main()
{
    float val = texture2D( heightMap, vUv ).x;

    float valU = texture2D( heightMap, vUv + vec2( 1.0 / resolution.x, 0.0 ) ).x;
    float valV = texture2D( heightMap, vUv + vec2( 0.0, 1.0 / resolution.y ) ).x;

    gl_FragColor = vec4( ( 0.5 * normalize( vec3( val - valU, val - valV, height  ) ) + 0.5 ), 1.0 );
}
