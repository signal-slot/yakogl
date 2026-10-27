#version 300 es
// gl_test case "cubelod", vertex stage: textureLod on a cube at an
// explicit level, per vertex, in directions across the whole +X face (up to
// 0.95 of the way to its edges). Each vertex must read the level's flat +X
// colour; a face clamp taken at level 0 lets a high level's filter blend in
// the rows of -X, which the stacked cube keeps next to it.
precision highp float;
layout(location = 0) in vec4 a_pos;
uniform highp samplerCube u_s;
uniform float u_lod;
out vec4 v_col;
void main()
{
    gl_Position = vec4(a_pos.xy, 0.0, 1.0);
    v_col = textureLod(u_s, vec3(1.0, a_pos.y * 0.95, a_pos.x * 0.95), u_lod);
}
