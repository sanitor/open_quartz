//! Lightweight lexical highlighting adapters for the native editor.
//!
//! This module intentionally does not validate either language.  A token is
//! emitted as soon as its lexical prefix is recognizable, which makes the
//! adapters useful while a document is incomplete or temporarily malformed.

use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Plain,
    Wgsl,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Keyword,
    Type,
    Attribute,
    Function,
    Number,
    String,
    Comment,
    Boolean,
    Punctuation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub category: Category,
}

pub fn highlight(source: &str, language: Language) -> Vec<Highlight> {
    match language {
        Language::Plain => Vec::new(),
        Language::Wgsl => highlight_wgsl(source),
        Language::Json => highlight_json(source),
    }
}

fn push(out: &mut Vec<Highlight>, start: usize, end: usize, category: Category) {
    if start < end {
        out.push(Highlight {
            range: start..end,
            category,
        });
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Returns the byte end of an identifier beginning at `start`.
fn identifier_end(source: &str, start: usize) -> usize {
    let mut end = start;
    let mut first = true;
    for (offset, c) in source[start..].char_indices() {
        if (first && is_ident_start(c)) || (!first && is_ident_continue(c)) {
            end = start + offset + c.len_utf8();
            first = false;
        } else {
            break;
        }
    }
    end
}

fn next_non_whitespace(source: &str, mut at: usize) -> Option<u8> {
    let bytes = source.as_bytes();
    while at < bytes.len() {
        let c = source[at..].chars().next()?;
        if !c.is_whitespace() {
            return Some(bytes[at]);
        }
        at += c.len_utf8();
    }
    None
}

const WGSL_KEYWORDS: &[&str] = &[
    "alias",
    "break",
    "case",
    "const",
    "continuing",
    "continue",
    "default",
    "discard",
    "else",
    "enable",
    "fallthrough",
    "fn",
    "for",
    "if",
    "let",
    "loop",
    "override",
    "requires",
    "return",
    "struct",
    "switch",
    "var",
    "while",
    "workgroup_size",
];

const WGSL_TYPES: &[&str] = &[
    "array",
    "atomic",
    "bool",
    "f16",
    "f32",
    "i32",
    "mat2x2",
    "mat2x3",
    "mat2x4",
    "mat3x2",
    "mat3x3",
    "mat3x4",
    "mat4x2",
    "mat4x3",
    "mat4x4",
    "ptr",
    "sampler",
    "sampler_comparison",
    "texture_1d",
    "texture_2d",
    "texture_2d_array",
    "texture_2d_ms",
    "texture_2d_ms_array",
    "texture_3d",
    "texture_cube",
    "texture_cube_array",
    "texture_depth_2d",
    "texture_depth_2d_array",
    "texture_depth_cube",
    "texture_depth_cube_array",
    "texture_storage_1d",
    "texture_storage_2d",
    "texture_storage_2d_array",
    "texture_storage_3d",
    "u32",
    "vec2",
    "vec3",
    "vec4",
    "vec2f",
    "vec3f",
    "vec4f",
    "vec2h",
    "vec3h",
    "vec4h",
    "vec2i",
    "vec3i",
    "vec4i",
    "vec2u",
    "vec3u",
    "vec4u",
];

const WGSL_BUILTINS: &[&str] = &[
    "abs",
    "acos",
    "acosh",
    "all",
    "any",
    "arrayLength",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "ceil",
    "clamp",
    "cos",
    "cosh",
    "countLeadingZeros",
    "countOneBits",
    "countTrailingZeros",
    "cross",
    "degrees",
    "determinant",
    "distance",
    "dot",
    "exp",
    "exp2",
    "faceForward",
    "firstLeadingBit",
    "firstTrailingBit",
    "floor",
    "fma",
    "fract",
    "frexp",
    "inverseSqrt",
    "ldexp",
    "length",
    "log",
    "log2",
    "max",
    "min",
    "mix",
    "normalize",
    "pack2x16float",
    "pack2x16snorm",
    "pack2x16unorm",
    "pack4x8snorm",
    "pack4x8unorm",
    "pow",
    "quantizeToF16",
    "radians",
    "reflect",
    "refract",
    "round",
    "select",
    "sign",
    "sin",
    "sinh",
    "smoothstep",
    "sqrt",
    "step",
    "tan",
    "tanh",
    "textureDimensions",
    "textureGather",
    "textureLoad",
    "textureNumLayers",
    "textureNumLevels",
    "textureNumSamples",
    "textureSample",
    "textureSampleBias",
    "textureSampleCompare",
    "textureSampleCompareLevel",
    "textureSampleGrad",
    "textureSampleLevel",
    "textureStore",
    "transpose",
    "trunc",
    "unpack2x16float",
    "unpack2x16snorm",
    "unpack2x16unorm",
    "unpack4x8snorm",
    "unpack4x8unorm",
];

fn contains(words: &[&str], word: &str) -> bool {
    words.iter().any(|candidate| *candidate == word)
}

fn highlight_wgsl(source: &str) -> Vec<Highlight> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;

    while at < bytes.len() {
        // Comments take precedence over every other lexical class.  WGSL's
        // block comments nest, including when the outer comment is incomplete.
        if bytes[at] == b'/' && at + 1 < bytes.len() && bytes[at + 1] == b'/' {
            let start = at;
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += source[at..].chars().next().map_or(1, char::len_utf8);
            }
            push(&mut out, start, at, Category::Comment);
            continue;
        }
        if bytes[at] == b'/' && at + 1 < bytes.len() && bytes[at + 1] == b'*' {
            let start = at;
            at += 2;
            let mut depth = 1usize;
            while at < bytes.len() && depth != 0 {
                if at + 1 < bytes.len() && bytes[at] == b'/' && bytes[at + 1] == b'*' {
                    depth += 1;
                    at += 2;
                } else if at + 1 < bytes.len() && bytes[at] == b'*' && bytes[at + 1] == b'/' {
                    depth -= 1;
                    at += 2;
                } else {
                    at += source[at..].chars().next().map_or(1, char::len_utf8);
                }
            }
            push(&mut out, start, at, Category::Comment);
            continue;
        }

        // Strings are accepted here even though WGSL's current core grammar
        // has few string-producing contexts; doing so prevents false keyword
        // colors in editor snippets and diagnostics.
        if bytes[at] == b'"' || bytes[at] == b'\'' {
            let start = at;
            let quote = bytes[at];
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at += 1;
                    if at < bytes.len() {
                        at += source[at..].chars().next().map_or(1, char::len_utf8);
                    }
                } else if bytes[at] == quote {
                    at += 1;
                    break;
                } else {
                    at += source[at..].chars().next().map_or(1, char::len_utf8);
                }
            }
            push(&mut out, start, at, Category::String);
            continue;
        }

        if bytes[at] == b'@' {
            let start = at;
            at += 1;
            let end = identifier_end(source, at);
            if end != at {
                at = end;
            }
            push(&mut out, start, at, Category::Attribute);
            continue;
        }

        if let Some(end) = consume_wgsl_number(source, at) {
            push(&mut out, at, end, Category::Number);
            at = end;
            continue;
        }

        let c = source[at..].chars().next().expect("valid UTF-8 offset");
        if is_ident_start(c) {
            let end = identifier_end(source, at);
            let word = &source[at..end];
            let category = if word == "true" || word == "false" {
                Some(Category::Boolean)
            } else if contains(WGSL_KEYWORDS, word) {
                Some(Category::Keyword)
            } else if contains(WGSL_TYPES, word) {
                Some(Category::Type)
            } else if contains(WGSL_BUILTINS, word)
                || next_non_whitespace(source, end) == Some(b'(')
            {
                // In addition to the built-in table, an identifier directly
                // followed by '(' is treated as a user function call.  This
                // deliberately favors useful editor feedback over compiler
                // certainty (declarations and macros are not distinguished).
                Some(Category::Function)
            } else {
                None
            };
            if let Some(category) = category {
                push(&mut out, at, end, category);
            }
            at = end;
            continue;
        }

        if is_wgsl_punctuation(bytes[at]) {
            let start = at;
            at += 1;
            while at < bytes.len()
                && is_wgsl_punctuation(bytes[at])
                && !(bytes[at] == b'/'
                    && at + 1 < bytes.len()
                    && (bytes[at + 1] == b'/' || bytes[at + 1] == b'*'))
            {
                at += 1;
            }
            push(&mut out, start, at, Category::Punctuation);
            continue;
        }

        at += c.len_utf8();
    }
    out
}

