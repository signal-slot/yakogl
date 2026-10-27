// unitype_116.vert's v_color is read by nobody in uniidx_d.frag, so Mesa drops
// the vertex stage's vcols[vidx] read (unitype_116 records no varying
// program). This fragment shader reads it, to record the run-time-indexed
// uniform read of the vertex stage (with unitype_116.vert).
precision mediump float;
varying vec4 v_color;
void main()
{
    gl_FragColor = v_color;
}
