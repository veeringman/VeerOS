//! Shell scripting engine for VeerOS.
//!
//! Provides variables, control flow (if/elif/else/fi, while/do/done,
//! for/do/done), and script file execution.
//! All `no_std`, fixed-size buffers, zero allocation.

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of user variables.
const MAX_VARS: usize = 32;
/// Maximum length of a variable name.
const MAX_NAME: usize = 16;
/// Maximum length of a variable value.
const MAX_VAL: usize = 64;
/// Maximum nesting depth for if/while/for.
const MAX_NEST: usize = 8;
/// Maximum number of lines in a script (for while/for buffering).
const MAX_SCRIPT_LINES: usize = 128;
/// Maximum line length in a script.
const MAX_LINE: usize = 128;

// ═══════════════════════════════════════════════════════════════════════════
// Variable store
// ═══════════════════════════════════════════════════════════════════════════

/// A single shell variable.
struct Var {
    name: [u8; MAX_NAME],
    name_len: usize,
    val: [u8; MAX_VAL],
    val_len: usize,
}

impl Var {
    const fn empty() -> Self {
        Self {
            name: [0u8; MAX_NAME],
            name_len: 0,
            val: [0u8; MAX_VAL],
            val_len: 0,
        }
    }

    fn name_eq(&self, s: &[u8]) -> bool {
        self.name_len == s.len() && self.name[..self.name_len] == *s
    }
}

/// Fixed-capacity variable store.
pub struct VarStore {
    vars: [Var; MAX_VARS],
    count: usize,
    /// Last command exit status (0 = success).
    pub last_status: u8,
}

impl VarStore {
    pub const fn new() -> Self {
        const EMPTY: Var = Var::empty();
        Self {
            vars: [EMPTY; MAX_VARS],
            count: 0,
            last_status: 0,
        }
    }

    /// Set a variable. Creates it if it doesn't exist.
    pub fn set(&mut self, name: &str, val: &str) {
        let nb = name.as_bytes();
        let vb = val.as_bytes();
        if nb.is_empty() || nb.len() > MAX_NAME { return; }
        let vlen = if vb.len() > MAX_VAL { MAX_VAL } else { vb.len() };

        // Update existing
        for i in 0..self.count {
            if self.vars[i].name_eq(nb) {
                self.vars[i].val[..vlen].copy_from_slice(&vb[..vlen]);
                self.vars[i].val_len = vlen;
                return;
            }
        }
        // Create new
        if self.count < MAX_VARS {
            let v = &mut self.vars[self.count];
            v.name[..nb.len()].copy_from_slice(nb);
            v.name_len = nb.len();
            v.val[..vlen].copy_from_slice(&vb[..vlen]);
            v.val_len = vlen;
            self.count += 1;
        }
    }

    /// Get a variable value. Returns empty str if not found.
    pub fn get<'a>(&'a self, name: &str) -> &'a str {
        let nb = name.as_bytes();
        for i in 0..self.count {
            if self.vars[i].name_eq(nb) {
                return unsafe {
                    core::str::from_utf8_unchecked(&self.vars[i].val[..self.vars[i].val_len])
                };
            }
        }
        ""
    }

    /// Remove a variable.
    pub fn unset(&mut self, name: &str) {
        let nb = name.as_bytes();
        for i in 0..self.count {
            if self.vars[i].name_eq(nb) {
                // Shift remaining
                for j in i..self.count - 1 {
                    self.vars[j] = self.vars[j + 1];
                }
                self.vars[self.count - 1] = Var::empty();
                self.count -= 1;
                return;
            }
        }
    }

    /// Iterate over all variables.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        (0..self.count).map(move |i| {
            let v = &self.vars[i];
            let name = unsafe { core::str::from_utf8_unchecked(&v.name[..v.name_len]) };
            let val = unsafe { core::str::from_utf8_unchecked(&v.val[..v.val_len]) };
            (name, val)
        })
    }

    /// Number of defined variables.
    pub fn len(&self) -> usize {
        self.count
    }
}

// Implement Copy for Var to allow array shifts
impl Copy for Var {}
impl Clone for Var {
    fn clone(&self) -> Self { *self }
}

