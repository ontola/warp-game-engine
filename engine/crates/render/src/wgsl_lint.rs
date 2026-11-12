//! A check for the WGSL rule naga lets through and browsers (Tint) do not:
//! operators that the WGSL grammar keeps apart may not be mixed without
//! parentheses. `a * b ^ c`, `a & b | c`, `a & b == c`, `a << b + c`,
//! `a < b < c` and `a && b || c` all parse in naga and fail to parse in
//! Chrome, Safari and Firefox, so the shader (shade, the whole lighting
//! pass) is invalid there.
//!
//! The grammar (WGSL spec, "Expressions"): a bitwise expression is unary
//! operands joined by one bitwise operator (`&`, `|` or `^`), and it is a
//! whole expression, not an operand of anything else; a shift takes two
//! unary operands; a relational expression compares two shift expressions;
//! `&&` and `||` join relational expressions and do not mix.
//!
//! [`check`] tokenizes a module and applies these rules to every operator
//! sequence between brackets, commas, semicolons and assignments. The test
//! below runs it over every `.wgsl` file in the repository and every WGSL
//! module written inline in the renderer's Rust sources.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok<'a> { Word(&'a str), Num, Op(&'a str), Open(char), Close(char), Sep }

const OPS: [&str; 30] = ["<<=", ">>=", "->", "&&", "||", "<<", ">>", "<=", ">=", "==", "!=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "++", "--", "+", "-", "*", "/", "%", "&", "|", "^", "!"];

fn tokens(src: &str) -> Vec<(Tok<'_>, usize)> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < b.len() {
        let c = b[i] as char;
        if c == '\n' { line += 1; i += 1; continue; }
        if c.is_whitespace() { i += 1; continue; }
        if b[i..].starts_with(b"//") { while i < b.len() && b[i] != b'\n' { i += 1; } continue; }
        if b[i..].starts_with(b"/*") {
            let mut depth = 0;
            while i < b.len() {
                if b[i..].starts_with(b"/*") { depth += 1; i += 2; } else if b[i..].starts_with(b"*/") { depth -= 1; i += 2; if depth == 0 { break; } }
                else { if b[i] == b'\n' { line += 1; } i += 1; }
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            out.push((Tok::Word(&src[s..i]), line));
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < b.len() && (b[i + 1] as char).is_ascii_digit()) {
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'.'
                || ((b[i] == b'+' || b[i] == b'-') && matches!(b[i - 1], b'e' | b'E' | b'p' | b'P') && !b[..i].ends_with(b"0x"))) { i += 1; }
            out.push((Tok::Num, line));
            continue;
        }
        match c {
            '(' | '[' | '{' => { out.push((Tok::Open(c), line)); i += 1; continue; }
            ')' | ']' | '}' => { out.push((Tok::Close(c), line)); i += 1; continue; }
            ',' | ';' | ':' | '@' | '.' => { out.push((Tok::Sep, line)); i += 1; continue; }
            _ => {}
        }
        if let Some(op) = OPS.iter().find(|op| b[i..].starts_with(op.as_bytes())) { out.push((Tok::Op(op), line)); i += op.len(); continue; }
        if c == '=' || c == '<' || c == '>' { out.push((Tok::Op(&src[i..i + 1]), line)); i += 1; continue; }
        out.push((Tok::Sep, line));
        i += 1;
    }
    out
}

/// Types whose `<` opens a template list, not a comparison.
fn templated(word: &str) -> bool {
    matches!(word, "vec2" | "vec3" | "vec4" | "array" | "ptr" | "atomic" | "bitcast")
        || word.starts_with("mat") && word.len() == 6
        || word.starts_with("texture_")
}

