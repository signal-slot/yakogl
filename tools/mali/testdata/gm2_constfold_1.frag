#version 100
/* Reduction of glmark2's desktop blur (separable, horizontal pass, 7f17):
 * the generated sum of look-ups times Kernel<|offset|> with the centre tap's
 * `0.0 * TextureStepX` offset and the trailing `+ 0.0` term, radius 2. */
precision mediump float;
uniform sampler2D Texture0;
varying vec2 TextureCoord;
const float Kernel0 = 0.239365;
const float Kernel1 = 0.198972;
const float Kernel2 = 0.116496;
const float TextureStepX = 0.001250;
const float TextureStepY = 0.002083;
void main(void)
{
    vec4 result;
    result =
    texture2D(Texture0, TextureCoord + vec2(-2.0 * TextureStepX, 0.0)) * Kernel2 +
    texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 0.0)) * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(0.0 * TextureStepX, 0.0)) * Kernel0 +
    texture2D(Texture0, TextureCoord + vec2(1.0 * TextureStepX, 0.0)) * Kernel1 +
    texture2D(Texture0, TextureCoord + vec2(2.0 * TextureStepX, 0.0)) * Kernel2 +
    0.0 ;
    gl_FragColor = vec4(result.xyz, 1.0);
}