fn is_wgsl_punctuation(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'*'
            | b'+'
            | b','
            | b'-'
            | b'.'
            | b'/'
            | b':'
            | b';'
            | b'<'
            | b'='
            | b'>'
            | b'?'
            | b'['
            | b']'
            | b'^'
            | b'{'
            | b'}'
            | b'|'
            | b'~'
            | b'('
            | b')'
    )
}

fn consume_wgsl_number(source: &str, start: usize) -> Option<usize> {
    let b = source.as_bytes();
    let len = b.len();
    let mut at = start;
    let hex = b.get(at) == Some(&b'0') && matches!(b.get(at + 1), Some(b'x' | b'X'));

    if hex {
        at += 2;
        let before = at;
        while at < len && (b[at].is_ascii_hexdigit() || b[at] == b'_') {
            at += 1;
        }
        let mut had_dot = false;
        if at < len && b[at] == b'.' {
            had_dot = true;
            at += 1;
            while at < len && (b[at].is_ascii_hexdigit() || b[at] == b'_') {
                at += 1;
            }
        }
        let mantissa_end = at;
        let mut has_exponent = false;
        if at < len && matches!(b[at], b'p' | b'P') {
            let exponent = at;
            at += 1;
            if at < len && matches!(b[at], b'+' | b'-') {
                at += 1;
            }
            let exponent_digits = at;
            while at < len && (b[at].is_ascii_digit() || b[at] == b'_') {
                at += 1;
            }
            if at != exponent_digits {
                has_exponent = true;
            } else {
                at = exponent;
            }
        }
        if before == mantissa_end && !has_exponent {
            return Some(start + 1); // Keep malformed `0x` recoverable.
        }
        if has_exponent || had_dot {
            at = consume_f16_suffix(b, at);
        }
        return Some(at);
    }

    let starts_with_dot = b.get(at) == Some(&b'.') && b.get(at + 1).is_some_and(u8::is_ascii_digit);
    if !starts_with_dot && !b.get(at).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    if starts_with_dot {
        at += 1;
        while at < len && (b[at].is_ascii_digit() || b[at] == b'_') {
            at += 1;
        }
    } else {
        while at < len && (b[at].is_ascii_digit() || b[at] == b'_') {
            at += 1;
        }
        if at < len && b[at] == b'.' {
            at += 1;
            while at < len && (b[at].is_ascii_digit() || b[at] == b'_') {
                at += 1;
            }
        }
    }
    if at < len && matches!(b[at], b'e' | b'E') {
        let exponent = at;
        at += 1;
        if at < len && matches!(b[at], b'+' | b'-') {
            at += 1;
        }
        let digits = at;
        while at < len && (b[at].is_ascii_digit() || b[at] == b'_') {
            at += 1;
        }
        if at == digits {
            at = exponent;
        }
    }
    Some(consume_f16_suffix(b, at))
}

