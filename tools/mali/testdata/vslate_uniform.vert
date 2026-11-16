attribute highp vec4 p; attribute highp vec4 b; uniform mediump float u; varying mediump vec4 v; void main(){ gl_Position = p; v = vec4(b.x, b.y, u, 1.0) * vec4(b.z, b.w, 0.0, 1.0); }
