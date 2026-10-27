/* The fragment shader of the board reference program tri_varying.c (Mesa 25.0.7 on
 * the NanoPC-T6, tools/mali/README-pan_shc.md), verbatim. */
precision mediump float; varying vec3 vcol; void main(){ gl_FragColor = vec4(vcol, 1.0); }
