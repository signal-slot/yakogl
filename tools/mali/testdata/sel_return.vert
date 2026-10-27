/* Selector population: tri's position with a trailing return (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; src/shaderc/online/mali/select.rs, the trailing Return node). */
attribute vec2 pos; void main(){ gl_Position = vec4(pos, 0.0, 1.0); return; }