// ═══════════════════════════════════════════════════════════════════════════
// Variable expansion
// ═══════════════════════════════════════════════════════════════════════════

/// Expand `$name`, `${name}`, and `$?` in a string.
/// Single-quoted strings are passed through literally (no expansion).
/// Writes result to `out` buffer, returns length.
pub fn expand_vars(input: &str, vars: &VarStore, out: &mut [u8]) -> usize {
    let bytes = input.as_bytes();
    let mut pos = 0;
    let mut i = 0;
    let mut in_single_quote = false;

    while i < bytes.len() && pos < out.len() {
        // Single-quote toggle (only outside double-quotes for simplicity)
        if bytes[i] == b'\'' {
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }

        // Inside single quotes: literal copy, no expansion
        if in_single_quote {
            out[pos] = bytes[i];
            pos += 1;
            i += 1;
            continue;
        }

        if bytes[i] == b'$' {
            i += 1;
            if i >= bytes.len() { break; }

            // $? — last exit status
            if bytes[i] == b'?' {
                i += 1;
                let mut tmp = [0u8; 4];
                let n = fmt_u8(&mut tmp, vars.last_status);
                let take = if n > out.len() - pos { out.len() - pos } else { n };
                out[pos..pos + take].copy_from_slice(&tmp[..take]);
                pos += take;
                continue;
            }

            // ${name} — braced variable
            if bytes[i] == b'{' {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                if i < bytes.len() {
                    if let Ok(name) = core::str::from_utf8(&bytes[start..i]) {
                        let val = vars.get(name);
                        let vb = val.as_bytes();
                        let take = if vb.len() > out.len() - pos { out.len() - pos } else { vb.len() };
                        out[pos..pos + take].copy_from_slice(&vb[..take]);
                        pos += take;
                    }
                    i += 1; // skip '}'
                }
                continue;
            }

            // $name — bare variable (alphanumeric + _)
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            if i > start {
                if let Ok(name) = core::str::from_utf8(&bytes[start..i]) {
                    let val = vars.get(name);
                    let vb = val.as_bytes();
                    let take = if vb.len() > out.len() - pos { out.len() - pos } else { vb.len() };
                    out[pos..pos + take].copy_from_slice(&vb[..take]);
                    pos += take;
                }
            }
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            // Escaped dollar: \$
            out[pos] = b'$';
            pos += 1;
            i += 2;
        } else {
            out[pos] = bytes[i];
            pos += 1;
            i += 1;
        }
    }
    pos
}

// ═══════════════════════════════════════════════════════════════════════════
// Test evaluator (for `if` / `while` conditions)
// ═══════════════════════════════════════════════════════════════════════════

/// Evaluate a test expression. Returns true/false.
///
/// Supported:
///   - `test <expr>` or `[ <expr> ]`
///   - String: `-z str`, `-n str`, `str1 = str2`, `str1 != str2`
///   - Numeric: `n1 -eq n2`, `-ne`, `-lt`, `-le`, `-gt`, `-ge`
///   - File: `-e path`, `-f path`, `-d path` (via callback)
///   - Logic: `! expr`, `expr -a expr`, `expr -o expr`
///   - Bare word: true if non-empty
pub fn eval_test(args: &str, file_exists: Option<fn(&str) -> bool>) -> bool {
    let trimmed = args.trim();

    // Handle `[ ... ]` syntax — strip brackets
    let expr = if trimmed.starts_with('[') && trimmed.ends_with(']') {
        trimmed[1..trimmed.len() - 1].trim()
    } else if trimmed.starts_with("test ") {
        &trimmed[5..]
    } else {
        trimmed
    };

    eval_expr(expr, file_exists)
}

