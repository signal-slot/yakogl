#pragma STDGL invariant(all)
precision highp float; uniform vec4 a, b, c, d;
void main(){ gl_FragColor = (a + b + c + d) * 0.25; }
