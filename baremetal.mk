# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# yakogl/baremetal.mk - plain cross-compile build of libyakogl.a with the
# bare-metal OSAL (no OS: direct MMIO, static carveout, spin-wait).
#
# The archive is the Rust archive crate (capi/: the yakogl crate + panic
# handler), post-processed by capi/assemble.cmake (Rust runtime and
# compiler_builtins' libc/libm symbols made local), with the C parts
# appended: the bare-metal OSAL, the layout checks and the driver's OS-free
# command generators.
#
# Usage:
#   make -f yakogl/baremetal.mk CROSS_COMPILE=aarch64-none-elf- \
#        [GPU=powervr|vivante] \
#        [BUILD=build/bm] [RUST_TARGET=aarch64-unknown-none] \
#        [RUST_TARGET_CPU=cortex-a53] [CARGO_PROFILE=release|dev] \
#        [CARGO=cargo] [RUSTUP_TOOLCHAIN=...] [BUILD_STD=1] \
#        [RUST_LINKAGE=staticlib|external] \
#        [BOARD_CFLAGS='...']
# The board's register base, carveout and hooks are not build options: the
# application registers them at run time (include/gles_osal_baremetal.h,
# osal_bm_board_config()) and init refuses without them.
#   clang: CC='clang --target=aarch64-none-elf' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy
#
# Needs: cargo and the Rust target (rustup target add aarch64-unknown-none),
# or a nightly toolchain with rust-src and BUILD_STD=1; cmake (for
# capi/assemble.cmake); objcopy/nm for the target (the cross binutils or llvm).
# Output: $(BUILD)/libyakogl.a, or with RUST_LINKAGE=external only
# $(BUILD)/libyakogl_c.a (the C parts; the application's own Rust staticlib
# links the yakogl crate, README.md "Linking").
#
# The application provides (a libc such as newlib or picolibc): abort,
# malloc, free, memcpy, memmove, memset, memcmp, strlen, snprintf, and
# vprintf for the bare-metal OSAL's log; float printf only for the backend's
# debug attribute dump. No libm.
#
# Incremental: cargo runs only when a crate source (cargo's dep-info), a
# manifest or this makefile changed, and the archive is re-assembled only
# when cargo produced a new staticlib or a C object changed.

CROSS_COMPILE ?= aarch64-none-elf-
CC      := $(CROSS_COMPILE)gcc
AR      := $(CROSS_COMPILE)ar
NM      ?= $(CROSS_COMPILE)nm
OBJCOPY ?= $(CROSS_COMPILE)objcopy
CMAKE   ?= cmake
CARGO   ?= cargo
RUST_TARGET ?= aarch64-unknown-none
RUST_TARGET_CPU ?=
CARGO_PROFILE ?= release
RUST_LINKAGE ?= staticlib
# The GPU whose backend and offline shader table are compiled in.
GPU ?= powervr
BUILD_STD ?=

THIS_MK := $(abspath $(lastword $(MAKEFILE_LIST)))
HERE    := $(patsubst %/,%,$(dir $(THIS_MK)))
SRC     := $(HERE)/src
INCLUDE := $(HERE)/include
BUILD   ?= build/bm
BUILD_ABS := $(abspath $(BUILD))

# ---- C sources ---------------------------------------------------------------
# powervr-bxs-am62p/src (this repository is its yakogl/ submodule); override for other layouts
DRVSRC  ?= $(HERE)/../src
SRCS    := $(SRC)/osal/baremetal.c $(SRC)/abi_layout_check.c
# The PowerVR backend calls the driver's OS-free command generators through
# FFI, and pins their structs; the GC7000 has no such driver and writes its own
# command stream, so a vivante build compiles neither.
ifeq ($(GPU),powervr)
SRCS    += $(SRC)/pvr/ffi_layout_check.c \
           $(DRVSRC)/pvr_vdm_ppp_gen.c $(DRVSRC)/pvr_pds_render_gen.c \
           $(DRVSRC)/pvr_pds_pixel_event_gen.c \
           $(HERE)/driver/compat/pvr_mesa_pds.c
# The generators call Mesa 26.2.1's PDS generators and csbgen packers
# (driver/compat/pvr_mesa.h; the same directories as driver/compat/pvr_mesa.cmake).
MESA    := $(HERE)/third_party/mesa-26.2.1
PVR_MESA_CFLAGS := -I$(HERE)/driver/compat -I$(HERE)/driver/src/pvr/generated \
           -I$(MESA)/include -I$(MESA)/src -I$(MESA)/src/imagination \
           -I$(MESA)/src/imagination/include -I$(MESA)/src/imagination/common \
           -I$(MESA)/src/imagination/vulkan -I$(MESA)/src/imagination/vulkan/pds \
           -I$(MESA)/src/imagination/vulkan/pds/pvr_pds_programs
