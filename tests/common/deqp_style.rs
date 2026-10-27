// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-style GLSL ES 1.00 shaders from a seeded generator, for the host
//! tests that run a back end's program against the IR it was compiled from
//! (`usc_limits.rs` on the PowerVR, `backend_differential.rs` on the GC7000).
//!
//! The table's sources are applications' shaders. dEQP-GLES2's `shaders.*`
//! groups hand glShaderSource other shapes: an `if` on a uniform or an input
//! whose arms assign locals read after it (`conditionals`), chains of
//! operators over constructors and swizzles (`operator`, `swizzles`), loops
//! bounded by a constant, a uniform or an input, with a `continue`, a
//! `break` or an `if` in the body (`loops`), functions that return from
//! inside an `if` (`functions`, `return`), the random generator's wrappers
//! (`random`: `x * 1.0`, `float(int(..))`, a bool as a float), and at times
//! an attribute whose only read is in code that never runs.
//!
//! Those are the shapes the PowerVR's later transformations rewrite -- a
//! product by 1.0 read as its operand, a product and its sum as one `fmad`,
//! a clamp to [0, 1] as one saturating add, an `if`'s arms run predicated
//! into one register or under a branch -- so this writes them, many times
//! over, with nothing a model cannot run: no texture, no derivative, no
//! division by a value, no function that makes a NaN.
//!
//! A program is its seed: the same seed gives the same text on every host,
//! so a failure names a text anyone can regenerate.

#![allow(dead_code)]

/// One generated shader.
pub struct Shader {
    pub name: String,
    pub fragment: bool,
    pub text: String,
}

struct Gen {
    x: u64,
    fragment: bool,
    /// Inside a loop body: its counter's name, which is readable.
    counter: Option<String>,
    /// Loops written so far: a `do`/`while` counter's name carries it.
    loops: u32,
}

const FLOAT_LITERALS: [&str; 10] = ["0.0", "1.0", "-1.0", "0.5", "2.0", "0.25", "1.5", "-0.5", "0.75", "3.0"];
const SWIZZLES4: [&str; 8] = ["xyzw", "wzyx", "yzwx", "xxyy", "rgba", "bgra", "stpq", "zwxy"];
const LANES: [&str; 4] = ["x", "y", "z", "w"];

