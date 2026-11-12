/* A mat3's column read in part: that column kept whole, .v3. */
precision mediump float;
varying mat3 m;
void main() { gl_FragColor = vec4(m[1].y, 0.5, 0.25, 1.0); }
