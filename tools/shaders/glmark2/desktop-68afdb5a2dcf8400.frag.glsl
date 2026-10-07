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
const float TextureStepY = 0.001667;
const float Kernel5 = 0.002659;
const float Kernel4 = 0.013437;
const float Kernel3 = 0.047370;
const float Kernel2 = 0.116512;
const float Kernel1 = 0.199935;
const float Kernel0 = 0.239365;
uniform sampler2D Texture0;

varying vec2 TextureCoord;

void main(void)
{
    vec4 result;

    result = 
texture2D(Texture0, TextureCoord + vec2(0.0, -5.0 * TextureStepY)) * Kernel5 +
texture2D(Texture0, TextureCoord + vec2(0.0, -4.0 * TextureStepY)) * Kernel4 +
texture2D(Texture0, TextureCoord + vec2(0.0, -3.0 * TextureStepY)) * Kernel3 +
texture2D(Texture0, TextureCoord + vec2(0.0, -2.0 * TextureStepY)) * Kernel2 +
texture2D(Texture0, TextureCoord + vec2(0.0, -1.0 * TextureStepY)) * Kernel1 +
texture2D(Texture0, TextureCoord + vec2(0.0, 0.0 * TextureStepY)) * Kernel0 +
texture2D(Texture0, TextureCoord + vec2(0.0, 1.0 * TextureStepY)) * Kernel1 +
texture2D(Texture0, TextureCoord + vec2(0.0, 2.0 * TextureStepY)) * Kernel2 +
texture2D(Texture0, TextureCoord + vec2(0.0, 3.0 * TextureStepY)) * Kernel3 +
texture2D(Texture0, TextureCoord + vec2(0.0, 4.0 * TextureStepY)) * Kernel4 +
texture2D(Texture0, TextureCoord + vec2(0.0, 5.0 * TextureStepY)) * Kernel5 +
0.0 ;


    gl_FragColor = vec4(result.xyz, 1.0);
}