impl Gen {
    fn next(&mut self) -> u32 {
        self.x ^= self.x << 13;
        self.x ^= self.x >> 7;
        self.x ^= self.x << 17;
        (self.x >> 24) as u32
    }
    fn pick(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn of<'a>(&mut self, set: &[&'a str]) -> &'a str {
        set[self.pick(set.len() as u32) as usize]
    }

    fn float_var(&mut self) -> String {
        let input = if self.fragment { "v_s" } else { "a_s" };
        match self.pick(8) {
            0 | 1 => self.of(&["u_f0", "u_f1", "u_f2"]).to_string(),
            2 => input.to_string(),
            3 | 4 => self.of(&["f0", "f1"]).to_string(),
            5 => format!("{}.{}", self.vec4_var(), self.of(&LANES)),
            _ => self.of(&FLOAT_LITERALS).to_string(),
        }
    }
    fn vec4_var(&mut self) -> String {
        let inputs: [&str; 2] = if self.fragment { ["v_in0", "v_in1"] } else { ["a_in0", "a_in1"] };
        match self.pick(7) {
            0 | 1 => self.of(&["u_v0", "u_v1"]).to_string(),
            2 | 3 => self.of(&inputs).to_string(),
            _ => self.of(&["t0", "t1", "t2"]).to_string(),
        }
    }

    fn float(&mut self, d: u32) -> String {
        if d == 0 || self.pick(6) == 0 {
            return self.float_var();
        }
        let d = d - 1;
        match self.pick(28) {
            0 => format!("({} + {})", self.float(d), self.float(d)),
            1 => format!("({} - {})", self.float(d), self.float(d)),
            2 | 3 => format!("({} * {})", self.float(d), self.float(d)),
            /* the random generator's products by one */
            4 => format!("({} * 1.0)", self.float(d)),
            5 => format!("(1.0 * {})", self.float(d)),
            /* a product and the sum that alone reads it */
            6 => format!("({} * {} + {})", self.float(d), self.float(d), self.float(d)),
            7 => format!("({} - {} * {})", self.float(d), self.float(d), self.float(d)),
            8 => format!("({} * 2.0 - 1.0)", self.float(d)),
            9 => format!("({} + {} * {})", self.float(d), self.float(d), self.float(d)),
            /* clamps to [0, 1], each nesting, and one to other bounds */
            10 => format!("clamp({}, 0.0, 1.0)", self.float(d)),
            11 => format!("min(max({}, 0.0), 1.0)", self.float(d)),
            12 => format!("max(min({}, 1.0), 0.0)", self.float(d)),
            13 => format!("clamp({}, -0.5, 0.75)", self.float(d)),
            14 => format!("(-({}))", self.float(d)),
            15 => format!("abs({})", self.float(d)),
            16 => format!("float({})", self.int(d)),
            17 => format!("{}({}, {})", self.of(&["min", "max"]), self.float(d), self.float(d)),
            18 => format!("mix({}, {}, {})", self.float(d), self.float(d), self.float(d)),
            19 => format!("dot({}, {})", self.vec4(d), self.vec4(d)),
            20 => format!("({} ? {} : {})", self.cond(d), self.float(d), self.float(d)),
            21 => format!("float({})", self.cond(d)),
            22 => format!("{}.{}", self.vec4(d), self.of(&LANES)),
            23 => format!("({} / {})", self.float(d), self.of(&["2.0", "4.0", "0.5"])),
            24 => format!("floor({})", self.float(d)),
            25 if self.counter.is_some() => format!("float({})", self.counter.clone().unwrap()),
            26 => format!("fn_sel({}, {})", self.float(d), self.float(d)),
            _ => format!("({} * {})", self.float(d), self.of(&FLOAT_LITERALS)),
        }
    }

    fn vec2(&mut self, d: u32) -> String {
        match self.pick(3) {
            0 => format!("vec2({}, {})", self.float(d), self.float(d)),
            1 => format!("{}.{}", self.vec4(d), self.of(&["xy", "zw", "yx", "wz", "xx"])),
            _ => format!("({}.{} * {})", self.vec4_var(), self.of(&["xy", "zw"]), self.float(d)),
        }
    }

    fn vec4(&mut self, d: u32) -> String {
        if d == 0 || self.pick(6) == 0 {
            return match self.pick(4) {
                0 => format!("{}.{}", self.vec4_var(), self.of(&SWIZZLES4)),
                _ => self.vec4_var(),
            };
        }
        let d = d - 1;
        match self.pick(25) {
            0 => format!("({} + {})", self.vec4(d), self.vec4(d)),
            1 => format!("({} - {})", self.vec4(d), self.vec4(d)),
            2 => format!("({} * {})", self.vec4(d), self.vec4(d)),
            3 => format!("({} * {})", self.vec4(d), self.float(d)),
            4 => format!("({} * {})", self.float(d), self.vec4(d)),
            5 => format!("vec4({}, {}, {}, {})", self.float(d), self.float(d), self.float(d), self.float(d)),
            6 => format!("vec4({}, {})", self.vec2(d), self.vec2(d)),
            7 => format!("vec4({})", self.float(d)),
            8 => format!("{}.{}", self.vec4(d), self.of(&SWIZZLES4)),
            9 => format!("clamp({}, 0.0, 1.0)", self.vec4(d)),
            10 => format!("min(max({}, 0.0), 1.0)", self.vec4(d)),
            11 => format!("mix({}, {}, {})", self.vec4(d), self.vec4(d), self.float(d)),
            12 => format!("(-({}))", self.vec4(d)),
            13 => format!("abs({})", self.vec4(d)),
            14 => format!("({} * 1.0)", self.vec4(d)),
            /* a lane of 1.0 among the factors, and a matrix times a vector
             * with one: `scissorMat * vec3(p, 1.0)`'s shape */
            15 => format!("({} * vec4({}, 1.0, {}, 1.0))", self.vec4(d), self.float(d), self.float(d)),
            16 => format!("(u_m * vec4({}.xyz, 1.0))", self.vec4(d)),
            17 => format!("(u_m * {})", self.vec4(d)),
            18 => format!("({} * {} + {})", self.vec4(d), self.vec4(d), self.vec4(d)),
            19 => format!("({} - {} * {})", self.vec4(d), self.vec4(d), self.float(d)),
            20 => format!("({} * 2.0 - 1.0)", self.vec4(d)),
            21 => format!("({} ? {} : {})", self.cond(d), self.vec4(d), self.vec4(d)),
            22 => format!("vec4({}, {}, {})", self.float(d), self.vec2(d), self.float(d)),
            23 => format!("fn_mad({}, {})", self.vec4(d), self.float(d)),
            _ => format!("max(min({}, 1.0), 0.0)", self.vec4(d)),
        }
    }

    fn int(&mut self, d: u32) -> String {
        if d == 0 || self.pick(3) == 0 {
            return match self.pick(7) {
                0 | 1 => self.of(&["ui_a", "ui_b"]).to_string(),
                2 => "i0".to_string(),
                3 if self.counter.is_some() => self.counter.clone().unwrap(),
                _ => self.of(&["0", "1", "2", "3", "-1"]).to_string(),
            };
        }
        let d = d - 1;
        match self.pick(7) {
            0 => format!("({} + {})", self.int(d), self.int(d)),
            1 => format!("({} - {})", self.int(d), self.int(d)),
            2 => format!("({} * {})", self.int(d), self.int(d)),
            3 => format!("({} * 1)", self.int(d)),
            4 => format!("int({})", self.float(d)),
            5 => format!("({} / {})", self.int(d), self.of(&["1", "2", "3"])),
            _ => format!("({} ? {} : {})", self.cond(d), self.int(d), self.int(d)),
        }
    }

    fn cond(&mut self, d: u32) -> String {
        let d = d.saturating_sub(1);
        match self.pick(if d == 0 { 7 } else { 10 }) {
            0..=2 => format!("{} {} {}", self.float(d), self.of(&["<", "<=", ">", ">="]), self.float(d)),
            3 | 4 => format!("{} {} {}", self.int(d), self.of(&["==", "!=", "<", ">="]), self.int(d)),
            5 => "ub_c".to_string(),
            6 => format!("{} {} {}", self.float_var(), self.of(&["<", ">"]), self.of(&FLOAT_LITERALS)),
            7 => format!("!({})", self.cond(d)),
            8 => format!("({} && {})", self.cond(d), self.cond(d)),
            _ => format!("({} || {})", self.cond(d), self.cond(d)),
        }
    }

    /// An assignment. `store`: the arms may write the output itself.
    fn simple(&mut self, store: bool) -> String {
        let t = self.of(&["t0", "t1", "t2"]);
        let f = self.of(&["f0", "f1"]);
        match self.pick(15) {
            /* a product, its factor rotated as dEQP's loop bodies rotate
             * `res` (`res = res.yzwx`), then the product's sum: in a loop
             * kept as a loop the factor's register is written between them */
            14 => {
                let (lane, k, c) = (self.of(&LANES), self.float_var(), self.float_var());
                let g = if f == "f0" { "f1" } else { "f0" };
                format!("{f} = {t}.{lane} * {k}; {t} = {t}.yzwx; {g} = {f} + {c};")
            }
            0..=3 => format!("{t} = {};", self.vec4(2)),
            4 | 5 => format!("{f} = {};", self.float(3)),
            6 => format!("{t}.{} = {};", self.of(&["xy", "zw", "yx"]), self.vec2(2)),
            7 => format!("{t}.{} = {};", self.of(&LANES), self.float(2)),
            8 => format!("{t} {} {};", self.of(&["*=", "+=", "-="]), self.vec4(1)),
            9 => format!("{f} {} {};", self.of(&["*=", "+=", "-="]), self.float(2)),
            10 => format!("i0 = {};", self.int(2)),
            11 | 12 if store => {
                let out = if self.fragment { "gl_FragColor" } else { "v_out3" };
                format!("{out} = {};", self.vec4(2))
            }
            _ => format!("{t} = {};", self.vec4(1)),
        }
    }

    fn block(&mut self, depth: u32, store: bool, pad: &str, out: &mut String) {
        let n = 1 + self.pick(4);
        for _ in 0..n {
            if depth < 2 && self.pick(8) == 0 {
                self.if_stmt(depth + 1, store, pad, out);
            } else {
                out.push_str(&format!("{pad}{}\n", self.simple(store)));
            }
        }
    }

    fn if_stmt(&mut self, depth: u32, store: bool, pad: &str, out: &mut String) {
        let inner = format!("{pad}    ");
        out.push_str(&format!("{pad}if ({})\n{pad}{{\n", self.cond(2)));
        if self.fragment && self.pick(24) == 0 {
            out.push_str(&format!("{inner}discard;\n"));
        } else {
            self.block(depth, store, &inner, out);
        }
        out.push_str(&format!("{pad}}}\n"));
        if self.pick(4) != 0 {
            out.push_str(&format!("{pad}else\n{pad}{{\n"));
            self.block(depth, store, &inner, out);
            out.push_str(&format!("{pad}}}\n"));
        }
    }

    /// A loop as dEQP's `loops` group writes them: `for` or `do`/`while`,
    /// bounded by a constant, by a uniform, or by an input (a bound that
    /// differs between instances), its body assignments with, at times, a
    /// `continue` on the counter, a `break` on a condition or an `if`.
    fn loop_stmt(&mut self, out: &mut String) {
        let input = if self.fragment { "v_s" } else { "a_s" };
        let bound = match self.pick(6) {
            0..=2 => (1 + self.pick(3)).to_string(),
            3 | 4 => "ui_b".to_string(),
            _ => format!("int(clamp({input} * 2.0, 0.0, 3.0))"),
        };
        self.loops += 1;
        let do_while = self.pick(4) == 0;
        let counter = if do_while { format!("w{}", self.loops) } else { "li".to_string() };
        if do_while {
            out.push_str(&format!("    int {counter} = 0;\n    do\n    {{\n"));
        } else {
            out.push_str(&format!("    for (int li = 0; li < {bound}; li++)\n    {{\n"));
        }
        self.counter = Some(counter.clone());
        let extra = self.pick(6);
        /* a `continue` in a `do`/`while` runs the test, which increments */
        if extra == 0 {
            out.push_str(&format!("        if ({counter} == ui_a)\n            continue;\n"));
        }
        let n = 1 + self.pick(2);
        for _ in 0..n {
            out.push_str(&format!("        {}\n", self.simple(false)));
        }
        match extra {
            1 => out.push_str(&format!("        if ({})\n            break;\n", self.cond(1))),
            2 => self.if_stmt(2, false, "        ", out),
            _ => {}
        }
        self.counter = None;
        if do_while {
            out.push_str(&format!("    }} while (++{counter} < {bound});\n"));
        } else {
            out.push_str("    }\n");
        }
    }

    fn shader(&mut self) -> String {
        let mut s = String::new();
        if self.fragment {
            s.push_str("precision mediump float;\n");
            s.push_str("varying mediump vec4 v_in0;\nvarying mediump vec4 v_in1;\nvarying mediump float v_s;\n");
        } else {
            s.push_str("attribute highp vec4 a_position;\nattribute mediump vec4 a_in2;\n");
            s.push_str("attribute mediump vec4 a_in0;\n");
            s.push_str("attribute mediump vec4 a_in1;\nattribute mediump float a_s;\n");
            s.push_str("varying mediump vec4 v_out0;\nvarying mediump vec4 v_out1;\n");
            s.push_str("varying mediump vec4 v_out2;\nvarying mediump vec4 v_out3;\n");
        }
        s.push_str("uniform mediump float u_f0;\nuniform mediump float u_f1;\nuniform mediump float u_f2;\n");
        s.push_str("uniform mediump vec4 u_v0;\nuniform mediump vec4 u_v1;\nuniform mediump mat4 u_m;\n");
        s.push_str("uniform mediump int ui_a;\nuniform mediump int ui_b;\nuniform bool ub_c;\n\n");
        /* a return from inside an `if`, and a product and its sum behind a
         * call: both are inlined */
        s.push_str("mediump float fn_sel(mediump float a, mediump float b)\n{\n    if (a > b)\n        return a;\n");
        s.push_str("    return b * 2.0 - a;\n}\n\n");
        s.push_str("mediump vec4 fn_mad(mediump vec4 a, mediump float b)\n{\n    return a * b + u_v0;\n}\n\n");
        s.push_str("void main()\n{\n");
        /* every local written before anything reads it */
        s.push_str(&format!("    mediump vec4 t0 = {};\n", self.vec4_input()));
        s.push_str(&format!("    mediump vec4 t1 = {};\n", self.vec4_input()));
        s.push_str(&format!("    mediump vec4 t2 = {};\n", self.vec4_input()));
        let input = if self.fragment { "v_s" } else { "a_s" };
        s.push_str(&format!("    mediump float f0 = {input};\n    mediump float f1 = u_f{};\n", self.pick(3)));
        s.push_str("    mediump int i0 = ui_a;\n");
        /* the output the arms may store, written first so no path leaves it
         * unwritten */
        let store = self.pick(3) == 0;
        if store {
            let out = if self.fragment { "gl_FragColor" } else { "v_out3" };
            s.push_str(&format!("    {out} = {};\n", self.vec4(1)));
        }
        /* An attribute whose only read never runs -- an arm a constant
         * condition does not take, a loop of no trips -- declared ahead of
         * the attributes that are read: it is active (ES 2.0 2.10.4 counts
         * static use), so it keeps its place among them. */
        if !self.fragment && self.pick(3) == 0 {
            s.push_str(if self.pick(2) == 0 {
                "    if (2 < 1)\n    {\n        t1 = a_in2;\n    }\n"
            } else {
                "    for (int li = 0; li < 0; li++)\n    {\n        t1 += a_in2;\n    }\n"
            });
        }
        let statements = 3 + self.pick(5);
        for _ in 0..statements {
            match self.pick(10) {
                0..=4 => self.if_stmt(0, store, "    ", &mut s),
                5 | 6 => self.loop_stmt(&mut s),
                _ => s.push_str(&format!("    {}\n", self.simple(store))),
            }
        }
        if self.fragment {
            if !store {
                /* one of the values the body left, scaled into the colour's
                 * range: the comparison is of the packed pixel */
                let v = match self.pick(5) {
                    0 => "t0",
                    1 => "t1",
                    2 => "t2",
                    3 => "vec4(f0, f1, float(i0), 1.0)",
                    _ => "(t0 + t1 * 0.5 - t2 * f0)",
                };
                let scale = self.of(&["0.125", "0.0625", "0.25"]);
                s.push_str(&format!("    gl_FragColor = {v} * {scale} + 0.5;\n"));
            }
        } else {
            s.push_str("    v_out0 = t0;\n    v_out1 = t1;\n    v_out2 = t2 * f0 + vec4(f1, float(i0), f0, 1.0);\n");
            if !store {
                s.push_str("    v_out3 = vec4(f0, f1, float(i0), 1.0);\n");
            }
            s.push_str("    gl_Position = a_position;\n");
        }
        s.push_str("}\n");
        s
    }

    /// A local's first value: an input or a uniform as it is.
    fn vec4_input(&mut self) -> String {
        let inputs: [&str; 2] = if self.fragment { ["v_in0", "v_in1"] } else { ["a_in0", "a_in1"] };
        match self.pick(4) {
            0 => self.of(&["u_v0", "u_v1"]).to_string(),
            1 => format!("{}.{}", self.of(&inputs), self.of(&SWIZZLES4)),
            _ => self.of(&inputs).to_string(),
        }
    }
}

