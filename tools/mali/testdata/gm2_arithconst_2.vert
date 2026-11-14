uniform mat4 proj;
uniform mat4 view;
attribute vec2 vertex;
void main()
{
    vec4 p = vec4(vertex.x, vertex.y, 0.0, 1.0);
    gl_Position = proj * view * p;
}
