// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The resource budget of a GLSL ES 1.00 program: the packing algorithm of
//! the OpenGL ES Shading Language 1.00 (revision 17), Appendix A, section 7
//! "Counting of Varyings and Uniforms".
//!
//! ```text
//! GLSL ES 1.00 specifies the storage available for varying variables in
//! terms of an array of 4-vectors. Similarly for uniform variables. [...]
//! This algorithm specifies a minimum requirement for when a set of variables
//! must be supported by an implementation. The implementation is allowed to
//! support more than the minimum and so may use a more efficient algorithm
//! and/or may support more registers than the virtual target machine.
//! In all cases, failing resource allocation for variables must result in an
//! error.
//! ```
//!
//! and OpenGL ES 2.0.25, section 2.10.4 (vertex uniforms; 3.8.1 says the same
//! of the fragment shader's):
//!
//! ```text
//! The amount of storage available for uniform variables accessed by a vertex
//! shader is specified by the implementation-dependent constant MAX_VERTEX_-
//! UNIFORM_VECTORS. This value represents the number of four-element floating-
//! point, integer, or boolean vectors that can be held in uniform variable
//! storage for a vertex shader. A link error will be generated if an attempt
//! is made to utilize more than the space available for vertex shader uniform
//! variables.
//! ```
//!
//! This implementation packs with exactly the algorithm the appendix states,
//! on a grid as tall as the limit it advertises, and refuses what that
//! algorithm cannot place. It is the one copy of the rule: the GL link feeds
//! it the uniforms (`gl/program.rs` `check_uniform_packing`), the in-process
//! compiler's link and the host front end (`tools/yakogl-tools/src/es100.rs`)
//! feed it the varyings, each from where the varyings' types are known.
//!
//! Vertex attributes are not in the appendix: OpenGL ES 2.0.25 section
//! 2.10.4 counts them in whole generic attribute locations, which the GL
//! link's attribute layout does (`gl/program.rs` `attrib_layout`).

use core::fmt;

use crate::gl::consts::*;

/// The seven kinds the appendix orders the variables by, in its order.
///
/// ```text
/// Variables are packed in the following order:
///    1. Arrays of mat4 and mat4
///    2. Arrays of mat2 and mat2 (since they occupy full rows)
///    3. Arrays of vec4 and vec4
///    4. Arrays of mat3 and mat3
///    5. Arrays of vec3 and vec3
///    6. Arrays of vec2 and vec2
///    7. Arrays of float and float
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Mat4,
    Mat2,
    Vec4,
    Mat3,
    Vec3,
    Vec2,
    Float,
}

impl Kind {
    /// The packing order above.
    const ORDER: [Kind; 7] = [Kind::Mat4, Kind::Mat2, Kind::Vec4, Kind::Mat3, Kind::Vec3, Kind::Vec2, Kind::Float];

    /// `(rows, columns)` of one element.
    ///
    /// ```text
    /// Variables consume only the minimum space required with the exception
    /// that mat2 occupies 2 complete rows.
    /// ```
    const fn shape(self) -> (u32, u32) {
        match self {
            Kind::Mat4 => (4, 4),
            Kind::Mat2 => (2, 4),
            Kind::Vec4 => (1, 4),
            Kind::Mat3 => (3, 3),
            Kind::Vec3 => (1, 3),
            Kind::Vec2 => (1, 2),
            Kind::Float => (1, 1),
        }
    }

    /// The kind of a variable of GL type `gl_type` (`GL_FLOAT_VEC3`, ...), or
    /// None for a type that takes no uniform or varying storage (a sampler)
    /// or that this table does not know.
    ///
    /// The appendix names the float types only. The integer and boolean
    /// types take the float type of their width: OpenGL ES 2.0.25 counts
    /// uniform storage in "four-element floating-point, integer, or boolean
    /// vectors" (sections 2.10.4 and 3.8.1). Samplers are opaque and are
    /// counted against the texture units instead (section 2.10.5,
    /// `gl/program.rs` `check_sampler_counts`).
    ///
    /// ES 1.00 has no non-square matrix. For the ES 3.00 ones this takes the
    /// ES 3.00 rule (GLSL ES 3.00, section 11 "Counting of Inputs and
    /// Outputs"):
    ///
    /// ```text
    /// Non-square matrices of type matCxR consume the same space as a square
    /// matrix of type matN where N is the greater of C and R.
    /// ```
    pub fn of_gl_type(gl_type: u32) -> Option<Kind> {
        Some(match gl_type {
            GL_FLOAT | GL_INT | GL_BOOL | GL_UNSIGNED_INT => Kind::Float,
            GL_FLOAT_VEC2 | GL_INT_VEC2 | GL_BOOL_VEC2 | GL_UNSIGNED_INT_VEC2 => Kind::Vec2,
            GL_FLOAT_VEC3 | GL_INT_VEC3 | GL_BOOL_VEC3 | GL_UNSIGNED_INT_VEC3 => Kind::Vec3,
            GL_FLOAT_VEC4 | GL_INT_VEC4 | GL_BOOL_VEC4 | GL_UNSIGNED_INT_VEC4 => Kind::Vec4,
            GL_FLOAT_MAT2 => Kind::Mat2,
            GL_FLOAT_MAT3 | GL_FLOAT_MAT2x3 | GL_FLOAT_MAT3x2 => Kind::Mat3,
            GL_FLOAT_MAT4 | GL_FLOAT_MAT2x4 | GL_FLOAT_MAT4x2 | GL_FLOAT_MAT3x4 | GL_FLOAT_MAT4x3 => Kind::Mat4,
            _ => return None,
        })
    }
}