/// The shader of `seed` for a stage.
pub fn shader(seed: u32, fragment: bool) -> Shader {
    let mut g = Gen {
        x: 0x9e37_79b9_7f4a_7c15 ^ ((u64::from(seed) << 1) | u64::from(fragment)).wrapping_mul(0xbf58_476d_1ce4_e5b9),
        fragment,
        counter: None,
        loops: 0,
    };
    for _ in 0..4 {
        g.next();
    }
    Shader {
        name: format!("deqp_style.{}.{seed}", if fragment { "fragment" } else { "vertex" }),
        fragment,
        text: g.shader(),
    }
}

/// How many seeds a test compiles: `default`, or `DEQP_STYLE_SEEDS=<n>` for
/// a longer soak than the gate's (every seed is a vertex and a fragment
/// shader).
pub fn seeds(default: u32) -> u32 {
    std::env::var("DEQP_STYLE_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// `n` vertex and `n` fragment shaders, seeds 0..n.
pub fn population(n: u32) -> Vec<Shader> {
    (0..n).flat_map(|seed| [shader(seed, false), shader(seed, true)]).collect()
}

/// How many of `shaders` hold each shape the generator is for, so a test
/// can say its population was not empty of them.
pub fn shape_counts(shaders: &[Shader]) -> [(&'static str, usize); 10] {
    /* in `main`: the two functions above it are in every source */
    let count = |needles: &[&str]| {
        shaders
            .iter()
            .filter(|s| {
                let body = s.text.split_once("void main()").map_or("", |x| x.1);
                needles.iter().any(|n| body.contains(n))
            })
            .count()
    };
    [
        ("an if", count(&["if ("])),
        ("an else", count(&["else"])),
        ("a product by 1.0", count(&["* 1.0)", "(1.0 * "])),
        ("a clamp to [0, 1]", count(&[", 0.0, 1.0)", ", 0.0), 1.0)", ", 1.0), 0.0)"])),
        ("a loop", count(&["for (", "do\n"])),
        ("a loop bounded by a uniform or an input", count(&["< ui_b;", "< ui_b)", "< int(clamp("])),
        ("a break or a continue in a loop", count(&["break;", "continue;"])),
        ("a call", count(&["fn_sel(", "fn_mad("])),
        ("a discard", count(&["discard;"])),
        ("an attribute read only in code that never runs", count(&["a_in2;"])),
    ]
}

/// dEQP's own ES 1.00 programs, when `DEQP_PROGRAMS` names the JSON-lines
/// file of `{name, vert, frag}` that docs/notes/usc-backend.md
/// ("dEQP's shaders against the IR") builds from a VK-GL-CTS checkout's
/// `data/gles2/shaders/*.test`: (name, vertex source, fragment source) per
/// line. `None` without the variable: a test then measures nothing and
/// says so.
pub fn deqp_programs() -> Option<Vec<(String, String, String)>> {
    let path = std::env::var("DEQP_PROGRAMS").ok()?;
    let text = std::fs::read_to_string(&path).expect("the programs file");
    let field = |line: &str, key: &str| -> String {
        let at = line.find(&format!("\"{key}\": \"")).unwrap_or_else(|| panic!("no {key} in {line:.80}"));
        let mut out = String::new();
        let mut chars = line[at + key.len() + 5..].chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).expect("a \\u escape"));
                    }
                    Some(o) => out.push(o),
                    None => break,
                },
                c => out.push(c),
            }
        }
        out
    };
    Some(text.lines().map(|l| (field(l, "name"), field(l, "vert"), field(l, "frag"))).collect())
}
