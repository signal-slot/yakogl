// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! The command stream instruction words panthor's `queue_run_job` wraps a
//! job in (Linux v6.18, drivers/gpu/drm/panthor/panthor_sched.c:3133-3231,
//! used under the MIT option of its `GPL-2.0 or MIT` licence): the
//! parameters of `get_job_cs_params` (:3005-3030), the sequence of
//! `prepare_job_instrs` (:3039-3120) without the profiling words, padded to
//! a 64-byte line (:94, :3116-3119), and the ring copy of
//! `copy_instrs_to_ringbuf` (:2969-2994). [`super`]'s doc lists the words.
//!
//! Empty for now: the plan's item "jobwords" fills it.
