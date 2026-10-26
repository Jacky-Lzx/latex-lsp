//! Bounded compiler output retaining complete lines at both ends.
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

pub(crate) const TRUNCATED: &str = "[texquill: middle of compiler output omitted]";

pub(crate) fn truncated(text: &str) -> bool {
    text.lines().any(|line| line == TRUNCATED)
}

fn joined(mut head: Vec<u8>, tail: &[u8], previous: u8) -> Vec<u8> {
    // Do not fabricate a diagnostic, filename, or UTF-8 scalar across the gap.
    head.truncate(
        head.iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |end| end + 1),
    );
    head.extend_from_slice(TRUNCATED.as_bytes());
    head.push(b'\n');
    let tail = if previous == b'\n' {
        tail
    } else if let Some(end) = tail.iter().position(|byte| *byte == b'\n') {
        &tail[end + 1..]
    } else {
        // Preserve the end of a single enormous line for the output viewer,
        // while preventing the fragment from looking like a TeX diagnostic.
        head.extend_from_slice(b"[partial output line] ");
        tail
    };
    head.extend_from_slice(tail);
    head
}

pub(crate) fn capture(mut reader: impl Read, limit: usize) -> Vec<u8> {
    let head_limit = limit / 2;
    let tail_limit = limit - head_limit;
    let mut head = Vec::new();
    let mut tail = VecDeque::new();
    let mut previous = None;
    let mut buffer = [0; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let keep = count.min(head_limit - head.len());
        head.extend_from_slice(&buffer[..keep]);
        let bytes = &buffer[keep..count];
        if bytes.len() >= tail_limit {
            let discarded = bytes.len() - tail_limit;
            if discarded > 0 {
                previous = Some(bytes[discarded - 1]);
            } else if let Some(&byte) = tail.back() {
                previous = Some(byte);
            }
            tail.clear();
            tail.extend(&bytes[discarded..]);
        } else {
            let discarded = (tail.len() + bytes.len()).saturating_sub(tail_limit);
            if discarded > 0 {
                previous = Some(tail[discarded - 1]);
                tail.drain(..discarded);
            }
            tail.extend(bytes);
        }
    }
    if let Some(previous) = previous {
        joined(head, tail.make_contiguous(), previous)
    } else {
        head.extend(tail);
        head
    }
}

pub(crate) fn read(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() <= limit as u64 {
        let mut output = Vec::new();
        file.take(limit as u64).read_to_end(&mut output)?;
        return Ok(output);
    }
    let head_limit = limit / 2;
    let tail_limit = limit - head_limit;
    let mut head = Vec::new();
    file.by_ref()
        .take(head_limit as u64)
        .read_to_end(&mut head)?;
    // Seek over omitted bytes rather than scanning a potentially huge log.
    file.seek(SeekFrom::End(-(tail_limit as i64) - 1))?;
    let mut tail = Vec::new();
    file.take(tail_limit as u64 + 1).read_to_end(&mut tail)?;
    Ok(joined(
        head,
        tail.get(1..).unwrap_or_default(),
        tail.first().copied().unwrap_or(b'\n'),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_output_keeps_exact_small_outputs_and_complete_lines_on_both_ends() {
        for limit in [16, 31, 128, 8192] {
            for size in [0, 1, limit / 2, limit - 1, limit, limit + 1, limit * 3] {
                let text = "第一行。\nSecond line.\n"
                    .as_bytes()
                    .iter()
                    .copied()
                    .cycle()
                    .take(size)
                    .collect::<Vec<_>>();
                let actual = capture(text.as_slice(), limit);
                if size <= limit {
                    assert_eq!(actual, text, "limit={limit}, size={size}");
                } else {
                    assert!(actual
                        .windows(TRUNCATED.len())
                        .any(|window| window == TRUNCATED.as_bytes()));
                    assert!(actual.len() <= limit + TRUNCATED.len() + 24);
                }
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("output");
                std::fs::write(&path, &text).unwrap();
                assert_eq!(
                    read(&path, limit).unwrap(),
                    actual,
                    "limit={limit}, size={size}"
                );
            }
        }
    }

    #[test]
    fn interrupted_and_fragmented_reads_keep_the_same_tail_and_do_not_stop_draining() {
        struct Reader<'a> {
            bytes: &'a [u8],
            interrupted: bool,
        }
        impl Read for Reader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.interrupted = !self.interrupted;
                if self.interrupted {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.bytes.read(&mut buffer[..1])
            }
        }
        let text = b"First.\nnoise noise noise noise noise noise\nLast.\n";
        let mut reader = Reader {
            bytes: text,
            interrupted: false,
        };
        let actual = capture(&mut reader, 24);
        assert!(reader.bytes.is_empty());
        assert_eq!(actual, capture(text.as_slice(), 24));
        assert!(actual.starts_with(b"First.\n"));
        assert!(actual.ends_with(b"Last.\n"));
    }

    #[test]
    fn enormous_single_lines_preserve_their_tail_as_nonstructural_text() {
        let text = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaa! Undefined control sequence.";
        let output = capture(text.as_slice(), 32);
        assert!(output
            .windows(21)
            .any(|part| part == b"[partial output line]"));
        assert!(output.ends_with(&text[text.len() - 16..]));
    }
}
