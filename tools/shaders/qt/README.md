# Qt Quick shaders

The GLSL ES 1.00 sources Qt 6's GLES2 RHI backend passes to `glShaderSource`,
extracted byte for byte from the `.qsb` shader packs of a Qt-for-Zephyr Stage 1
build (`qt6-for-zephyr/build-qt-zephyr-am62p.sh`) with
`qt6-for-zephyr/zephyr-module/tools/qsb2glsl`:

```sh
qsb2glsl -o yakogl/tools/shaders/qt build-qt-zephyr-am62p/qtdeclarative
tools/glsl2usc.sh        # regenerates src/shaderc/blobs.rs with them
```

`<name>.vert` / `<name>.frag` are the Standard variants, `<name>.batchable.vert`
the batchable vertex variant the scene graph's batch renderer uses for merged
batches.  Qt writes each uniform block as `struct buf { ... }; uniform buf _16;`
and addresses members as `_16.qt_Matrix`; `yakogl-tools glsl2usc` flattens those into
the push-constant block and reports the dotted names to `glGetUniformLocation`.
