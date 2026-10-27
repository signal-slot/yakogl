// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * Prints the sizes, offsets and ioctl numbers that tools/mali/panthor/src/uapi.rs
 * pins in its tests, from the same two headers the structures were transcribed
 * from (Linux v6.18 include/uapi/drm/panthor_drm.h and the DRM core drm.h).
 * One line per pin, in the order of the tests:
 *
 *   size   <name> <sizeof>
 *   offset <struct>.<field> <offsetof>
 *   ioctl  <name> <0x...>
 *
 * uapi_pins.sh builds this for aarch64, runs it under qemu-aarch64 and diffs
 * the output against uapi_pins.expected, which the Rust tests include.
 */
#include <stddef.h>
#include <stdio.h>
#include <drm.h>
#include <panthor_drm.h>

#define SIZE(n, t) printf("size   %s %zu\n", n, sizeof(t))
#define OFFSET(t, f) printf("offset %s.%s %zu\n", #t, #f, offsetof(t, f))
#define IOCTL(n, v) printf("ioctl  %s %#010x\n", n, (unsigned)(v))

int main(void)
{
	SIZE("drm_version", struct drm_version);
	SIZE("drm_gem_close", struct drm_gem_close);
	SIZE("drm_syncobj_create", struct drm_syncobj_create);
	SIZE("drm_syncobj_destroy", struct drm_syncobj_destroy);
	SIZE("drm_syncobj_wait", struct drm_syncobj_wait);
	SIZE("obj_array", struct drm_panthor_obj_array);
	SIZE("sync_op", struct drm_panthor_sync_op);
	SIZE("gpu_info", struct drm_panthor_gpu_info);
	SIZE("csif_info", struct drm_panthor_csif_info);
	SIZE("timestamp_info", struct drm_panthor_timestamp_info);
	SIZE("group_priorities_info", struct drm_panthor_group_priorities_info);
	SIZE("dev_query", struct drm_panthor_dev_query);
	SIZE("vm_create", struct drm_panthor_vm_create);
	SIZE("vm_destroy", struct drm_panthor_vm_destroy);
	SIZE("vm_bind_op", struct drm_panthor_vm_bind_op);
	SIZE("vm_bind", struct drm_panthor_vm_bind);
	SIZE("vm_get_state", struct drm_panthor_vm_get_state);
	SIZE("bo_create", struct drm_panthor_bo_create);
	SIZE("bo_mmap_offset", struct drm_panthor_bo_mmap_offset);
	SIZE("queue_create", struct drm_panthor_queue_create);
	SIZE("group_create", struct drm_panthor_group_create);
	SIZE("group_destroy", struct drm_panthor_group_destroy);
	SIZE("queue_submit", struct drm_panthor_queue_submit);
	SIZE("group_submit", struct drm_panthor_group_submit);
	SIZE("group_get_state", struct drm_panthor_group_get_state);
	SIZE("tiler_heap_create", struct drm_panthor_tiler_heap_create);
	SIZE("tiler_heap_destroy", struct drm_panthor_tiler_heap_destroy);
	OFFSET(struct drm_panthor_gpu_info, shader_present);
	OFFSET(struct drm_panthor_gpu_info, gpu_features);
	OFFSET(struct drm_panthor_group_create, compute_core_mask);
	OFFSET(struct drm_panthor_group_create, vm_id);
	OFFSET(struct drm_panthor_tiler_heap_create, tiler_heap_ctx_gpu_va);
	OFFSET(struct drm_panthor_vm_bind_op, syncs);
	OFFSET(struct drm_panthor_queue_submit, syncs);
	IOCTL("VERSION", DRM_IOCTL_VERSION);
	IOCTL("GEM_CLOSE", DRM_IOCTL_GEM_CLOSE);
	IOCTL("SYNCOBJ_CREATE", DRM_IOCTL_SYNCOBJ_CREATE);
	IOCTL("SYNCOBJ_DESTROY", DRM_IOCTL_SYNCOBJ_DESTROY);
	IOCTL("SYNCOBJ_WAIT", DRM_IOCTL_SYNCOBJ_WAIT);
	IOCTL("PANTHOR_DEV_QUERY", DRM_IOCTL_PANTHOR_DEV_QUERY);
	IOCTL("PANTHOR_VM_CREATE", DRM_IOCTL_PANTHOR_VM_CREATE);
	IOCTL("PANTHOR_VM_DESTROY", DRM_IOCTL_PANTHOR_VM_DESTROY);
	IOCTL("PANTHOR_VM_BIND", DRM_IOCTL_PANTHOR_VM_BIND);
	IOCTL("PANTHOR_VM_GET_STATE", DRM_IOCTL_PANTHOR_VM_GET_STATE);
	IOCTL("PANTHOR_BO_CREATE", DRM_IOCTL_PANTHOR_BO_CREATE);
	IOCTL("PANTHOR_BO_MMAP_OFFSET", DRM_IOCTL_PANTHOR_BO_MMAP_OFFSET);
	IOCTL("PANTHOR_GROUP_CREATE", DRM_IOCTL_PANTHOR_GROUP_CREATE);
	IOCTL("PANTHOR_GROUP_DESTROY", DRM_IOCTL_PANTHOR_GROUP_DESTROY);
	IOCTL("PANTHOR_GROUP_SUBMIT", DRM_IOCTL_PANTHOR_GROUP_SUBMIT);
	IOCTL("PANTHOR_GROUP_GET_STATE", DRM_IOCTL_PANTHOR_GROUP_GET_STATE);
	IOCTL("PANTHOR_TILER_HEAP_CREATE", DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE);
	IOCTL("PANTHOR_TILER_HEAP_DESTROY", DRM_IOCTL_PANTHOR_TILER_HEAP_DESTROY);
	printf("const SYNC_OP_SIGNAL %#x\n", (unsigned)DRM_PANTHOR_SYNC_OP_SIGNAL);
	return 0;
}
