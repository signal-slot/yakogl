#version 300 es
/* ES 3.00: a mediump local read back bare into the output by parts: the lanes read. */
precision mediump float;
in vec4 a; out vec4 o;
void main() { mediump vec4 t = a; o = vec4(1.0); o.xy = t.xy; }
