# Host tests

The GL core, the PowerVR backend and the shader table all build for a plain
Linux host (`-DYAKOGL_OSAL=linux`). Nothing is rendered there, so only API
semantics can be tested — which is what these cover. Rendering behaviour is
tested on silicon, by `tools/pvrlab/gl_test.c` in the driver repository.

## negative_api

`negative_api` runs dEQP-GLES2 `functional.negative_api` (246 cases in the AOSP
mustpass). The case bodies are not rewritten: `negative_api/extract.py` copies
them out of the conformance suite's own sources, and `negative_api/deqp_shim.hpp`
supplies the few dEQP helpers they call. What runs here is what the suite runs.

```sh
cmake -S . -B build/host -DYAKOGL_OSAL=linux -DYAKOGL_SHADERC=offline \
      -DYAKOGL_TESTS=ON -DYAKOGL_CTS_DIR=/path/to/VK-GL-CTS
cmake --build build/host --target negative_api
./build/host/tests/negative_api            # all cases
./build/host/tests/negative_api texture.   # one group
```

The exit status is 0 when every selected case passes.
