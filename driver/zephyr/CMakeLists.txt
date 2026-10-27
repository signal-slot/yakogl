# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# zephyr/CMakeLists.txt - the PowerVR BXS-4-64 driver as a Zephyr library:
# the device (src/pvr_bringup.c), firmware boot, the render service and the
# OS-free command generators YakoGL's backend reuses. CONFIG_YAKOGL_DRV_GPU_PVR
# is selected when a device tree has an enabled "img,img-rogue" node.

if(CONFIG_YAKOGL_DRV_GPU_PVR)
  set(PVR_SRC "${CMAKE_CURRENT_SOURCE_DIR}/../src")

  zephyr_library_named(yakogl_drv)
  file(GLOB PVR_DRIVER_SRCS CONFIGURE_DEPENDS "${PVR_SRC}/*.c")
  zephyr_library_sources(${PVR_DRIVER_SRCS})
  # The command generators call Mesa 26.2.1's PDS generators and csbgen
  # packers (third_party/mesa-26.2.1, unmodified, and the csbgen headers
  # generated from it in src/pvr/generated): compiled here, for this library
  # only (driver/compat/pvr_mesa.h).
  include("${CMAKE_CURRENT_SOURCE_DIR}/../compat/pvr_mesa.cmake")
  zephyr_library_sources(${PVR_MESA_SOURCES})
  zephyr_library_include_directories(${PVR_MESA_INCLUDE_DIRS})
  # Compile-time only: the FWIF numbers the driver uses, asserted against
  # the vendored DDK headers by the target compiler.
  zephyr_library_sources("${CMAKE_CURRENT_SOURCE_DIR}/../compat/pvr_ddk_check.c")
  # The DDK's firmware start/stop and image loader, compiled unmodified
  # (driver/compat/pvr_rgxstartstop.c, pvr_rgxfwimageutils.c); the rgxlayer.h
  # functions they call are the yakogl-drivers crate's (feature pvr, below:
  # YAKOGL_DRV_GPU_PVR selects YAKOGL_DRV_GPU_PVR_RUST) except the two variadic log
  # functions (pvr_rgxlayer_log.c); pvr_rgxlayer_check.c pins the DDK side
  # of that layer (compile-time only).
  zephyr_library_sources(
    "${CMAKE_CURRENT_SOURCE_DIR}/../compat/pvr_rgxstartstop.c"
    "${CMAKE_CURRENT_SOURCE_DIR}/../compat/pvr_rgxfwimageutils.c"
    "${CMAKE_CURRENT_SOURCE_DIR}/../compat/pvr_rgxlayer_log.c"
    "${CMAKE_CURRENT_SOURCE_DIR}/../compat/pvr_rgxlayer_check.c")
  zephyr_include_directories("${PVR_SRC}")
  # The TI/Img DDK 25.2 firmware-interface headers, vendored unmodified, and
  # the build configuration the firmware matches; driver/compat/pvr_ddk.h is
  # the one place that includes them.
  set(PVR_DDK "${CMAKE_CURRENT_SOURCE_DIR}/../../third_party/ti-img-rogue-ddk-25.2")
  zephyr_include_directories(
    "${CMAKE_CURRENT_SOURCE_DIR}/../compat"
    "${PVR_DDK}/config/am62p_linux_release"
    "${PVR_DDK}/include"
    "${PVR_DDK}/include/rogue"
    "${PVR_DDK}/include/public"
    "${PVR_DDK}/hwdefs/rogue"
    "${PVR_DDK}/hwdefs/rogue/km"
    "${PVR_DDK}/services/include"
    "${PVR_DDK}/services/server/include"
    "${PVR_DDK}/services/server/devices")

  # The freelist sizing every silicon run used.
  zephyr_library_compile_definitions(PVR_BIG_GLOBAL_FL)
  # The driver runs the TI/Img DDK 25.2 firmware (its ABI is the vendored
  # headers'). The firmware image itself is embedded from src/fw/pvr_fw_blob.h,
  # generated locally by tools/mkfwblob.sh from the board's
  # rgx.fw.36.53.104.796 (never committed).
  zephyr_library_compile_definitions(PVR_FW_PM_LATENCY_MS=${CONFIG_YAKOGL_DRV_GPU_PVR_PM_LATENCY_MS})
  if(CONFIG_YAKOGL_DRV_GPU_PVR_RENDER_VERBOSE)
    zephyr_library_compile_definitions(PVR_RENDER_VERBOSE)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FL_GROW_ON_DEMAND)
    zephyr_library_compile_definitions(PVR_FL_GROW_ON_DEMAND_DEFAULT=1)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FL_READY_PAGES)
    zephyr_library_compile_definitions(PVR_FL_READY_PAGES_DEFAULT=1)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER)
    zephyr_library_compile_definitions(PVR_PARTIAL_RENDER_DEFAULT=1)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES GREATER 0)
    zephyr_library_compile_definitions(PVR_FL_PROBE_MAX_PAGES=${CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES})
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE)
    zephyr_library_compile_definitions(PVR_FL_PROBE_NO_RESERVE)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FWCCB_CHECK)
    zephyr_library_compile_definitions(PVR_FWCCB_CHECK_DEFAULT=1)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_SERVE_MAX GREATER 0)
    zephyr_library_compile_definitions(PVR_FL_PROBE_SERVE_MAX=${CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_SERVE_MAX})
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FW_UNCACHED)
    zephyr_library_compile_definitions(PVR_FW_UNCACHED_DEFAULT=1)
  endif()
  if(NOT CONFIG_YAKOGL_DRV_GPU_PVR_SELFTEST)
    zephyr_library_compile_definitions(PVR_NO_SELFTEST_RENDER)
  endif()
