attribute vec4 pos;
attribute vec3 nrm;
uniform mat4 mvp;
varying vec4 vp;
varying vec3 vn;
void main()
{
    vp = pos;
    vn = nrm;
    gl_Position = mvp * pos;
}
