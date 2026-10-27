/* SPDX-License-Identifier: MIT
 *
 * cl_golden's reference half: Mesa's generated packers and unpackers as
 * Mesa's own build made them (build-v3d/src/broadcom/cle/
 * v3d_packet_v42_pack.h), in a translation unit of their own, so that they
 * are not the header driver/src/v3d commits (both define V3D42_PACK_H). See
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
#include "cle/v3d_packet_v42_pack.h"

#include "v3d_cl_packets.h"

#define MESA_REF(name)                                                                             \
	void mesa_pack_##name(uint8_t *out, const void *v);                                        \
	void mesa_pack_##name(uint8_t *out, const void *v)                                         \
	{                                                                                          \
		V3D42_##name##_pack(NULL, out, (const struct V3D42_##name *)v);                     \
	}                                                                                          \
	void mesa_unpack_##name(const uint8_t *in, void *v);                                       \
	void mesa_unpack_##name(const uint8_t *in, void *v)                                        \
	{                                                                                          \
		V3D42_##name##_unpack(in, (struct V3D42_##name *)v);                                \
	}                                                                                          \
	extern const size_t mesa_size_##name, mesa_len_##name;                                     \
	const size_t mesa_size_##name = sizeof(struct V3D42_##name);                               \
	const size_t mesa_len_##name = V3D42_##name##_length;

V3D_CL_PACKETS(MESA_REF)
