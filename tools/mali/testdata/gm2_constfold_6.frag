#version 100
/* One tap of glmark2's desktop blur, vertical pass (68af): the look-up at
 * TextureCoord + vec2(0.0, offset * TextureStepY) times a Kernel weight. The
 * 0.0 is the x lane of the offset. */
precision mediump float;
uniform sampler2D Texture0;
varying vec2 TextureCoord;
const float Kernel1 = 0.198972;
const float TextureStepY = 0.002083;
void main(void)
{
    vec4 result;
    result = texture2D(Texture0, TextureCoord + vec2(0.0, 1.0 * TextureStepY)) * Kernel1;
    gl_FragColor = vec4(result.xyz, 1.0);
}
