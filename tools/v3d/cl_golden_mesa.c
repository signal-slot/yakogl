/* SPDX-License-Identifier: MIT
 *
 * cl_golden's reference half: Mesa's generated packers and unpackers as
 * Mesa's own build made them (build-v3d/src/broadcom/cle/
 * v3d_packet_v42_pack.h, or v3d_packet_v71_pack.h with -DV3D_CL_VER=71), in a translation unit of their own, so that they
 * are not the header driver/src/v3d commits (both define V3D42_PACK_H, or V3D71_PACK_H). See
 * cl_golden.c.
 */
#include <stddef.h>
#include <stdint.h>

#define NDEBUG
#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#define __gen_unpack_address(cl, s, e) ((uint32_t)(__gen_unpack_uint(cl, s, e) << (31 - ((e) - (s)))))
#ifndef V3D_CL_VER
#define V3D_CL_VER 42
#endif
#if V3D_CL_VER == 71
#include "cle/v3d_packet_v71_pack.h"
#else
#include "cle/v3d_packet_v42_pack.h"
#endif

#include "v3d_cl_packets.h"

/* V3D<ver>_<NAME>, as in driver/src/v3d/v3d_cl.h. */
#define V3D_CL_NAME__(ver, name) V3D##ver##_##name
#define V3D_CL_NAME_(ver, name) V3D_CL_NAME__(ver, name)
#define V3D_CL_NAME(name) V3D_CL_NAME_(V3D_CL_VER, name)

#define MESA_REF(name)                                                                             \
	void mesa_pack_##name(uint8_t *out, const void *v);                                        \
	void mesa_pack_##name(uint8_t *out, const void *v)                                         \
	{                                                                                          \
		V3D_CL_NAME(name##_pack)(NULL, out, (const struct V3D_CL_NAME(name) *)v);                     \
	}                                                                                          \
	void mesa_unpack_##name(const uint8_t *in, void *v);                                       \
	void mesa_unpack_##name(const uint8_t *in, void *v)                                        \
	{                                                                                          \
		V3D_CL_NAME(name##_unpack)(in, (struct V3D_CL_NAME(name) *)v);                                \
	}                                                                                          \
	extern const size_t mesa_size_##name, mesa_len_##name;                                     \
	const size_t mesa_size_##name = sizeof(struct V3D_CL_NAME(name));                               \
	const size_t mesa_len_##name = V3D_CL_NAME(name##_length);

V3D_CL_PACKET_LIST(MESA_REF)
