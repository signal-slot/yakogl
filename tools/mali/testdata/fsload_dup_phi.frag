/* p and q join the same two values: Mesa merges them (two stores), but its vertex words for the join are not this back end's; refused (VaryingDuplicate). */
precision mediump float;
varying vec2 p; varying vec2 q;
void main() { gl_FragColor = vec4(p.x, q.y, 0.5, 1.0); }