/// One variable to place: `count` variables of kind `kind`, each an array of
/// `len` elements (1 for a variable that is not an array).
///
/// ```text
/// Arrays of size N are assumed to take N times the size of the base type.
/// [...] Arrays of size 1 and the base type are considered equivalent.
/// ```
///
/// `count` is for the reflection's flattened rows, where one row stands for
/// the same member of every element of an array of structs; everywhere else
/// it is 1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Var {
    pub kind: Kind,
    pub len: u32,
    pub count: u32,
}

impl Var {
    pub const fn new(kind: Kind, len: u32) -> Var {
        Var { kind, len, count: 1 }
    }
}

/// The tallest grid [`pack`] builds: the largest limit this implementation
/// advertises (its uniform vectors; its varyings are fewer).
pub const MAX_ROWS: usize = crate::gl::context::MAX_VERTEX_UNIFORM_VECTORS;

const _: () = assert!(crate::gl::context::MAX_FRAGMENT_UNIFORM_VECTORS <= MAX_ROWS);
const _: () = assert!(crate::gl::context::MAX_VARYING_VECTORS <= MAX_ROWS);

/// The grid: one bit per column, one byte per row.
struct Grid {
    used: [u8; MAX_ROWS],
    rows: u32,
}

impl Grid {
    fn free(&self, row: u32, col: u32, h: u32, w: u32) -> bool {
        (row..row + h).all(|r| (col..col + w).all(|c| self.used[r as usize] & (1 << c) == 0))
    }

    fn take(&mut self, row: u32, col: u32, h: u32, w: u32) {
        for r in row..row + h {
            for c in col..col + w {
                self.used[r as usize] |= 1 << c;
            }
        }
    }

    /// Free cells of column `col`, and the lowest free run of at least `h`
    /// rows in it.
    fn column(&self, col: u32, h: u32) -> (u32, Option<u32>) {
        let free = |r: u32| self.used[r as usize] & (1 << col) == 0;
        let space = (0..self.rows).filter(|&r| free(r)).count() as u32;
        let start = (0..(self.rows + 1).saturating_sub(h)).find(|&r| (r..r + h).all(free));
        (space, start)
    }
}