/// The mixed operators in `src`, as (line, operators) for each offending
/// expression level.
pub fn check(src: &str) -> Vec<(usize, String)> {
    let toks = tokens(src);
    // Drop template lists: `vec3<f32>`, `array<vec4<f32>, 16>`.
    let mut t: Vec<(Tok, usize)> = Vec::with_capacity(toks.len());
    let mut k = 0;
    while k < toks.len() {
        if let (Tok::Word(w), Some((Tok::Op("<"), _))) = (toks[k].0, toks.get(k + 1)) && templated(w) {
            t.push(toks[k]);
            let mut depth = 0i32;
            k += 1;
            while k < toks.len() {
                match toks[k].0 {
                    Tok::Op("<") => depth += 1,
                    Tok::Op(">") => { depth -= 1; if depth == 0 { k += 1; break; } }
                    Tok::Op(">>") => { depth -= 2; if depth <= 0 { k += 1; break; } }
                    _ => {}
                }
                k += 1;
            }
            continue;
        }
        t.push(toks[k]);
        k += 1;
    }
    let mut problems = Vec::new();
    // Per open bracket level, the binary operators of the current level.
    let mut stack: Vec<Vec<(&str, usize)>> = vec![Vec::new()];
    let mut prev_operand = false;
    let flush = |ops: &mut Vec<(&str, usize)>, problems: &mut Vec<(usize, String)>| {
        if let Some(e) = level_error(ops) { problems.push((ops[0].1, e)); }
        ops.clear();
    };
    for (tok, line) in t {
        match tok {
            Tok::Open(_) => { stack.push(Vec::new()); prev_operand = false; }
            Tok::Close(_) => {
                if let Some(mut ops) = stack.pop() { flush(&mut ops, &mut problems); }
                if stack.is_empty() { stack.push(Vec::new()); }
                prev_operand = true;
            }
            Tok::Sep => { flush(stack.last_mut().unwrap(), &mut problems); prev_operand = false; }
            Tok::Word(w) => {
                if matches!(w, "return" | "let" | "var" | "const" | "if" | "else" | "for" | "while" | "loop" | "switch" | "case" | "default" | "fn" | "struct" | "break" | "continue" | "discard") {
                    flush(stack.last_mut().unwrap(), &mut problems);
                    prev_operand = false;
                } else { prev_operand = true; }
            }
            Tok::Num => prev_operand = true,
            Tok::Op(op) => {
                let assign = op == "=" || (op.ends_with('=') && !matches!(op, "==" | "!=" | "<=" | ">="));
                if assign || op == "->" || op == "++" || op == "--" { flush(stack.last_mut().unwrap(), &mut problems); prev_operand = false; continue; }
                if prev_operand && op != "!" { stack.last_mut().unwrap().push((op, line)); }
                prev_operand = false;
            }
        }
    }
    for mut ops in stack { flush(&mut ops, &mut problems); }
    problems
}

