#version 100
/* One tap of glmark2's desktop blur, horizontal pass (7f17): the look-up at
 * TextureCoord + vec2(offset * TextureStepX, 0.0) times a Kernel weight. The
 * 0.0 is the y lane of the offset (a constant added to a varying lane), the
 * smallest shape that carries the CONST_FOLD refusal of the blur passes. */
precision mediump float;
uniform sampler2D Texture0;
varying vec2 TextureCoord;
const float Kernel1 = 0.198972;
const float TextureStepX = 0.001250;
void main(void)
{
    vec4 result;
    result = texture2D(Texture0, TextureCoord + vec2(-1.0 * TextureStepX, 0.0)) * Kernel1;
    gl_FragColor = vec4(result.xyz, 1.0);
}
