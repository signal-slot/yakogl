attribute highp vec4 p;
varying highp vec4 v;
attribute highp float a; attribute highp float b; attribute highp float c; attribute highp float d;
void main() { gl_Position = p;
  mediump float ma = a; mediump float mb = b; mediump float mc = c; mediump float md = d;
  v = vec4(ma + mb + mc + md, ma * mb, mc * md, 0.0); }