fn consume_f16_suffix(bytes: &[u8], mut at: usize) -> usize {
    if matches!(bytes.get(at), Some(b'f' | b'h' | b'i' | b'u')) {
        at += 1;
    }
    at
}

fn highlight_json(source: &str) -> Vec<Highlight> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            let start = at;
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at += 1;
                    if at < bytes.len() {
                        at += source[at..].chars().next().map_or(1, char::len_utf8);
                    }
                } else if bytes[at] == b'"' {
                    at += 1;
                    break;
                } else {
                    at += source[at..].chars().next().map_or(1, char::len_utf8);
                }
            }
            push(&mut out, start, at, Category::String);
            continue;
        }
        if let Some(end) = consume_json_number(bytes, at) {
            push(&mut out, at, end, Category::Number);
            at = end;
            continue;
        }
        if bytes[at].is_ascii_alphabetic() {
            let end = identifier_end(source, at);
            if matches!(&source[at..end], "true" | "false" | "null") {
                push(&mut out, at, end, Category::Boolean);
            }
            at = end;
            continue;
        }
        if matches!(bytes[at], b'{' | b'}' | b'[' | b']' | b',' | b':') {
            push(&mut out, at, at + 1, Category::Punctuation);
            at += 1;
            continue;
        }
        at += source[at..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

fn consume_json_number(bytes: &[u8], start: usize) -> Option<usize> {
    let len = bytes.len();
    let mut at = start;
    if bytes.get(at) == Some(&b'-') {
        if !bytes.get(at + 1).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        at += 1;
    } else if !bytes.get(at).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    if bytes[at] == b'0' {
        at += 1;
    } else {
        while at < len && bytes[at].is_ascii_digit() {
            at += 1;
        }
    }
    if at < len && bytes[at] == b'.' {
        at += 1;
        while at < len && bytes[at].is_ascii_digit() {
            at += 1;
        }
    }
    if at < len && matches!(bytes[at], b'e' | b'E') {
        let exponent = at;
        at += 1;
        if at < len && matches!(bytes[at], b'+' | b'-') {
            at += 1;
        }
        let digits = at;
        while at < len && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at == digits {
            at = exponent;
        }
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid_spans(source: &str, highlights: &[Highlight]) {
        let mut previous = 0;
        for highlight in highlights {
            assert!(highlight.range.start < highlight.range.end);
            assert!(highlight.range.start >= previous);
            assert!(source.is_char_boundary(highlight.range.start));
            assert!(source.is_char_boundary(highlight.range.end));
            previous = highlight.range.end;
        }
    }

    #[test]
    fn nested_and_unterminated_comments_are_atomic() {
        let source = "/* outer /* inner */ fn */ let x = /* unfinished";
        let spans = highlight(source, Language::Wgsl);
        let comments = spans
            .iter()
            .filter(|s| s.category == Category::Comment)
            .map(|s| &source[s.range.clone()])
            .collect::<Vec<_>>();
        assert_eq!(comments, ["/* outer /* inner */ fn */", "/* unfinished"]);
        assert!(spans
            .iter()
            .any(|s| s.category == Category::Keyword && &source[s.range.clone()] == "let"));
        assert!(!spans
            .iter()
            .any(|s| s.category == Category::Keyword && &source[s.range.clone()] == "fn"));
    }

    #[test]
    fn unicode_identifiers_keep_byte_boundaries() {
        let source = "let π = vec3f(1.0); // café";
        let spans = highlight(source, Language::Wgsl);
        assert_valid_spans(source, &spans);
        assert!(spans.iter().any(|h| h.category == Category::Keyword));
        assert!(spans.iter().any(|h| h.category == Category::Type));
        assert!(!spans.iter().any(|h| h.category == Category::Function));
    }

    #[test]
    fn hex_float_and_f16_are_numbers() {
        let source = "0x1.8p+2 2.0h 0xFFu";
        let spans = highlight(source, Language::Wgsl);
        assert_eq!(spans[0].category, Category::Number);
        assert_eq!(&source[spans[0].range.clone()], "0x1.8p+2");
        assert_eq!(&source[spans[1].range.clone()], "2.0h");
        assert_valid_spans(source, &spans);
    }

    #[test]
    fn escaped_partial_json_string_and_literals() {
        let source = r##"{"key":"line\"still","ok":true,"n":-1.2e}"##;
        let spans = highlight(source, Language::Json);
        assert_valid_spans(source, &spans);
        assert!(spans.iter().any(|h| h.category == Category::String));
        assert!(spans.iter().any(|h| h.category == Category::Boolean));
        assert!(spans.iter().any(|h| h.category == Category::Number));
    }

    #[test]
    fn plain_is_unstyled_and_spans_do_not_overlap() {
        assert!(highlight("fn true 1", Language::Plain).is_empty());
        let source = "@compute /* x */ fn f() -> f32";
        assert_valid_spans(source, &highlight(source, Language::Wgsl));
    }
}
