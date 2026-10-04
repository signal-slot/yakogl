# Qt Quick 3D shaders

The GLSL ES 1.00 sources Qt Quick 3D's GLES 2.0 path hands to `glShaderSource`,
for YakoGL's offline table (`tools/glsl2usc.sh` compiles this directory like
`shaders/qt`, opaque plus the ONE/ONE_MINUS_SRC_ALPHA variant):

- `<name>.vert|frag` from the runtime renderer's built-in `.qsb` packs of a
  Stage 1 build (`qt6-for-zephyr/build-qt-zephyr-am62p.sh` with
  `QT_SUBMODULES` including qtquick3d), extracted with
  `qt6-for-zephyr/zephyr-module/tools/qsb2glsl`. The ES 3.0-only built-ins
  (clear/MRT, motion vectors, lightmap baking, environment prefiltering) have
  no 1.00 text and are absent.
- `qtappshaders_N.vert|frag` from an application's shadergen `.qsbc`
  collection (`qsb2glsl` reads it); the Stage 2 wrapper runs shadergen for
  every Quick 3D app.
- `<sha256 prefix>.vert|frag` harvested from the board console with
  `zephyr-module/tools/harvest-shaders.py`: variants Quick 3D generates at
  run time that shadergen did not foresee (a material whose texture map is
  set dynamically).

`qtparticleshaders_*`: Qt Quick 3D's particle system shaders are not valid
GLSL ES 1.00 (`flat` varyings for integer interpolants, `vec2[](...)` array
constructors); no OpenGL ES 2.0 driver can compile them, so glsl2usc skips
them (`--skip-failures`). Particles3D is out of scope on this path.
