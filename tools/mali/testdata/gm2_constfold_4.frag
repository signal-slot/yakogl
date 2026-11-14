#version 100
/* Reduction of glmark2's desktop blur, non-separable (BlurDirectionBoth):
 * look-up * Kernel<|i|> * Kernel<|j|> over a 3x3 window (radius 1) and the
 * trailing ` 0.0;` term. */
precision mediump float;
uniform sampler2D Texture0;
varying vec2 TextureCoord;
const float Kernel0 = 0.239365;
const float Kernel1 = 0.198972;
const float TextureStepX = 0.001250;
const float TextureStepY = 0.002083;
void main(void)
{
    vec4 result;
    result =
    texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel1 * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel1 * Kernel0 +
    texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel1 * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel0 * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel0 * Kernel0 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel0 * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel1 * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel1 * Kernel0 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel1 * Kernel1 +
     0.0;
    gl_FragColor = vec4(result.xyz, 1.0);
}