fn level_error(ops: &[(&str, usize)]) -> Option<String> {
    if ops.len() < 2 { return None; }
    let names: Vec<&str> = ops.iter().map(|o| o.0).collect();
    let all = names.join(" ");
    let bitwise = |o: &str| matches!(o, "&" | "|" | "^");
    let logical = |o: &str| matches!(o, "&&" | "||");
    let relational = |o: &str| matches!(o, "==" | "!=" | "<" | ">" | "<=" | ">=");
    let shift = |o: &str| matches!(o, "<<" | ">>");
    if names.iter().any(|o| bitwise(o)) {
        // A bitwise expression is one operator over unary operands.
        if names.iter().any(|o| *o != names[0]) { return Some(format!("bitwise operator mixed with others: {all}")); }
        return None;
    }
    if names.contains(&"&&") && names.contains(&"||") { return Some(format!("&& and || mixed: {all}")); }
    for rel in names.split(|o| logical(o)) {
        if rel.iter().filter(|o| relational(o)).count() > 1 { return Some(format!("chained comparison: {all}")); }
        for sh in rel.split(|o| relational(o)) {
            if sh.iter().any(|o| shift(o)) && sh.len() > 1 { return Some(format!("shift mixed with others: {all}")); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_what_tint_rejects() {
        for bad in ["let a = b * c ^ d;", "let a = b & c | d;", "let a = b & c == d;", "let a = b << c + d;", "let a = b + c >> d;",
            "if a < b < c {}", "if a && b || c {}", "let a = f(x * 2u ^ y);", "let a = v[i & 3u + 1u];"] {
            assert!(!check(bad).is_empty(), "accepted: {bad}");
        }
        for good in ["let a = (b * c) ^ d;", "let a = b ^ c ^ d;", "let a = (b & c) == d;", "let a = b << (c + d);", "if a < b && c < d {}",
            "let v: vec3<f32> = vec3<f32>(a * b + c, -d, e);", "let m = array<vec4<f32>, 16>();", "x &= y; y |= 1u;",
            "let p: ptr<function, vec3<f32>> = &v;", "fn f(a: i32) -> i32 { return -a * 2 + (b >> 1u); }",
            "if (a & 1u) != 0u {}", "let s = (x >> ((x >> 28u) + 4u)) ^ x;", "let b = a > 0.0 && c >= 1.0;", "let e = 1e-3 * x;",
            "out[i * 6u + a] = p;", "let c = bitcast<vec4<f32>>(w);", "if !(a < b) {}"] {
            assert!(check(good).is_empty(), "rejected: {good}: {:?}", check(good));
        }
    }

    fn rust_wgsl(src: &str) -> Vec<String> {
        // String literals (plain or raw) that hold WGSL entry points.
        let mut out = Vec::new();
        let b = src.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'r' && i + 1 < b.len() && (b[i + 1] == b'"' || b[i + 1] == b'#') && (i == 0 || !(b[i - 1] as char).is_ascii_alphanumeric()) {
                let mut j = i + 1;
                let mut hashes = 0;
                while j < b.len() && b[j] == b'#' { hashes += 1; j += 1; }
                if j < b.len() && b[j] == b'"' {
                    let close = format!("\"{}", "#".repeat(hashes));
                    if let Some(end) = src[j + 1..].find(&close) { out.push(src[j + 1..j + 1 + end].to_owned()); i = j + 1 + end + close.len(); continue; }
                }
            }
            if b[i] == b'"' && (i == 0 || b[i - 1] != b'\'') {
                let mut j = i + 1;
                let mut s = String::new();
                while j < b.len() && b[j] != b'"' { if b[j] == b'\\' { j += 1; if j < b.len() && b[j] == b'\n' { j += 1; continue; } } if j < b.len() { s.push(b[j] as char); } j += 1; }
                out.push(s);
                i = j + 1;
                continue;
            }
            if b[i..].starts_with(b"//") { while i < b.len() && b[i] != b'\n' { i += 1; } continue; }
            i += 1;
        }
        out.retain(|s| s.contains("@compute") || s.contains("@fragment") || s.contains("@vertex"));
        out
    }

    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() { if !name.starts_with('.') && !name.starts_with("target") && name != "build" && name != "node_modules" { walk(&p, out); } }
            else if name.ends_with(".wgsl") { out.push(p); }
        }
    }

    /// Every WGSL in the repository parses in browsers as far as this rule goes.
    #[test]
    fn no_mixed_operators_in_any_shader() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut files = Vec::new();
        walk(&root, &mut files);
        assert!(files.len() > 20, "found the shaders");
        let mut failures = Vec::new();
        for f in &files {
            let src = std::fs::read_to_string(f).unwrap();
            for (line, e) in check(&src) { failures.push(format!("{}:{line}: {e}", f.display())); }
        }
        let mut rs = Vec::new();
        fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { rs_files(&p, out); } else if p.extension().is_some_and(|x| x == "rs") { out.push(p); }
            }
        }
        rs_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut rs);
        for f in &rs {
            if f.ends_with("wgsl_lint.rs") { continue; }
            for module in rust_wgsl(&std::fs::read_to_string(f).unwrap()) {
                for (line, e) in check(&module) { failures.push(format!("{} (inline WGSL, line {line}): {e}", f.display())); }
            }
        }
        assert!(failures.is_empty(), "{} mixed-operator expressions browsers reject:\n{}", failures.len(), failures.join("\n"));
    }
}