fn eval_expr(expr: &str, file_exists: Option<fn(&str) -> bool>) -> bool {
    let expr = expr.trim();
    if expr.is_empty() { return false; }

    // Handle -o (OR) — lowest precedence
    if let Some(pos) = find_operator(expr, " -o ") {
        return eval_expr(&expr[..pos], file_exists)
            || eval_expr(&expr[pos + 4..], file_exists);
    }
    // Handle -a (AND)
    if let Some(pos) = find_operator(expr, " -a ") {
        return eval_expr(&expr[..pos], file_exists)
            && eval_expr(&expr[pos + 4..], file_exists);
    }
    // Handle ! (NOT)
    if expr.starts_with("! ") {
        return !eval_expr(&expr[2..], file_exists);
    }

    // Split into words
    let mut words = [("", 0usize); 4];
    let wc = split_words(expr, &mut words);

    match wc {
        1 => {
            let w = word_at(expr, &words, 0);
            // Non-empty string is true, "0" and "" are false
            !w.is_empty() && w != "0"
        }
        2 => {
            let op = word_at(expr, &words, 0);
            let arg = word_at(expr, &words, 1);
            match op {
                "-z" => arg.is_empty(),
                "-n" => !arg.is_empty(),
                "-e" | "-f" | "-d" => {
                    if let Some(f) = file_exists { f(arg) } else { false }
                }
                _ => !op.is_empty(), // fallback: non-empty is true
            }
        }
        3 => {
            let a = word_at(expr, &words, 0);
            let op = word_at(expr, &words, 1);
            let b = word_at(expr, &words, 2);
            match op {
                "=" | "==" => a == b,
                "!=" => a != b,
                "-eq" => parse_i32(a) == parse_i32(b),
                "-ne" => parse_i32(a) != parse_i32(b),
                "-lt" => parse_i32(a) < parse_i32(b),
                "-le" => parse_i32(a) <= parse_i32(b),
                "-gt" => parse_i32(a) > parse_i32(b),
                "-ge" => parse_i32(a) >= parse_i32(b),
                _ => false,
            }
        }
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Script line buffer (for control-flow blocks)
// ═══════════════════════════════════════════════════════════════════════════

/// A single buffered script line.
#[derive(Copy, Clone)]
struct ScriptLine {
    data: [u8; MAX_LINE],
    len: usize,
}

impl ScriptLine {
    const fn empty() -> Self {
        Self { data: [0u8; MAX_LINE], len: 0 }
    }

    fn set(&mut self, s: &str) {
        let b = s.as_bytes();
        let take = if b.len() > MAX_LINE { MAX_LINE } else { b.len() };
        self.data[..take].copy_from_slice(&b[..take]);
        self.len = take;
    }

    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.data[..self.len]) }
    }
}

/// Control-flow type.
#[derive(Copy, Clone, PartialEq)]
pub enum BlockKind {
    If,
    While,
    For,
}

/// State within one nesting level.
#[derive(Copy, Clone)]
pub struct BlockState {
    pub kind: BlockKind,
    /// For if: have we found a true branch yet?
    pub if_taken: bool,
    /// Are we in the active (executing) branch?
    pub active: bool,
    /// For while: line index where the condition starts.
    pub loop_start: usize,
    /// For while: the condition string.
    pub cond: [u8; MAX_LINE],
    pub cond_len: usize,
    /// For `for`: iteration variable name.
    pub for_var: [u8; MAX_NAME],
    pub for_var_len: usize,
    /// For `for`: word list string.
    pub for_list: [u8; MAX_VAL],
    pub for_list_len: usize,
    /// For `for`: current word index in the list.
    pub for_idx: usize,
}

impl BlockState {
    const fn empty() -> Self {
        Self {
            kind: BlockKind::If,
            if_taken: false,
            active: true,
            loop_start: 0,
            cond: [0u8; MAX_LINE],
            cond_len: 0,
            for_var: [0u8; MAX_NAME],
            for_var_len: 0,
            for_list: [0u8; MAX_VAL],
            for_list_len: 0,
            for_idx: 0,
        }
    }

    pub fn cond_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.cond[..self.cond_len]) }
    }
}

/// Script execution context — manages nesting and buffered lines.
pub struct ScriptCtx {
    /// Stack of nested blocks.
    pub blocks: [BlockState; MAX_NEST],
    /// Current nesting depth (0 = top level).
    pub depth: usize,
    /// Buffered script lines (for loops).
    pub lines: [ScriptLine; MAX_SCRIPT_LINES],
    pub line_count: usize,
    /// Current execution position in lines.
    pub pc: usize,
    /// Are we buffering lines for a block?
    pub buffering: bool,
    /// Depth when buffering started.
    pub buf_start_depth: usize,
}

