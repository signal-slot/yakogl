/* Store conversions: the colour's own lanes are converted at the colour's store in its order, q.y, c.z, p.x, across the two loads, each load before its first conversion. */
precision mediump float;
varying vec3 c;
varying vec2 p;
varying vec2 q;
void main() { gl_FragColor = vec4(q.y, c.z, p.x, 1.0); }
