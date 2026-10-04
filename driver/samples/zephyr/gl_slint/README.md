# gl_slint: the crate the SafeUI Supervisor builds (not a sample any more)

Step 1 of `driver/docs/one-method.md` deleted the `gl_slint` sample: the one
Slint sample on all three boards is `../slint`, which also holds the UI
(`../slint/ui`) and the no_std patches (`../slint/nostd`).

What is left here has one user outside this repository: the SafeUI
application repository's Supervisor builds its rich UI from this crate by
path (`supervisor/core/rich_ui.cmake` looks for
`samples/zephyr/gl_slint/rust/Cargo.toml.in` and reads
`samples/zephyr/gl_slint/ui/cluster.slint`). So, as with the board layer's
aliases for the Qt application (`one-method.md` section 2), it stays until
that user has moved:

- `rust/`: the crate (`slint_draw_frame`, `slint_set_vehicle`, the underlay
  and `standalone` the Supervisor uses);
- `ui`: a link to `../slint/ui`, so there is one copy of the UI.

It goes in its own commit once `rich_ui.cmake` builds from `../slint` (the
SafeUI repository's owner decides that change).