impl ScriptCtx {
    pub const fn new() -> Self {
        const EMPTY_BLOCK: BlockState = BlockState::empty();
        const EMPTY_LINE: ScriptLine = ScriptLine::empty();
        Self {
            blocks: [EMPTY_BLOCK; MAX_NEST],
            depth: 0,
            lines: [EMPTY_LINE; MAX_SCRIPT_LINES],
            line_count: 0,
            pc: 0,
            buffering: false,
            buf_start_depth: 0,
        }
    }

    /// Check if execution is currently active (not in a skipped branch).
    pub fn is_active(&self) -> bool {
        if self.depth == 0 { return true; }
        self.blocks[self.depth - 1].active
    }

    /// Push a new block level.
    pub fn push_block(&mut self, kind: BlockKind) -> bool {
        if self.depth >= MAX_NEST { return false; }
        self.blocks[self.depth] = BlockState::empty();
        self.blocks[self.depth].kind = kind;
        // Inherit parent active state
        self.blocks[self.depth].active = self.is_active();
        self.depth += 1;
        true
    }

    /// Pop a block level.
    pub fn pop_block(&mut self) -> Option<BlockKind> {
        if self.depth == 0 { return None; }
        self.depth -= 1;
        Some(self.blocks[self.depth].kind)
    }

    /// Current block (mutable).
    pub fn current_mut(&mut self) -> Option<&mut BlockState> {
        if self.depth > 0 {
            Some(&mut self.blocks[self.depth - 1])
        } else {
            None
        }
    }

    /// Current block (immutable).
    pub fn current(&self) -> Option<&BlockState> {
        if self.depth > 0 {
            Some(&self.blocks[self.depth - 1])
        } else {
            None
        }
    }

    /// Buffer a line.
    pub fn push_line(&mut self, line: &str) {
        if self.line_count < MAX_SCRIPT_LINES {
            self.lines[self.line_count].set(line);
            self.line_count += 1;
        }
    }

