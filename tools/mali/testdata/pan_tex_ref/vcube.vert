attribute highp vec2 a_position;
attribute highp vec3 a_texCoord;
uniform highp samplerCube u_texture;
uniform highp float u_lod;
varying mediump vec4 v_color;

void main()
{
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_color = textureCubeLod(u_texture, a_texCoord, u_lod);
}