endif()

# The Vivante GC7000 driver for the i.MX8MP: selected when a device tree has
# an enabled "vivante,gc" node. The kernel side (power, clocks, identity,
# reset, MMU, ring, interrupt) is driver/rust's gpu::vivante (feature
# `vivante`) behind the Zephyr shim gpu_vivante_gc_rust.c; src/vivante/ holds
# the composition code (gc_render.c, gc_draw.c, gc_cmd.c). See
# driver/docs/vivante-kernel-rust.md.
if(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
  set(GC_SRC "${CMAKE_CURRENT_SOURCE_DIR}/../src/vivante")
  zephyr_library_named(vivante_gc)
  file(GLOB GC_DRIVER_SRCS CONFIGURE_DEPENDS "${GC_SRC}/*.c")
  zephyr_library_sources("${CMAKE_CURRENT_SOURCE_DIR}/../drivers/gpu/gpu_vivante_gc_rust.c")
  zephyr_library_include_directories("${CMAKE_CURRENT_SOURCE_DIR}/../rust/include")
  zephyr_library_sources(${GC_DRIVER_SRCS})
  zephyr_include_directories("${GC_SRC}")
endif()

# The i.MX8MP display pipeline (src/display/): LCDIFv3, MIPI DSIM + D-PHY;
# the media power domains (yakogl,imx8mp-media-pd), the SN65DSI8x LVDS bridge
# (yakogl,sn65dsi8x) and the backlight (yakogl,imx-pwm) are devices of their
# own (below).
# Selected when a device tree has an enabled "yakogl,imx8mp-display" node.
if(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP)
  set(DISP_SRC "${CMAKE_CURRENT_SOURCE_DIR}/../src/display")
  zephyr_include_directories("${DISP_SRC}")
  if(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_HDMI)
    # The HDMI chain from cold (yakogl,imx8mp-hdmi): the imx8mp_display_*
    # API is its shim's, built with the Rust drivers below; nothing here.
  elseif(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST AND NOT CONFIG_DT_HAS_YAKOGL_IMX8MP_DISPLAY_ENABLED)
    # No chain to own: the adopt-only yakogl,imx8mp-lcdif device (below)
    # provides the imx8mp_display_* API; nothing here.
  else()
    zephyr_library_named(imx8mp_display)
    if(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_ADOPT)
      # An LCDIF the previous kernel left running, adopted as is: the layer
      # helpers and the adopt front end, nothing that owns the chain.
      zephyr_library_sources("${DISP_SRC}/imx8mp_lcdif.c" "${DISP_SRC}/imx8mp_lcdif_adopt.c")
    else()
      file(GLOB DISP_SRCS CONFIGURE_DEPENDS "${DISP_SRC}/*.c")
      list(REMOVE_ITEM DISP_SRCS "${DISP_SRC}/imx8mp_lcdif_adopt.c"
        "${DISP_SRC}/imx8mp_lcdif_rust.c" "${DISP_SRC}/imx8mp_dsim_rust.c")
      # The LCDIF and the DSIM over the Rust devices (group E steps 3, 5;
      # driver/docs/imx8mp-display-cold.md), each behind its own switch.
      if(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST)
        list(REMOVE_ITEM DISP_SRCS "${DISP_SRC}/imx8mp_lcdif.c")
        list(APPEND DISP_SRCS "${DISP_SRC}/imx8mp_lcdif_rust.c")
      endif()
      if(CONFIG_YAKOGL_DRV_DSIM_RUST)
        list(REMOVE_ITEM DISP_SRCS "${DISP_SRC}/imx8mp_dsim.c")
        list(APPEND DISP_SRCS "${DISP_SRC}/imx8mp_dsim_rust.c")
      endif()
      zephyr_library_sources(${DISP_SRCS})
    endif()
  endif()
endif()

# Drivers whose register logic is Rust (docs/source-form.md, group E): the
# yakogl-drivers crate, built into libyakogl_drivers.a and post-processed like
# libyakogl.a (../../capi/assemble.cmake makes its Rust runtime and
# compiler_builtins' libc names local, so it links beside libyakogl.a and an
# application's librustapp.a), plus the C shims that register its devices.
# CONFIG_YAKOGL_DRV_DISPLAY_RUST is selected by each such display/SoC driver,
# CONFIG_YAKOGL_DRV_GPU_PVR_RUST by the PowerVR driver (its rgxlayer, feature
# `pvr`), CONFIG_YAKOGL_BOARD_RECORD_RUST by the board layer's record writer
# (board::record, no feature: any SoC); any of them builds the crate, and the
# PowerVR driver library links it.
#
# The crate's location is named in exactly two places, so moving it is two
# edits: YAKOGL_DRIVERS_CRATE_DIR here and its `members` entry in the
# repository's Cargo.toml.
#
#   YAKOGL_DRIVERS_RUST_TARGET       aarch64-unknown-none-softfloat: no FP/SIMD
#                                    registers, because the code runs at
#                                    PRE_KERNEL_1 and in ISRs.
#   YAKOGL_DRIVERS_RUSTUP_TOOLCHAIN  RUSTUP_TOOLCHAIN for this crate (default:
#                                    YAKOGL_RUSTUP_TOOLCHAIN). The toolchain
#                                    needs the target's core (`rustup target add
#                                    aarch64-unknown-none-softfloat`) or, with
#                                    nightly + rust-src, builds it
#                                    (YAKOGL_CARGO_BUILD_STD=AUTO).
set(YAKOGL_DRIVERS_CRATE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../rust")
if(CONFIG_YAKOGL_DRV_DISPLAY_RUST OR CONFIG_YAKOGL_DRV_GPU_PVR_RUST OR CONFIG_YAKOGL_BOARD_RECORD_RUST)
  if(NOT CONFIG_ARM64)
    message(FATAL_ERROR "yakogl-drivers: libyakogl_drivers.a is built for aarch64 only")
  endif()
  include("${CMAKE_CURRENT_SOURCE_DIR}/../../capi/yakogl_capi.cmake")
  set(YAKOGL_DRIVERS_RUST_TARGET "aarch64-unknown-none-softfloat" CACHE STRING
    "Rust target of libyakogl_drivers.a")
  set(YAKOGL_DRIVERS_RUSTUP_TOOLCHAIN "${YAKOGL_RUSTUP_TOOLCHAIN}" CACHE STRING
    "RUSTUP_TOOLCHAIN for the yakogl-drivers crate (empty: inherited)")

  # The crate's SoC features, from the drivers this image has.
  set(_yd_features "")
  if(CONFIG_YAKOGL_DRV_PWM_IMX)
    list(APPEND _yd_features imx8mp)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_RUST)
    list(APPEND _yd_features pvr)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT)
    list(APPEND _yd_features pvr-fine-wait)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES)
    list(APPEND _yd_features pvr-kick-times)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_PVR_GROW_SCENE)
    list(APPEND _yd_features pvr-grow-scene)
  endif()
  if(CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD)
    list(APPEND _yd_features imx8mp)
  endif()
  if(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_HDMI OR CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST OR
     CONFIG_YAKOGL_DRV_DSIM_RUST)
    list(APPEND _yd_features imx8mp)
  endif()
  if(CONFIG_YAKOGL_DRV_DSS_AM62P OR CONFIG_YAKOGL_DRV_DSI_CDNS_J721E)
    list(APPEND _yd_features am62p)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
    list(APPEND _yd_features vivante)
  endif()
  if(CONFIG_GC_COMMON_PANEL_CONSOLE)
    # soc::imx8mp::media::lcdif_unreachable behind lib/board/imx8mp/gc_panel_console.c
    list(APPEND _yd_features imx8mp)
  endif()
  if(CONFIG_GC_BOARD_WDOG_SELF_ARM)
    # ip::imx_wdog behind lib/board/imx8mp/gc_wdog_arm.c
    list(APPEND _yd_features imx8mp)
  endif()
  if(CONFIG_YAKOGL_NANOPC_T6)
    # ip::dw_wdt, soc::rk3588 behind lib/board/nanopc_t6/nanopc_t6.c; the
    # GPU power-on (board::nanopc_t6_gpu, gpu::mali) behind samples' calls of
    # yd_t6_gpu_up
    list(APPEND _yd_features rk3588 mali)
  endif()
  list(REMOVE_DUPLICATES _yd_features)

  set(_yd_archive "${CMAKE_CURRENT_BINARY_DIR}/libyakogl_drivers.a")
  yakogl_add_rust_archive(
    CRATE yakogl-drivers
    MANIFEST "${YAKOGL_DRIVERS_CRATE_DIR}/Cargo.toml"
    LIB_NAME yakogl_drivers
    FEATURES ${_yd_features}
    TARGET "${YAKOGL_DRIVERS_RUST_TARGET}"
    RUSTUP_TOOLCHAIN "${YAKOGL_DRIVERS_RUSTUP_TOOLCHAIN}"
    CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-drivers"
    OUTPUT "${_yd_archive}"
    TARGET_NAME yakogl_drivers_rust)
  add_library(yakogl_drivers_rust_lib STATIC IMPORTED GLOBAL)
  set_target_properties(yakogl_drivers_rust_lib PROPERTIES IMPORTED_LOCATION "${_yd_archive}")
  add_dependencies(yakogl_drivers_rust_lib yakogl_drivers_rust)
  zephyr_link_libraries(yakogl_drivers_rust_lib)
  if(CONFIG_YAKOGL_DRV_GPU_PVR_RUST)
    # The driver library calls the crate's rgxlayer: link it there too, so the
    # archive follows libyakogl_drv.a on the link line.
    target_link_libraries(yakogl_drv PUBLIC yakogl_drivers_rust_lib)
  endif()
  if(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
    # The same for the Vivante driver library's kernel side (gpu::vivante).
    target_link_libraries(vivante_gc PUBLIC yakogl_drivers_rust_lib)
  endif()

  # The shims: registration only (devices, API tables, register mappings,
  # kernel objects) and the struct yd_env every Rust entry point takes.
  zephyr_library_named(yakogl_drivers_shims)
  zephyr_library_sources("${CMAKE_CURRENT_SOURCE_DIR}/../drivers/common/yd_zephyr.c")
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_PWM_IMX
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/pwm/pwm_yakogl_imx.c")
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_SN65DSI8X
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/display/bridge_sn65dsi8x.c")
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/power_domain/power_domain_yakogl_imx8mp_media.c")
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_HDMI
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/display/display_yakogl_imx8mp_hdmi.c")
  # The i.MX8MP LCDIFv3 and Samsung DSIM (group E steps 3, 5); the C files in
  # src/display stay the default until these are board-proven.
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/display/display_yakogl_imx8mp_lcdif.c")
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_DSIM_RUST
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/mipi_dsi/dsi_yakogl_samsung.c")
  # The AM62P display path (its register logic in Rust).
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_DSS_AM62P
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/display/display_am62p_dss_rust.c")
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_DSI_CDNS_J721E
    "${CMAKE_CURRENT_SOURCE_DIR}/../drivers/mipi_dsi/dsi_cdns_rust.c")
  zephyr_include_directories("${YAKOGL_DRIVERS_CRATE_DIR}/include")
  add_dependencies(yakogl_drivers_shims yakogl_drivers_rust)
  message(STATUS "yakogl-drivers: features '${_yd_features}', target ${YAKOGL_DRIVERS_RUST_TARGET}")
