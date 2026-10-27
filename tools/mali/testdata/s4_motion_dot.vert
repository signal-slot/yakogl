/* S4 code motion: a dot product is not moved (pan_shc's record, tools/mali/testdata/pan_shc-out/s4_motion_dot.json; the program is highp fp32 on a varying, as femtovg's are). */
attribute vec4 p; attribute vec4 c; varying vec4 v; void main(){ gl_Position = p; v = c; }
