#version 100
/* effect2d convolution shape (original reduction of glmark2's
 * effect-2d-convolution.frag as scene-effect-2d.cpp fills it): emboss 3x3 (-2,-1,0;-1,1,1;0,1,2): one zero diagonal.
 * Step constants are 1/800 x 1/600. */
precision mediump float;
uniform sampler2D Texture0;
varying vec2 TextureCoord;
const float TextureStepX = 0.00125;
const float TextureStepY = 0.00166667;
const float Kernel0 = -2.0;
const float Kernel1 = -1.0;
const float Kernel2 = 0.0;
const float Kernel3 = -1.0;
const float Kernel4 = 1.0;
const float Kernel5 = 1.0;
const float Kernel6 = 0.0;
const float Kernel7 = 1.0;
const float Kernel8 = 2.0;
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