endif()

# The board layer (lib/board, driver/docs/one-method.md): yakogl_board.h and
# one implementation per board, plus the per-board support it is built from
# (the i.MX8MP's yakogl_gc_board library, which applications outside this
# repository link too; the AM62P's RTI self-arm).
add_subdirectory(lib/board)

# Verdin AM62P Cortex-A53 display path (DSS1 -> Cadence DSI -> SN65DSI83 ->
# LVDS panel; the bridge is yakogl,sn65dsi8x, above) and the TISCI helpers it uses, from ../drivers and ../lib (split
# out of the platform module, which compiled the same files under the same
# symbols). Their headers are <yakogl_drv/*.h> under ../include, which the samples
# also include (yakogl_drv/am62p_dss.h).
zephyr_include_directories(${CMAKE_CURRENT_SOURCE_DIR}/../include)

# Zephyr's TI secure proxy mailbox driver with the order of a message's
# trailing bytes fixed (YAKOGL_DRV_MBOX_TI_SECURE_PROXY_TAIL_FIX, Kconfig.am62p):
# the upstream source with the two lines of its tail loop replaced, compiled
# in Zephyr's mbox library instead of the original, so it holds whatever an
# application's configuration says. The build stops if those lines are not
# there any more: a Zephyr update that changes or fixes the driver is noticed
# and this can be dropped.
if(CONFIG_YAKOGL_DRV_MBOX_TI_SECURE_PROXY_TAIL_FIX)
  set(secproxy_src ${ZEPHYR_BASE}/drivers/mbox/mbox_ti_secproxy.c)
  set(secproxy_out ${CMAKE_CURRENT_BINARY_DIR}/mbox_ti_secproxy_le_tail.c)
  file(READ ${secproxy_src} secproxy_text)
  set(secproxy_shift "data_trail <<= 8;")
  set(secproxy_or "data_trail |= byte_data[i];")
  string(FIND "${secproxy_text}" "${secproxy_shift}" secproxy_at_shift)
  string(FIND "${secproxy_text}" "${secproxy_or}" secproxy_at_or)
  get_target_property(secproxy_lib_srcs drivers__mbox SOURCES)
  list(FILTER secproxy_lib_srcs EXCLUDE REGEX "mbox_ti_secproxy\\.c$")
  get_target_property(secproxy_lib_srcs_all drivers__mbox SOURCES)
  list(LENGTH secproxy_lib_srcs secproxy_n_kept)
  list(LENGTH secproxy_lib_srcs_all secproxy_n_all)
  math(EXPR secproxy_n_removed "${secproxy_n_all} - ${secproxy_n_kept}")
  if(secproxy_at_shift EQUAL -1 OR secproxy_at_or EQUAL -1 OR NOT secproxy_n_removed EQUAL 1)
    message(FATAL_ERROR "YAKOGL_DRV_MBOX_TI_SECURE_PROXY_TAIL_FIX: ${secproxy_src} or its "
      "place in the drivers__mbox library has changed; check whether Zephyr fixed the "
      "byte order of the trailing bytes and drop this workaround if so")
  endif()
  string(REPLACE "${secproxy_shift}" "/* little endian, like the full words */" secproxy_text "${secproxy_text}")
  string(REPLACE "${secproxy_or}" "data_trail |= (uint32_t)byte_data[i] << (8 * i);" secproxy_text "${secproxy_text}")
  file(WRITE ${secproxy_out}.tmp "${secproxy_text}")
  configure_file(${secproxy_out}.tmp ${secproxy_out} COPYONLY) # rewritten only when it changes
  set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS ${secproxy_src})
  set_property(TARGET drivers__mbox PROPERTY SOURCES ${secproxy_lib_srcs} ${secproxy_out})
endif()

if(CONFIG_YAKOGL_DRV_TISCI_ZEPHYR)
  zephyr_library_named(yakogl_drv_drivers)
  zephyr_library_sources(${CMAKE_CURRENT_SOURCE_DIR}/../lib/tisci_zephyr/tisci_zephyr.c)
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_DRV_TISCI_POWER_ON ${CMAKE_CURRENT_SOURCE_DIR}/../drivers/firmware/tisci_power_on.c)
endif()
