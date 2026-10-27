// gl_test case 101 "cubebias": a direction onto the +X face whose face
// coordinates (s = x, t = y for the +X major axis) span the whole face across
// the quad, so a 64-texel level 0 is sampled one texel per pixel -- an
// implicit level of detail of exactly 0.
attribute vec4 a_pos;
varying vec3 v_dir;
void main()
{
    gl_Position = a_pos;
    v_dir = vec3(1.0, -a_pos.y, -a_pos.x);
}
