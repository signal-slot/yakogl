/* #1385 (a): the fragment shader of the tex_vs_* references, which draws
 * the vertex stage's texel. */
precision mediump float; varying vec4 v_color; void main(){ gl_FragColor = v_color; }
