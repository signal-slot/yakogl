#version 100
/* Reduction of glmark2's effect2d Laplace kernel (kernel=0,1,0;1,-4,1;0,1,0;,
 * 3c3d): nine look-ups times Kernel<i>, four of the weights 0.0 and one -4.0. */
precision mediump float;
uniform sampler2D Texture0;
varying vec2 TextureCoord;
const float TextureStepX = 0.001250;
const float TextureStepY = 0.002083;
const float Kernel0 = 0.0;
const float Kernel1 = 1.0;
const float Kernel2 = 0.0;
const float Kernel3 = 1.0;
const float Kernel4 = -4.0;
const float Kernel5 = 1.0;
const float Kernel6 = 0.0;
const float Kernel7 = 1.0;
const float Kernel8 = 0.0;
void main(void)
{
    vec4 result;
    result = texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel0 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, -1.0 * TextureStepY)) * Kernel2 +
    texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel3 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel4 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 0.0 * TextureStepY)) * Kernel5 +
    texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel6 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel7 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 1.0 * TextureStepY)) * Kernel8;
    gl_FragColor = vec4(result.xyz, 1.0);
}
