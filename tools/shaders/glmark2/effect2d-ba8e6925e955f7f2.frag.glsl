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
const float Kernel0 = 0.066667;
const float Kernel1 = 0.066667;
const float Kernel2 = 0.066667;
const float Kernel3 = 0.066667;
const float Kernel4 = 0.066667;
const float Kernel5 = 0.066667;
const float Kernel6 = 0.066667;
const float Kernel7 = 0.066667;
const float Kernel8 = 0.066667;
const float Kernel9 = 0.066667;
const float Kernel10 = 0.066667;
const float Kernel11 = 0.066667;
const float Kernel12 = 0.066667;
const float Kernel13 = 0.066667;
const float Kernel14 = 0.066667;
const float TextureStepY = 0.001667;
const float TextureStepX = 0.001250;
uniform sampler2D Texture0;
varying vec2 TextureCoord;

void main(void)
{
    vec4 result;

    result = texture2D(Texture0, TextureCoord + vec2(-2.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel0 +
texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel1 +
texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel2 +
texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel3 +
texture2D(Texture0, TextureCoord + vec2(2.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel4 +
texture2D(Texture0, TextureCoord + vec2(-2.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel5 +
texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel6 +
texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel7 +
texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel8 +
texture2D(Texture0, TextureCoord + vec2(2.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel9 +
texture2D(Texture0, TextureCoord + vec2(-2.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel10 +
texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel11 +
texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel12 +
texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel13 +
texture2D(Texture0, TextureCoord + vec2(2.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel14;


    gl_FragColor = vec4(result.xyz, 1.0);
}