/// Place every variable `vars` yields on a grid of `rows` rows of 4 columns,
/// in the appendix's order and by its rules. `vars` is walked once per kind
/// and size, so a caller needs no buffer: an iterator over its reflection
/// does. `at(index, row, column)` is told where each variable (its position
/// in `vars`) went, once per element of `count`. Err(the variable that did
/// not fit) at the first failure:
///
/// ```text
/// If at any time the packing of a variable fails, the compiler or linker
/// must report an error.
/// ```
pub fn pack<I>(vars: I, rows: u32, mut at: impl FnMut(usize, u32, u32)) -> Result<(), Var>
where
    I: Iterator<Item = Var> + Clone,
{
    if rows as usize > MAX_ROWS {
        /* No limit this implementation advertises is taller (the asserts
         * above); a caller that passes one has a bug, not a big program. */
        return vars.clone().next().map_or(Ok(()), Err);
    }
    let mut g = Grid { used: [0; MAX_ROWS], rows };
    /* the first row the left-aligned kinds have not reached */
    let mut next = 0u32;
    /* the vec2 rule's second strategy, once a vec2 found no spare rows */
    let mut switched = false;

    for kind in Kind::ORDER {
        /* ```text
         * For each of the above types, the arrays are processed in order of
         * size, largest first.
         * ```
         * Equal sizes keep the order they were given in. */
        let mut below = u32::MAX;
        while let Some(len) = vars.clone().filter(|v| v.kind == kind && v.len < below).map(|v| v.len).max() {
            below = len;
            for (i, v) in vars.clone().enumerate().filter(|(_, v)| v.kind == kind && v.len == len) {
                let (eh, w) = kind.shape();
                let h = eh * len.max(1);
                for _ in 0..v.count {
                    let placed = match kind {
                        /* ```text
                         * For 2,3 and 4 component variables packing is
                         * started using the 1st column of the 1st row.
                         * Variables are then allocated to successive rows,
                         * aligning them to the 1st column.
                         * For 2 component variables, when there are no spare
                         * rows, the strategy is switched to using the highest
                         * numbered row and the lowest numbered column where
                         * the variable will fit. [...] Packing of any further
                         * 3 or 4 component variables will fail at this point.
                         * ```
                         * The order puts every 3 and 4 component kind before
                         * the first vec2, so that last sentence needs no code. */
                        Kind::Vec2 if switched || next + h > rows => {
                            switched = true;
                            (0..(rows + 1).saturating_sub(h))
                                .rev()
                                .find_map(|r| (0..=4 - w).find(|&c| g.free(r, c, h, w)).map(|c| (r, c)))
                        }
                        Kind::Float => {
                            /* ```text
                             * 1 component variables (i.e. floats and arrays
                             * of floats) have their own packing rule. They
                             * are packed in order of size, largest first.
                             * Each variable is placed in the column that
                             * leaves the least amount of space in the column
                             * and aligned to the lowest available rows within
                             * that column.
                             * ```
                             * A tie goes to the lower column. */
                            let mut best: Option<(u32, u32, u32)> = None;
                            for c in 0..4 {
                                if let (space, Some(r)) = g.column(c, h) {
                                    if best.is_none_or(|(left, _, _)| space - h < left) {
                                        best = Some((space - h, r, c));
                                    }
                                }
                            }
                            best.map(|(_, r, c)| (r, c))
                        }
                        _ => (next + h <= rows).then(|| {
                            next += h;
                            (next - h, 0)
                        }),
                    };
                    let Some((r, c)) = placed else { return Err(v) };
                    g.take(r, c, h, w);
                    at(i, r, c);
                }
            }
        }
    }
    Ok(())
}

/// Whether `vars` fit `rows` rows by [`pack`].
pub fn fits(vars: impl Iterator<Item = Var> + Clone, rows: u32) -> bool {
    pack(vars, rows, |_, _, _| {}).is_ok()
}

/// The three budgets the appendix's algorithm is applied to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Budget {
    VertexUniforms,
    FragmentUniforms,
    Varyings,
}

impl Budget {
    /// The GL limit's name and the value this build advertises for it: the
    /// same constants `glGetIntegerv` answers with (`gl/state.rs`).
    pub const fn limit(self) -> (&'static str, usize) {
        use crate::gl::context::{MAX_FRAGMENT_UNIFORM_VECTORS, MAX_VARYING_VECTORS, MAX_VERTEX_UNIFORM_VECTORS};
        match self {
            Budget::VertexUniforms => ("MAX_VERTEX_UNIFORM_VECTORS", MAX_VERTEX_UNIFORM_VECTORS),
            Budget::FragmentUniforms => ("MAX_FRAGMENT_UNIFORM_VECTORS", MAX_FRAGMENT_UNIFORM_VECTORS),
            Budget::Varyings => ("MAX_VARYING_VECTORS", MAX_VARYING_VECTORS),
        }
    }

    const fn what(self) -> &'static str {
        match self {
            Budget::VertexUniforms => "the vertex shader's uniforms",
            Budget::FragmentUniforms => "the fragment shader's uniforms",
            Budget::Varyings => "the program's varyings",
        }
    }
}

/// The link log's sentence for a budget that was overrun, with the limit
/// `rows` it was packed against: "the vertex shader's uniforms do not fit
/// MAX_VERTEX_UNIFORM_VECTORS = 384 (Appendix A packing)". Each caller puts
/// its own prefix before it.
pub struct Unfit {
    pub budget: Budget,
    pub rows: usize,
}

impl fmt::Display for Unfit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, _) = self.budget.limit();
        write!(f, "{} do not fit {name} = {} (Appendix A packing)", self.budget.what(), self.rows)
    }
}

/// Pack `vars` against `budget`'s limit as this build advertises it:
/// Err(the log sentence) when they do not fit.
pub fn check(budget: Budget, vars: impl Iterator<Item = Var> + Clone) -> Result<(), Unfit> {
    check_rows(budget, vars, budget.limit().1)
}

