varying mediump float v_vtxOut;

uniform bvec3 u_a;

void main (void)
{
    mediump float result = v_vtxOut;
    bool RES = (u_a == bvec3(true, false, true));
    gl_FragColor = vec4(float(RES) * result, 0.0, 0.0, 1.0);
}
