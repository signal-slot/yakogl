attribute vec4 a_pos;
attribute vec3 normal, tangent;
varying vec3 v_b;
void main()
{
    gl_Position = a_pos;
    v_b = cross(normal, tangent);
}