/// [`check`] against `rows` rows, for a caller that is told the limit rather
/// than built with it (the host front end answers for more than one GPU).
pub fn check_rows(budget: Budget, vars: impl Iterator<Item = Var> + Clone, rows: usize) -> Result<(), Unfit> {
    if u32::try_from(rows).is_ok_and(|r| fits(vars, r)) {
        Ok(())
    } else {
        Err(Unfit { budget, rows })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::format;
    use std::string::String;
    use std::vec::Vec;

    /// The grid as letters, one string per row: the shape of the spec's
    /// table.
    fn layout(vars: &[(char, Var)], rows: u32) -> Result<Vec<String>, Var> {
        let only: Vec<Var> = vars.iter().map(|(_, v)| *v).collect();
        let mut grid = std::vec![[' '; 4]; rows as usize];
        pack(only.iter().copied(), rows, |i, r, c| {
            let (h, w) = only[i].kind.shape();
            for row in grid.iter_mut().skip(r as usize).take((h * only[i].len.max(1)) as usize) {
                for cell in row.iter_mut().skip(c as usize).take(w as usize) {
                    *cell = vars[i].0;
                }
            }
        })?;
        Ok(grid.iter().map(|r| r.iter().collect::<String>().trim_end().into()).collect())
    }

    /// The appendix's example and the table it prints. Its comment on g
    /// says "choice of 2, either one can be used"; the rule it states (the
    /// column that leaves the least space) picks z, which is where its
    /// table puts g.
    const EXAMPLE: [(char, Var); 8] = [
        ('a', Var::new(Kind::Vec4, 1)),
        ('b', Var::new(Kind::Mat3, 1)),
        ('c', Var::new(Kind::Vec2, 3)),
        ('d', Var::new(Kind::Vec2, 2)),
        ('e', Var::new(Kind::Vec2, 1)),
        ('f', Var::new(Kind::Float, 3)),
        ('g', Var::new(Kind::Float, 2)),
        ('h', Var::new(Kind::Float, 1)),
    ];
    const EXAMPLE_TABLE: [&str; 8] = ["aaaa", "bbbf", "bbbf", "bbbf", "ccgh", "ccg", "ccdd", "eedd"];

    #[test]
    fn the_appendix_example_packs_as_its_table_shows() {
        assert_eq!(layout(&EXAMPLE, 8).unwrap(), EXAMPLE_TABLE);
    }

    /// "Packing is independent of the order of declaration."
    #[test]
    fn the_example_packs_the_same_in_any_declaration_order() {
        let mut vars = EXAMPLE.to_vec();
        vars.reverse();
        assert_eq!(layout(&vars, 8).unwrap(), EXAMPLE_TABLE);
        vars.rotate_left(3);
        assert_eq!(layout(&vars, 8).unwrap(), EXAMPLE_TABLE);
    }

    /// The example's table leaves one cell free (row 5, w): one more float
    /// fits, a second float or a vec2 does not.
    #[test]
    fn the_example_is_one_float_below_its_limit() {
        let mut vars: Vec<Var> = EXAMPLE.iter().map(|(_, v)| *v).collect();
        vars.push(Var::new(Kind::Vec2, 1));
        assert!(!fits(vars.iter().copied(), 8));
        vars.pop();
        vars.push(Var::new(Kind::Float, 1));
        assert!(fits(vars.iter().copied(), 8));
        vars.push(Var::new(Kind::Float, 1));
        assert!(!fits(vars.iter().copied(), 8));
    }

    /// "Some varyings e.g. mat4[8] will be too large to fit. These always
    /// fail with this algorithm." And "large arrays e.g.. for varyings,
    /// float[16] will always fail".
    #[test]
    fn the_appendix_examples_that_always_fail_fail() {
        assert!(!fits([Var::new(Kind::Mat4, 8)].iter().copied(), 8));
        assert!(!fits([Var::new(Kind::Float, 16)].iter().copied(), 8));
        assert!(fits([Var::new(Kind::Float, 8)].iter().copied(), 8), "float[8] will occupy one column");
    }

    /// "the first type to be packed (successfully) is mat4[2]": it fills the
    /// eight varying rows exactly.
    #[test]
    fn mat4_2_fills_the_varying_grid() {
        assert!(fits([Var::new(Kind::Mat4, 2)].iter().copied(), 8));
        assert!(!fits([Var::new(Kind::Mat4, 2), Var::new(Kind::Float, 1)].iter().copied(), 8));
    }

    /// mat2 "occupies 2 complete rows", so four fill eight rows and a vec2
    /// no longer fits -- where eight vec2[2] fit (the second four at z).
    #[test]
    fn mat2_takes_full_rows_and_vec2_arrays_do_not() {
        let mat2 = [Var::new(Kind::Mat2, 1); 4];
        assert!(fits(mat2.iter().copied(), 8));
        assert!(!fits(mat2.iter().copied().chain([Var::new(Kind::Vec2, 1)]), 8));
        let vec2 = [Var::new(Kind::Vec2, 2); 8];
        assert!(fits(vec2.iter().copied(), 8));
        assert!(!fits(vec2.iter().copied().chain([Var::new(Kind::Vec2, 1)]), 8));
    }

    /// A mat3 is three rows of three columns: two of them and two vec3 take
    /// the eight rows, and the w column is left to the floats.
    #[test]
    fn mat3_leaves_its_fourth_column_to_the_floats() {
        let vars = [Var::new(Kind::Mat3, 2), Var::new(Kind::Vec3, 2), Var::new(Kind::Float, 8)];
        assert!(fits(vars.iter().copied(), 8));
        let vars = [Var::new(Kind::Mat3, 2), Var::new(Kind::Vec3, 3)];
        assert!(!fits(vars.iter().copied(), 8), "nine rows of three columns");
    }

    /// Four floats share a row: a plain row count would have refused the
    /// 32 floats this packs into eight rows.
    #[test]
    fn floats_share_rows() {
        assert!(fits([Var::new(Kind::Float, 1); 32].iter().copied(), 8));
        assert!(!fits([Var::new(Kind::Float, 1); 33].iter().copied(), 8));
    }

    /// Every vec3 is packed before the first vec2, whatever the declaration
    /// order.
    #[test]
    fn a_vec3_declared_after_a_vec2_is_packed_first() {
        assert!(!fits([Var::new(Kind::Vec2, 8), Var::new(Kind::Vec3, 1)].iter().copied(), 8));
        assert!(fits([Var::new(Kind::Vec2, 7), Var::new(Kind::Vec3, 1)].iter().copied(), 8));
    }

    /// `count` places that many copies, each a variable of its own.
    #[test]
    fn count_is_that_many_variables() {
        assert!(fits([Var { kind: Kind::Vec4, len: 1, count: 8 }].iter().copied(), 8));
        assert!(!fits([Var { kind: Kind::Vec4, len: 1, count: 9 }].iter().copied(), 8));
        assert!(fits([Var { kind: Kind::Float, len: 2, count: 16 }].iter().copied(), 8));
        assert!(!fits([Var { kind: Kind::Float, len: 3, count: 12 }].iter().copied(), 8), "two per column of eight");
    }

    #[test]
    fn a_grid_taller_than_any_advertised_limit_is_refused() {
        assert!(!fits([Var::new(Kind::Float, 1)].iter().copied(), MAX_ROWS as u32 + 1));
        assert!(fits([Var::new(Kind::Vec4, MAX_ROWS as u32)].iter().copied(), MAX_ROWS as u32));
    }

    #[test]
    fn the_types_take_the_kind_of_their_width() {
        assert_eq!(Kind::of_gl_type(GL_BOOL), Some(Kind::Float));
        assert_eq!(Kind::of_gl_type(GL_INT_VEC3), Some(Kind::Vec3));
        assert_eq!(Kind::of_gl_type(GL_BOOL_VEC2), Some(Kind::Vec2));
        assert_eq!(Kind::of_gl_type(GL_FLOAT_MAT2), Some(Kind::Mat2));
        assert_eq!(Kind::of_gl_type(GL_FLOAT_MAT2x3), Some(Kind::Mat3));
        assert_eq!(Kind::of_gl_type(GL_FLOAT_MAT4x2), Some(Kind::Mat4));
        assert_eq!(Kind::of_gl_type(GL_SAMPLER_2D), None);
    }

    #[test]
    fn the_message_names_the_limit_and_its_value() {
        let e = check_rows(Budget::VertexUniforms, [Var::new(Kind::Vec4, 2)].into_iter(), 1).unwrap_err();
        assert_eq!(
            format!("{e}"),
            "the vertex shader's uniforms do not fit MAX_VERTEX_UNIFORM_VECTORS = 1 (Appendix A packing)"
        );
        let e = check_rows(Budget::Varyings, [Var::new(Kind::Vec4, 2)].into_iter(), 1).unwrap_err();
        assert_eq!(format!("{e}"), "the program's varyings do not fit MAX_VARYING_VECTORS = 1 (Appendix A packing)");
    }
}
