<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# A buffer deleted while an unbound transform feedback object attaches it

Design, no code. Cause of `dEQP-GLES3.functional.lifetime.attach.deleted_output.buffer_transform_feedback`
("Writing to container with deleted attachment modified a new object"), a
Fail on the AM62P (`docs/es3/am62p-gles3-notpass.tsv`) and the Pi 4
(`docs/es3/pi4-gles3-fails.tsv`, image LT). It is GL core, so it is the same
on every back end.

## 1. The sequence

`glsLifetimeTests.cpp`, `OutputAttachmentTest::iterate()`, with the
transform feedback types of `es3fLifetimeTests.cpp` (`BufferTfAttacher`,
`BufferTfOutputAttacher`). The container is a transform feedback object (TF
object), the element a buffer:

1. Generate the container; `glGenBuffers` the element `X`; `initAttachment(0, X)`
   (`glBindBuffer(ARRAY, X)`, `glBufferData(.., GL_DYNAMIC_READ)`, unbind).
2. Attach: `glBindTransformFeedback(tf)`, `glBindBufferBase(TRANSFORM_FEEDBACK_BUFFER, 0, X)`,
   `glBindTransformFeedback(0)`.
3. `setupContainer(refSeed, tf)`: a temporary buffer and VAO, then with `tf`
   bound `glBeginTransformFeedback(GL_TRIANGLES)`, `glDrawArrays(.., 0, 3)`,
   `glEndTransformFeedback`, `glBindTransformFeedback(0)`. The vec4
   `gl_Position` is captured into `X`.
4. `drawAttachment(X, refSurface)` renders `X` as the `pos` attribute.
5. `X` leaves scope: `glDeleteBuffers(X)` while `tf` (not bound) still attaches it.
6. `replaceName()`: **a buffer's binder does not require `glGen*`**
   (`SimpleBinder` without the flag; only the VAO binder passes `true`), so the
   test reuses the old name: the new element is `X` again, "bound without
   gen" (ES 2.0 and ES 3.0 2.9: binding an unused name creates the object).
   `initAttachment(newSeed, X)`: `glBindBuffer(ARRAY, X)`, `glBufferData`.
7. `drawAttachment(X, newSurface)`: render the new buffer.
8. `setupContainer(refSeed, tf)` again: the capture writes through `tf`.
9. `drawAttachment(X, delSurface)`; `newSurface` and `delSurface` must be
   equal with threshold zero. Then `glBindBufferBase(.., 0, 0)` on `tf`.

## 2. The rule

ES 3.0 D.1.2 (the repo cites it so since `653fe45d`; 2.9 for the buffer
object): deleting a buffer resets its bindings in the current context and the
bound container only; a TF object (or VAO) that is not current keeps its
attachment, and the name is free at once. A later `glBindBuffer(X)` is the
"bind creates the object" case: a new object, unrelated to the deleted one,
which the TF object keeps writing to in step 8.

## 3. The current behaviour

The deleted buffer's storage does survive:

- `glDeleteBuffers` (`src/gl/buffer.rs:533`, the call at `:589`) asks
  `tf::on_buffer_delete` (`src/gl/tf.rs:340`); an unbound TF object or VAO
  holding the slot makes it keep the storage and set `ORPHANS[slot]`
  (`tf.rs:355`, `ORPHANS` at `:111`); `buffer.rs:590` sets `target = 0`
  (`glIsBuffer` false) and **leaves `used = true`**. `names::forget` drops
  the far name, a small name stays the slot number (`names.rs:171`).
- `glGenBuffers` (`buffer.rs:283`) skips a `used` slot, so a *generated* new
  buffer is another slot. That is why `653fe45d` and the host test
  `capture_into_a_deleted_buffer_leaves_the_new_buffer_alone`
  (`src/gl/tf_capture_tests.rs:1322`) pass: that test calls `glGenBuffers`.
- The TF object's binding is the slot number (`TfBinding.buffer`, `tf.rs:63`),
  and the capture writes `bind[i].buffer`'s slot
  (`tf.rs:1054..1058`, `buffer::write_at` from `tf_capture.rs:371`).

The defect is step 6. The name `X` is a slot index (`names.rs`, small names),
so `glBindBuffer(ARRAY, X)` resolves to the orphan's slot: `names::bind_slot`
(`buffer.rs:383`) returns `X`, and at `buffer.rs:388..392` the "first bind
creates the name" branch is skipped because `b.used` is still true; the
bind only sets `target`. `glBufferData` then respecifies **the orphan's own
record**. The "new object" is the orphan, and step 8's capture through `tf`
writes the new object's bytes: `delSurface != newSurface`. Textures and
renderbuffers solved this in `glBindTexture` (`src/gl/texture.rs:1130..1150`,
`framebuffer::orphan_moved`, `framebuffer.rs:1419`, the rule at `:1387`);
buffers have no such move.

