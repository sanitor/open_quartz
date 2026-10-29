//! Small, syntax-agnostic metadata helpers for the native code editor.
//! Offsets intentionally remain UTF-8 byte offsets: Slint TextInput's selection
//! API uses byte offsets, so Unicode source stays aligned with cursor navigation.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeEditorMetadata {
    pub line_numbers: Vec<String>,
    pub line_offsets: Vec<i32>,
    pub find_offsets: Vec<i32>,
    pub find_lengths: Vec<i32>,
}

/// Build line starts and matching UTF-8 offsets without splitting Unicode codepoints.
/// `query == ""` deliberately produces no matches.
pub fn metadata(source: &str, query: &str) -> CodeEditorMetadata {
    let mut line_offsets = vec![0_i32];
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_offsets.push((offset + 1) as i32);
        }
    }

    let line_numbers = (1..=line_offsets.len())
        .map(|line| line.to_string())
        .collect();
    let (find_offsets, find_lengths) = if query.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        source
            .match_indices(query)
            .map(|(offset, matched)| (offset as i32, matched.len() as i32))
            .unzip()
    };
    CodeEditorMetadata {
        line_numbers,
        line_offsets,
        find_offsets,
        find_lengths,
    }
}

#[cfg(test)]
mod tests {
    use super::metadata;

    #[test]
    fn unicode_find_ranges_and_line_starts_are_native_byte_offsets() {
        let source = "// 中文🙂\r\nlet 色 = 色;\n";
        let data = metadata(source, "色");
        assert_eq!(data.line_numbers, ["1", "2", "3"]);
        assert_eq!(data.line_offsets, [0, 15, 30]);
        assert_eq!(data.find_offsets, [19, 25]);
        assert_eq!(data.find_lengths, [3, 3]);
        for (start, length) in data.find_offsets.iter().zip(&data.find_lengths) {
            assert_eq!(&source[*start as usize..(*start + *length) as usize], "色");
        }
        assert_eq!(metadata(source, "🙂").find_lengths, [4]);
    }

    #[test]
    fn empty_query_does_not_select_every_character() {
        let data = metadata("", "");
        assert_eq!(data.line_numbers, ["1"]);
        assert_eq!(data.line_offsets, [0]);
        assert!(data.find_offsets.is_empty());
        assert!(data.find_lengths.is_empty());
        assert!(metadata("中文", "missing").find_offsets.is_empty());
    }
}
