/// Byte offsets of every line start in a source file, so per-diagnostic
/// line/column lookups are a binary search plus a scan of one line instead of a
/// scan of the whole file from byte 0. All methods must be called with the same
/// source text the index was built from.
#[derive(Debug)]
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source_text: &str) -> Self {
        let mut line_starts = vec![0];
        for (byte_index, byte) in source_text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(byte_index + 1);
            }
        }
        Self { line_starts }
    }

    /// 1-based line and column for a byte offset. Columns count characters, and
    /// an offset past the end of the source clamps to the final position,
    /// matching the previous linear-scan implementations.
    pub fn line_col(&self, source_text: &str, offset: usize) -> (usize, usize) {
        let target = offset.min(source_text.len());
        let line_number = self.line_starts.partition_point(|start| *start <= target);
        let line_start = self.line_starts[line_number - 1];
        // tsc counts a column in UTF-16 code units, two for an astral character.
        let column = source_text[line_start..]
            .char_indices()
            .take_while(|(byte_index, _)| line_start + byte_index < target)
            .map(|(_, character)| character.len_utf16())
            .sum::<usize>()
            + 1;
        (line_number, column)
    }

    /// Byte range `[start, end)` of the line containing `offset`, excluding the
    /// trailing newline.
    pub fn line_bounds(&self, source_text: &str, offset: usize) -> (usize, usize) {
        let target = offset.min(source_text.len());
        let line_number = self.line_starts.partition_point(|start| *start <= target);
        let line_start = self.line_starts[line_number - 1];
        let line_end = match self.line_starts.get(line_number) {
            Some(next_start) => next_start - 1,
            None => source_text.len(),
        };
        (line_start, line_end)
    }

    pub fn line_text<'a>(&self, source_text: &'a str, offset: usize) -> &'a str {
        let (line_start, line_end) = self.line_bounds(source_text, offset);
        &source_text[line_start..line_end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_line_col(source_text: &str, offset: usize) -> (usize, usize) {
        let mut line = 1usize;
        let mut column = 1usize;
        let target = offset.min(source_text.len());
        for (byte_index, ch) in source_text.char_indices() {
            if byte_index >= target {
                break;
            }
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        (line, column)
    }

    fn naive_line_bounds(source_text: &str, offset: usize) -> (usize, usize) {
        let target = offset.min(source_text.len());
        let mut line_start = 0usize;
        let mut line_end = source_text.len();
        for (byte_index, ch) in source_text.char_indices() {
            if byte_index >= target {
                break;
            }
            if ch == '\n' {
                line_start = byte_index + ch.len_utf8();
            }
        }
        for (byte_index, ch) in source_text[line_start..].char_indices() {
            if ch == '\n' {
                line_end = line_start + byte_index;
                break;
            }
        }
        (line_start, line_end)
    }

    #[test]
    fn matches_naive_scan_on_every_offset() {
        let samples = [
            "",
            "a",
            "\n",
            "abc",
            "abc\n",
            "\n\n\n",
            "const a = 1;\nexport {};\na = 3;\n",
            "no trailing newline\nsecond line",
            "unicode: 한글 텍스트\nsecond 줄\nthird",
            "\r\nwindows\r\nlines\r\n",
        ];
        for source in samples {
            let index = LineIndex::new(source);
            for offset in 0..=source.len() + 2 {
                assert_eq!(
                    index.line_col(source, offset),
                    naive_line_col(source, offset),
                    "line_col mismatch at offset {offset} in {source:?}"
                );
                assert_eq!(
                    index.line_bounds(source, offset),
                    naive_line_bounds(source, offset),
                    "line_bounds mismatch at offset {offset} in {source:?}"
                );
            }
        }
    }
}