GPU_DEF := YAKOGL_GPU_POWERVR
CARGO_FEATURES := osal-baremetal,gpu-powervr
else ifeq ($(GPU),vivante)
GPU_DEF := YAKOGL_GPU_VIVANTE
CARGO_FEATURES := osal-baremetal,gpu-vivante
else
$(error GPU must be powervr or vivante)
endif
# REPORT_ES2=1 (the default): report OpenGL ES 2.0; REPORT_ES2=0 reports
# ES 3.0 (see Cargo.toml).
REPORT_ES2 ?= 1
ifeq ($(REPORT_ES2),1)
CARGO_FEATURES := $(CARGO_FEATURES),report-es2
endif
OBJS    := $(patsubst %.c,$(BUILD_ABS)/%.o,$(notdir $(SRCS)))

# resolve object -> source across the several source dirs
VPATH := $(SRC):$(SRC)/osal:$(SRC)/pvr:$(DRVSRC):$(HERE)/driver/compat

# ---- flags -------------------------------------------------------------------
CFLAGS ?= -O2 -g
CFLAGS += -std=c11 -ffreestanding -fno-common -Wall -Wextra
# src/vivante is for gc_kick.h, the C view of the Vivante submit descriptor
CFLAGS += -I$(INCLUDE) -I$(SRC) -I$(SRC)/osal -I$(SRC)/vivante -I$(DRVSRC)
CFLAGS += $(PVR_MESA_CFLAGS)
CFLAGS += -DYAKOGL_OSAL_BAREMETAL -D$(GPU_DEF)
CFLAGS += $(BOARD_CFLAGS)

ifeq ($(CARGO_PROFILE),release)
CARGO_FLAGS := --release
CARGO_OUT   := release
else ifeq ($(CARGO_PROFILE),dev)
# one LTO object with dev semantics (capi/assemble.cmake needs LTO)
CARGO_FLAGS := --profile dev-lto
CARGO_OUT   := dev-lto
else
$(error CARGO_PROFILE must be release or dev)
endif
ifneq ($(BUILD_STD),)
CARGO_FLAGS += -Z build-std=core
endif
ifneq ($(RUST_TARGET_CPU),)
CARGO_FLAGS += --config 'target.$(RUST_TARGET).rustflags=["-Ctarget-cpu=$(RUST_TARGET_CPU)"]'
endif
CARGO_ENV := $(if $(RUSTUP_TOOLCHAIN),RUSTUP_TOOLCHAIN=$(RUSTUP_TOOLCHAIN))
# The C the crate compiles itself (build.rs: the PowerVR backend's csbgen
# packer wrappers; Mesa's QPU packer for gpu-v3d) goes through the same
# compiler as the C here: the `cc` crate reads CC_<target>/AR_<target>.
RUST_TARGET_US := $(subst -,_,$(RUST_TARGET))
CARGO_ENV += CC_$(RUST_TARGET_US)='$(CC)' AR_$(RUST_TARGET_US)='$(AR)'

empty :=
space := $(empty) $(empty)

RUST_LIB := $(BUILD_ABS)/cargo/$(RUST_TARGET)/$(CARGO_OUT)/libyakogl_capi.a

ifeq ($(RUST_LINKAGE),staticlib)
TARGET := $(BUILD_ABS)/libyakogl.a
else ifeq ($(RUST_LINKAGE),external)
TARGET := $(BUILD_ABS)/libyakogl_c.a
else
$(error RUST_LINKAGE must be staticlib or external)
endif

.PHONY: all clean
all: $(TARGET)

# cargo runs when the stamp is older than a crate source (cargo's dep-info,
# re-targeted at the stamp), a manifest or this makefile. The staticlib itself
# only changes when cargo rebuilt it, so the archive is not re-assembled after
# a cargo run that found everything fresh.
RUST_STAMP := $(BUILD_ABS)/cargo/yakogl_capi.stamp
-include $(RUST_STAMP).d

$(RUST_STAMP): $(HERE)/Cargo.toml $(HERE)/capi/Cargo.toml $(THIS_MK)
	cd $(HERE) && $(CARGO_ENV) $(CARGO) build -p yakogl-capi --target-dir $(BUILD_ABS)/cargo \
		--target $(RUST_TARGET) $(CARGO_FLAGS) \
		--no-default-features --features $(CARGO_FEATURES)
	sed -e '1s|^[^:]*:|$(RUST_STAMP):|' $(RUST_LIB:.a=.d) > $@.d
	touch $@

$(RUST_LIB): $(RUST_STAMP) ;

$(BUILD_ABS)/libyakogl.a: $(RUST_LIB) $(OBJS) $(HERE)/capi/assemble.cmake
	$(CMAKE) -DRUST_LIB=$(RUST_LIB) -DOUTPUT=$@ -DAR=$(AR) -DNM=$(NM) -DOBJCOPY=$(OBJCOPY) \
		"-DC_OBJECTS=$(subst $(space),|,$(OBJS))" -P $(HERE)/capi/assemble.cmake

$(BUILD_ABS)/libyakogl_c.a: $(OBJS)
	rm -f $@
	$(AR) qs $@ $(OBJS)

$(BUILD_ABS)/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -rf $(BUILD)
