# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pvr_rust_zephyr_application(<manifest.in> [ENV <NAME>=<value> ...])
#
# The same job as the Rust module's rust_cargo_application(), for an application
# whose Cargo.toml has to be generated: a UI application depends on crates that
# live outside this repository (the no_std Slint workspace and its femtovg/glow
# forks), and a path dependency cannot be written without knowing where they
# are. CMake substitutes the @VARIABLES@ of <manifest.in> into the build
# directory and cargo is pointed at that manifest.
#
# Everything else matches the module: the crate is named rustapp and built for
# the Zephyr target with the module's environment (bindgen against this build's
# headers), and the C main() that calls rust_main() comes from the module.
# ENV entries reach cargo (and the build scripts it runs) as environment
# variables, which is how a build.rs is told where a generated manifest's
# sources are.
#
# Include this file after find_package(Zephyr) and after the Rust module is in
# ZEPHYR_EXTRA_MODULES. It borrows these from that module, so a change to any of
# them upstream is what to look at if this stops working:
# _rust_map_target(), get_include_dirs(), add_cargo_target_with_zephyr_env()
# (which reads include_dirs, include_defines, rust_build_type_arg, RUST_TARGET,
# CARGO_TARGET_DIR and command_paths from its caller's scope), and the cache
# variables RUST_MODULE_DIR and DT_AUGMENTS.

function(pvr_rust_zephyr_application manifest_in)
  cmake_parse_arguments(RUSTAPP "" "" "ENV" ${ARGN})
  if(RUSTAPP_UNPARSED_ARGUMENTS)
    message(FATAL_ERROR
      "pvr_rust_zephyr_application: unexpected arguments: ${RUSTAPP_UNPARSED_ARGUMENTS}")
  endif()

  if(NOT CONFIG_RUST)
    message(FATAL_ERROR "pvr_rust_zephyr_application: CONFIG_RUST is not set")
  endif()
  if(NOT COMMAND add_cargo_target_with_zephyr_env OR NOT DEFINED CACHE{RUST_MODULE_DIR})
    message(FATAL_ERROR
      "pvr_rust_zephyr_application: the zephyr-lang-rust module is not in this "
      "build. Add it to ZEPHYR_EXTRA_MODULES before find_package(Zephyr).")
  endif()

  # What the zephyr-sys bindgen run needs to see (read by the module's
  # add_cargo_target_with_zephyr_env through the calling scope).
  get_include_dirs(zephyr_interface include_dirs)
  get_property(include_defines TARGET zephyr_interface
               PROPERTY INTERFACE_COMPILE_DEFINITIONS)

  _rust_map_target()   # -> RUST_TARGET

  if(CONFIG_DEBUG)
    set(RUST_BUILD_TYPE "debug")
    set(rust_build_type_arg "")
  else()
    set(RUST_BUILD_TYPE "release")
    set(rust_build_type_arg "--release")
  endif()

  set(CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/rust/target")
  set(RUST_LIBRARY "${CARGO_TARGET_DIR}/${RUST_TARGET}/${RUST_BUILD_TYPE}/librustapp.a")
  set(WRAPPER_FILE "${CMAKE_CURRENT_BINARY_DIR}/rust/wrapper.c")
  set(CARGO_MANIFEST "${CMAKE_CURRENT_BINARY_DIR}/rust/Cargo.toml")
  # An output that is never created, so cargo runs on every build; the library
  # is a byproduct (the same trick the module uses).
  set(DUMMY_FILE "${CMAKE_BINARY_DIR}/always-run-cargo.dummy")

  configure_file("${manifest_in}" "${CARGO_MANIFEST}" @ONLY)

  # The --config values below are quoted with backslashes because the build
  # recipe these arguments end up in is handed to a shell (ninja and make both
  # run custom commands through one), which strips one level of quoting.
  #
  # The zephyr crates are path-patched over their crates.io names.
  set(command_paths)
  foreach(module IN ITEMS zephyr zephyr-build zephyr-sys)
    list(APPEND command_paths
      "--config"
      "patch.crates-io.${module}.path=\\\"$CACHE{RUST_MODULE_DIR}/${module}\\\"")
  endforeach()

  # ENV NAME=value -> cargo's [env] table, which build scripts see too. The
  # value is quoted but not braced: the command runs through a shell, which
  # would treat {a,b} as a brace expansion and split the argument.
  foreach(entry IN LISTS RUSTAPP_ENV)
    string(FIND "${entry}" "=" eq)
    if(eq LESS 0)
      message(FATAL_ERROR "pvr_rust_zephyr_application: ENV entry '${entry}' is not NAME=value")
    endif()
    string(SUBSTRING "${entry}" 0 ${eq} name)
    math(EXPR after "${eq} + 1")
    string(SUBSTRING "${entry}" ${after} -1 value)
    list(APPEND command_paths
      "--config" "env.${name}=\\\"${value}\\\"")
  endforeach()

  add_cargo_target_with_zephyr_env(librustapp
    ALL
    OUTPUT ${DUMMY_FILE}
    BYPRODUCTS ${RUST_LIBRARY} ${WRAPPER_FILE}
    CARGO_ARGS build --manifest-path "${CARGO_MANIFEST}"
    COMMENT "Building the Rust application"
  )

  # Same link rules as the module: the runtime library before librustapp.a, and
  # the C main() plus the bindgen wrappers as the application's C sources.
  target_link_libraries(app PUBLIC $<TARGET_PROPERTY:linker,rt_library>
                        -Wl,--allow-multiple-definition ${RUST_LIBRARY})
  add_dependencies(app librustapp)
  target_sources(app PRIVATE $CACHE{RUST_MODULE_DIR}/main.c ${WRAPPER_FILE})
endfunction()