Side effects of the same hole, not tested by dEQP today: `ORPHANS[X]` stays
true over the new live object, so a later `tf::release_slot(X)` (the last
holder lets go, `tf.rs:362`, step 9's `glBindBufferBase(.., 0, 0)`) calls
`free_orphan` (`buffer.rs:608`) and frees a buffer the application still
owns; a VAO holder (`vao::holds_buffer`, `vao.rs:60`) has the same hole, so
`deleted_input.buffer_vertex_array` passes only where the test generates a
name.

## 4. Fix design (as 653fe45d, plus the move the framebuffer rule has)

In `glBindBuffer` (`buffer.rs`), after `bind_slot` and before the "first bind
creates" branch: if `tf::is_orphan(c, slot)` (the `ORPHANS` flag; it is the
only state that marks a deleted slot, `target == 0 && used` is not enough
because a generated, never bound buffer looks the same):

1. Find a free slot `to` (`1..MAX_BUFFERS`, `!used`).
2. Move the record: `buffers[to] = replace(buffers[X], zeroed_buffer())`
   (the `mem` and `last_use` move with it), and `ORPHANS[to] = true`,
   `ORPHANS[X] = false`.
3. Rewrite the holders from `X` to `to`: every used TF object's `bind[].buffer`
   (`tf.rs`, new `tf::orphan_moved`), and every VAO's `attribs[].buffer` and
   `element_buffer` (new `vao::orphan_moved`; the bound VAO's live
   `c.attribs`/`bound_element_buffer` do not hold an orphan, delete reset them).
4. Move the buffer's retired storages and dirty ranges, which are keyed by
   slot: `RETIRED[i].buffer` (`buffer.rs:963..`, `retire`/`take_retired`/
   `note_write`); a pending `last_use` kick travels with the record.
5. No free slot: free the orphan (`free_buffer_storage`) as the texture path
   does (`texture.rs:1140..1143`), and log it; the holders then lose the buffer.

The record at `X` is then zero, and the existing branch makes the fresh
object. `glBufferData` only reaches a bound buffer, `glBindBufferBase/Range`
for the indexed points go through `glBindBuffer` (`tf.rs:300`), so this is the
one place. Both VAO and TF holders use the same function, so
`deleted_input.buffer_vertex_array` stops depending on the test generating a
name. Files: `src/gl/buffer.rs`, `src/gl/tf.rs`, `src/gl/vao.rs`; none is in
the file lists of the other plans named in the item (check against the
current main at the time).

Not chosen: giving the new object a far name over a borrowed slot
(`names::bind_slot`): the orphan's old slot would be handed again by
`glGenBuffers` when it is freed, while the far entry still stands, and
`FAR_NAMES` is 16.

## 5. The host test

New `tests/es3_tf_lifetime.rs` (the fix item), through the C API, no
rendering: make a TF program capturing one varying; generate `X`, attach it
to TF object `tf` at index 0 (as step 2), `glBufferData` 64 bytes `0xcd`;
`glBindTransformFeedback(0)`; `glDeleteBuffers(X)`; then **`glBindBuffer(ARRAY, X)`
and `glBufferData` 64 bytes of `0xab`, without `glGenBuffers`** (the step the
existing test lacks); bind `tf`, capture a draw; the bytes of the buffer named
`X` (`glGetBufferSubData`/`gpu_view`) must still be `0xab`, the capture must
have gone to the old storage (check with the TF binding queried through
`glGetIntegeri_v` on `tf`, which must not name `X` any more: `0`/other), and
`glIsBuffer(X)` true. Also: the release at the end does not free the live
`X` (`glBufferSubData` on it works), and the same with a VAO holder.
It fails today at the byte check. `tf_capture_tests.rs` is `#[cfg(test)]` in
`src/`, which plan #339 changes; the new file avoids it.

## 6. What the host run says

The host library keeps the command stream instead of executing it, so every
`glReadPixels` is black (`tools/cts/host-deqp-run.sh` header). This case
compares two renders of the new buffer; both are black on the host, so the
host verdict is Pass whatever the fix does and means nothing for this case
(it could not be run in this workspace at all: no VK-GL-CTS checkout, hence no
`DEQP_WORK`; the CTS source was read from the Khronos repository). The
decider is the byte-level host test above, and the board rerun of the case.
