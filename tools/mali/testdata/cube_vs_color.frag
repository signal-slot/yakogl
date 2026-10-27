/* #1385 (c): the fragment shader of the cube_vs reference, which draws the
 * vertex stage's texel; gl_test case 113's tools/shaders/es100/cubelod.frag. */
precision mediump float; varying vec4 v_col; void main(){ gl_FragColor = v_col; }
