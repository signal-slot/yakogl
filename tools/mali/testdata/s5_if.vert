/* S5 (if): the vertex shader of the s5_* fragment programs, a plain copy (pan_shc, Mesa 26.2.1). */
attribute vec4 a; varying vec4 v; void main(){ v = a; gl_Position = a; }