    /// Reset the line buffer.
    pub fn reset_lines(&mut self) {
        self.line_count = 0;
        self.pc = 0;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Arithmetic evaluator (for `let`)
// ═══════════════════════════════════════════════════════════════════════════

/// Evaluate simple integer arithmetic: +, -, *, /, %.
/// Returns the result as i32.
pub fn eval_arith(expr: &str) -> i32 {
    let expr = expr.trim();

    // Try to find + or - (not at start) for addition/subtraction
    // Scan right-to-left so left-associativity works
    let bytes = expr.as_bytes();
    let mut paren_depth = 0i32;
    let mut i = bytes.len() as i32 - 1;
    while i > 0 {
        let idx = i as usize;
        match bytes[idx] {
            b')' => paren_depth += 1,
            b'(' => paren_depth -= 1,
            b'+' | b'-' if paren_depth == 0 => {
                let left = &expr[..idx];
                let right = &expr[idx + 1..];
                if !left.is_empty() {
                    if bytes[idx] == b'+' {
                        return eval_arith(left).wrapping_add(eval_arith(right));
                    } else {
                        return eval_arith(left).wrapping_sub(eval_arith(right));
                    }
                }
            }
            _ => {}
        }
        i -= 1;
    }

    // Try *, /, %
    i = bytes.len() as i32 - 1;
    paren_depth = 0;
    while i > 0 {
        let idx = i as usize;
        match bytes[idx] {
            b')' => paren_depth += 1,
            b'(' => paren_depth -= 1,
            b'*' | b'/' | b'%' if paren_depth == 0 => {
                let left = &expr[..idx];
                let right = &expr[idx + 1..];
                if !left.is_empty() {
                    let r = eval_arith(right);
                    return match bytes[idx] {
                        b'*' => eval_arith(left).wrapping_mul(r),
                        b'/' => if r != 0 { eval_arith(left) / r } else { 0 },
                        b'%' => if r != 0 { eval_arith(left) % r } else { 0 },
                        _ => 0,
                    };
                }
            }
            _ => {}
        }
        i -= 1;
    }

    // Parentheses
    if bytes.first() == Some(&b'(') && bytes.last() == Some(&b')') {
        return eval_arith(&expr[1..expr.len() - 1]);
    }

    // Base case: parse integer
    parse_i32(expr)
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn fmt_u8(buf: &mut [u8; 4], v: u8) -> usize {
    if v >= 100 {
        buf[0] = b'0' + v / 100;
        buf[1] = b'0' + (v / 10) % 10;
        buf[2] = b'0' + v % 10;
        3
    } else if v >= 10 {
        buf[0] = b'0' + v / 10;
        buf[1] = b'0' + v % 10;
        2
    } else {
        buf[0] = b'0' + v;
        1
    }
}

fn parse_i32(s: &str) -> i32 {
    let s = s.trim();
    if s.is_empty() { return 0; }
    let (neg, s) = if s.as_bytes()[0] == b'-' {
        (true, &s[1..])
    } else {
        (false, s)
    };
    // Handle 0x prefix for hex
    let (radix, s) = if s.len() > 2 && s.as_bytes()[0] == b'0'
        && (s.as_bytes()[1] == b'x' || s.as_bytes()[1] == b'X')
    {
        (16, &s[2..])
    } else {
        (10, s)
    };
    let mut result: i32 = 0;
    for &b in s.as_bytes() {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as i32,
            b'a'..=b'f' if radix == 16 => (b - b'a' + 10) as i32,
            b'A'..=b'F' if radix == 16 => (b - b'A' + 10) as i32,
            _ => break,
        };
        result = result.wrapping_mul(radix).wrapping_add(digit);
    }
    if neg { -result } else { result }
}

/// Format i32 to buffer, return slice.
pub fn fmt_i32(buf: &mut [u8; 12], v: i32) -> usize {
    if v == 0 {
        buf[0] = b'0';
        return 1;
    }
    let neg = v < 0;
    let mut val = if neg { (-(v as i64)) as u32 } else { v as u32 };
    let mut pos = 12;
    while val > 0 && pos > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    if neg && pos > 0 {
        pos -= 1;
        buf[pos] = b'-';
    }
    let len = 12 - pos;
    // Shift to front
    if pos > 0 {
        for i in 0..len {
            buf[i] = buf[pos + i];
        }
    }
    len
}

fn find_operator(s: &str, op: &str) -> Option<usize> {
    s.find(op)
}

/// Split a string into whitespace-separated words (up to `out.len()`).
/// Returns word count. Each entry stores (start_offset, len).
fn split_words<'a>(s: &'a str, out: &mut [(&'a str, usize)]) -> usize {
    let mut count = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() && count < out.len() {
        // Skip whitespace
        while i < bytes.len() && bytes[i] == b' ' { i += 1; }
        if i >= bytes.len() { break; }
        let start = i;
        // Handle quoted strings
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let content_start = i;
            while i < bytes.len() && bytes[i] != quote { i += 1; }
            out[count] = (&s[content_start..i], i - content_start);
            if i < bytes.len() { i += 1; } // skip closing quote
        } else {
            while i < bytes.len() && bytes[i] != b' ' { i += 1; }
            out[count] = (&s[start..i], i - start);
        }
        count += 1;
    }
    count
}

fn word_at<'a>(_s: &'a str, words: &[(&'a str, usize)], idx: usize) -> &'a str {
    if idx < words.len() { words[idx].0 } else { "" }
}

/// Get the Nth word from a whitespace-separated string.
pub fn get_word(s: &str, n: usize) -> &str {
    let mut count = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b' ' { i += 1; }
        if i >= bytes.len() { break; }
        let start = i;
        while i < bytes.len() && bytes[i] != b' ' { i += 1; }
        if count == n {
            return &s[start..i];
        }
        count += 1;
    }
    ""
}

/// Count whitespace-separated words.
pub fn word_count(s: &str) -> usize {
    let mut count = 0;
    let mut in_word = false;
    for &b in s.as_bytes() {
        if b == b' ' {
            in_word = false;
        } else if !in_word {
            in_word = true;
            count += 1;
        }
    }
    count
}

/// Get all words starting from index N as a sub-slice.
pub fn words_from(s: &str, n: usize) -> &str {
    let mut count = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b' ' { i += 1; }
        if i >= bytes.len() { break; }
        if count == n { return &s[i..]; }
        while i < bytes.len() && bytes[i] != b' ' { i += 1; }
        count += 1;
    }
    ""
}
