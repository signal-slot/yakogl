attribute vec3 pos;
attribute vec3 col;
uniform mat4 mvp;
uniform float t;
void main()
{
    float s = t / 15.0;
    vec3 p = pos + col / 8.0 * sin(s * 30.0 + pos.y / 0.5);
    gl_Position = mvp * vec4(p, 1.0);
}
