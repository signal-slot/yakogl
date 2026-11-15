#version 300 es
layout(location = 3) in highp vec4 p; layout(location = 1) in highp vec4 b;
out mediump vec4 v;
void main(){ gl_Position = p; v = b; }
