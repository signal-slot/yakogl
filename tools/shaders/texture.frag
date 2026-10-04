#version 450
/* Texture-sample FS. NOTE: pco_shc does not yet wire Vulkan descriptor-set
 * samplers, so this entry uses the known-good hand-verified smp_tex_fs USC
 * (scratchpad/smp_fs_shader.md). tex/sampler state -> sh0..3 / sh4..7; the uv
 * varying (VAR0.xy) is iterated from the coefficient store. See the .prebuilt.json
 * sidecar and SHADERC_NOTES.md. */
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;
layout(set = 0, binding = 0) uniform sampler2D u_tex;
void main() { o_color = texture(u_tex, v_uv); }
