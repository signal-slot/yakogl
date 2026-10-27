#version 100
precision highp float;
#define UNIFORMARRAY_SIZE 14
uniform vec4 frag[UNIFORMARRAY_SIZE];
#define innerCol frag[6]
varying vec2 ftcoord;
varying vec2 fpos;
void main(void) {
    /* A texkill that can never fire at runtime (ftcoord.y is in [0,1], so the
     * argument is >= 0.0 and never < -100.0), but the compiler must still emit
     * the texkill instruction because the condition is data-dependent. Isolates
     * "texkill present but never taken" from strokeThr delivery. */
    if (ftcoord.y < -100.0) discard;
    gl_FragColor = innerCol;
}
