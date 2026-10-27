/* Selector population: Xy01 position, a vec4 varying of another attribute (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; VsVarying comps 4). */
attribute vec2 pos; attribute vec4 c; varying vec4 v; void main(){ gl_Position = vec4(pos, 0.0, 1.0); v = c; }
