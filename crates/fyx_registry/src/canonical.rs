//! The canonical byte form that signatures cover.
//!
//! JSON is not signed directly: the same document has many JSON spellings (whitespace, key
//! order, escapes), and a re-serialized index must still verify. Instead every signed document
//! is encoded field by field, in a fixed order, as:
//!
//! - a string or byte string: its length as a big-endian `u64`, then the bytes;
//! - a `u64`: 8 bytes, big-endian;
//! - a `bool`: one byte, `0` or `1`;
//! - a list: its length as a big-endian `u64`, then each item.
//!
//! The first field is always a domain tag such as `fyx-registry/release/v1`, so a signature over
//! one kind of document can never be replayed as a signature over another.

pub(crate) struct CanonicalWriter {
    bytes: Vec<u8>,
}

impl CanonicalWriter {
    pub(crate) fn new(domain: &str) -> Self {
        let mut writer = Self { bytes: Vec::new() };
        writer.string(domain);
        writer
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }

    pub(crate) fn byte_string(&mut self, value: &[u8]) {
        self.length(value.len());
        self.bytes.extend_from_slice(value);
    }

    pub(crate) fn string(&mut self, value: &str) {
        self.byte_string(value.as_bytes());
    }

    pub(crate) fn strings<'a>(&mut self, values: impl ExactSizeIterator<Item = &'a str>) {
        self.length(values.len());
        for value in values {
            self.string(value);
        }
    }

    pub(crate) fn length(&mut self, length: usize) {
        // usize is at most 64 bits on every supported target.
        self.u64(length as u64);
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Decodes the canonical form back into fields. Only tests use it: decoding every encoded
/// document back to an equal value proves the encoding is injective, which is what makes
/// signing it safe.
#[cfg(test)]
pub(crate) struct CanonicalReader<'a> {
    bytes: &'a [u8],
}

#[cfg(test)]
impl<'a> CanonicalReader<'a> {
    pub(crate) fn new(bytes: &'a [u8], domain: &str) -> Option<Self> {
        let mut reader = Self { bytes };
        (reader.string()? == domain).then_some(reader)
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < count {
            return None;
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Some(head)
    }

    pub(crate) fn u64(&mut self) -> Option<u64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().ok()?;
        Some(u64::from_be_bytes(bytes))
    }

    pub(crate) fn bool(&mut self) -> Option<bool> {
        match self.take(1)? {
            [0] => Some(false),
            [1] => Some(true),
            _ => None,
        }
    }

    pub(crate) fn length(&mut self) -> Option<usize> {
        usize::try_from(self.u64()?).ok()
    }

    pub(crate) fn byte_string(&mut self) -> Option<&'a [u8]> {
        let length = self.length()?;
        self.take(length)
    }

    pub(crate) fn string(&mut self) -> Option<String> {
        String::from_utf8(self.byte_string()?.to_vec()).ok()
    }

    pub(crate) fn strings(&mut self) -> Option<Vec<String>> {
        let count = self.length()?;
        (0..count).map(|_| self.string()).collect()
    }

    pub(crate) fn finish(self) -> Option<()> {
        self.bytes.is_empty().then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn layout_is_length_prefixed_big_endian() {
        let mut writer = CanonicalWriter::new("d");
        writer.u64(0x0102);
        writer.bool(true);
        writer.strings(["ab"].into_iter());
        assert_eq!(
            writer.finish(),
            [
                0, 0, 0, 0, 0, 0, 0, 1, b'd', //
                0, 0, 0, 0, 0, 0, 1, 2, //
                1, //
                0, 0, 0, 0, 0, 0, 0, 1, //
                0, 0, 0, 0, 0, 0, 0, 2, b'a', b'b',
            ]
        );
    }

    #[test]
    fn reader_refuses_wrong_domain_and_trailing_bytes() {
        let mut writer = CanonicalWriter::new("one");
        writer.u64(7);
        let bytes = writer.finish();
        assert!(CanonicalReader::new(&bytes, "two").is_none());

        let mut with_trailing = bytes;
        with_trailing.push(0);
        let mut reader = CanonicalReader::new(&with_trailing, "one").expect("domain matches");
        assert_eq!(reader.u64(), Some(7));
        assert!(reader.finish().is_none());
    }

    proptest! {
        #[test]
        fn fields_round_trip(
            number in any::<u64>(),
            flag in any::<bool>(),
            text in ".*",
            list in proptest::collection::vec(".*", 0..5),
        ) {
            let mut writer = CanonicalWriter::new("test/v1");
            writer.u64(number);
            writer.bool(flag);
            writer.string(&text);
            writer.strings(list.iter().map(String::as_str));
            let bytes = writer.finish();

            let mut reader = CanonicalReader::new(&bytes, "test/v1").expect("domain matches");
            prop_assert_eq!(reader.u64(), Some(number));
            prop_assert_eq!(reader.bool(), Some(flag));
            prop_assert_eq!(reader.string(), Some(text));
            prop_assert_eq!(reader.strings(), Some(list));
            prop_assert_eq!(reader.finish(), Some(()));
        }

        // Moving bytes between adjacent strings must change the encoding: ("ab", "c") and
        // ("a", "bc") are different documents.
        #[test]
        fn string_boundaries_are_unambiguous(first in ".*", second in ".*", split in 0usize..8) {
            let joined = format!("{first}{second}");
            let split = joined
                .char_indices()
                .map(|(index, _)| index)
                .nth(split)
                .unwrap_or(joined.len());
            let (other_first, other_second) = joined.split_at(split);
            prop_assume!(other_first != first);

            let encode = |a: &str, b: &str| {
                let mut writer = CanonicalWriter::new("test/v1");
                writer.string(a);
                writer.string(b);
                writer.finish()
            };
            prop_assert_ne!(encode(&first, &second), encode(other_first, other_second));
        }
    }
}
