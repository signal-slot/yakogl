attribute vec3 position;
uniform mat4 projection;
uniform mat4 modelview;
void main()
{
    vec4 curVertex = vec4(position.x, position.y, position.z, 1.0);
    gl_Position = projection * modelview * curVertex;
}
