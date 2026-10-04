// A dynamically indexed vec4 uniform array, ES 1.00 (Appendix A allows a
// uniform array indexed by any integer expression). Each element fills a
// register, so the read takes all four lanes of the element: until the
// indexed-array layout filled them, lanes y, z and w read unused zeros.
precision mediump float;
uniform vec4 v[8];
uniform float fi;
void main()
{
    gl_FragColor = v[int(fi * 2.0 + 4.0)];
}
