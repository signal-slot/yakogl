varying mediump float v_vtxOut;


void main (void)
{
    mediump float result = v_vtxOut;
    gl_FragColor = vec4(result, result, result, 1.0);
}
