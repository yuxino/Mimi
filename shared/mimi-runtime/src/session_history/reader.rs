//! Read legacy JSON headers and recoverable JSONL with a fixed input buffer.
use super::{HistoryMetadata, HistoryPairMetadata};
use crate::core::{models::SubtitlePair, session_archive::TRANSCRIPT_COUNT_LIMIT};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::io::{self, BufRead, Read};

type Receive<'a> = Option<&'a mut dyn FnMut(usize, SubtitlePair) -> io::Result<()>>;

/// A later pass must recover the same entry count and header as the first.
/// A newly interrupted read must not publish a shorter transcript as complete.
pub(super) fn scan_verified(
    reader: &mut impl BufRead,
    expected: &HistoryMetadata,
    receive: Receive<'_>,
) -> io::Result<()> {
    if scan(reader, &expected.id, receive)? != *expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "history changed during read",
        ));
    }
    Ok(())
}

pub(super) fn scan(
    reader: &mut impl BufRead,
    id: &str,
    receive: Receive<'_>,
) -> io::Result<HistoryMetadata> {
    let mut sink = Entries {
        receive,
        count: 0,
        failure: None,
    };
    let mut line = Line::next(reader)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty history"))?;
    let header = line.parse(Header(&mut sink));
    line.finish()?;
    if let Some(error) = sink.failure.take() {
        return Err(error);
    }
    let mut metadata = header?;
    if metadata.version != 1 || metadata.id != id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid history file",
        ));
    }
    loop {
        // Existing header arrays are not truncated; only journal additions use
        // the cap. No later journal entry can change an already full archive.
        if sink.count >= TRANSCRIPT_COUNT_LIMIT {
            break;
        }
        let Ok(Some(mut line)) = Line::next(reader) else {
            break;
        };
        let pair = line.parse(PairSeed(sink.receive.is_some()));
        // read_line historically stopped recovery on invalid UTF-8 or an I/O
        // error, even if the JSON parser had already rejected this line.
        if line.finish().is_err() {
            break;
        }
        let Ok(pair) = pair else {
            continue;
        };
        metadata.ended_at_ms = metadata.ended_at_ms.max(pair.created_at_ms());
        sink.push(pair)?;
    }
    metadata.entries = sink.count;
    Ok(metadata)
}

enum Pair {
    Full(SubtitlePair),
    Metadata(HistoryPairMetadata),
}

struct PairSeed(bool);
impl<'de> DeserializeSeed<'de> for PairSeed {
    type Value = Pair;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Pair, D::Error> {
        if self.0 {
            SubtitlePair::deserialize(decoder).map(Pair::Full)
        } else {
            HistoryPairMetadata::deserialize(decoder).map(Pair::Metadata)
        }
    }
}
impl Pair {
    fn created_at_ms(&self) -> u64 {
        match self {
            Self::Full(pair) => pair.created_at_ms,
            Self::Metadata(pair) => pair.created_at_ms,
        }
    }
}

struct Entries<'a> {
    receive: Receive<'a>,
    count: usize,
    failure: Option<io::Error>,
}
impl Entries<'_> {
    fn push(&mut self, pair: Pair) -> io::Result<()> {
        if let (Some(receive), Pair::Full(pair)) = (self.receive.as_mut(), pair) {
            receive(self.count, pair)?;
        }
        self.count += 1;
        Ok(())
    }
}
struct EntryArray<'a, 'b>(&'a mut Entries<'b>);
impl<'de> DeserializeSeed<'de> for EntryArray<'_, '_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for EntryArray<'_, '_> {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an array of subtitle pairs")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<(), A::Error> {
        loop {
            let pair = values.next_element_seed(PairSeed(self.0.receive.is_some()))?;
            let Some(pair) = pair else { break };
            if let Err(error) = self.0.push(pair) {
                self.0.failure = Some(error);
                return Err(de::Error::custom("transcript visitor failed"));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "camelCase")]
enum Field {
    Version,
    Id,
    StartedAtMs,
    EndedAtMs,
    Limited,
    Entries,
    #[serde(other)]
    Other,
}
struct Header<'a, 'b>(&'a mut Entries<'b>);
impl<'de> DeserializeSeed<'de> for Header<'_, '_> {
    type Value = HistoryMetadata;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        decoder.deserialize_struct(
            "SavedTranscript",
            &[
                "version",
                "id",
                "startedAtMs",
                "endedAtMs",
                "limited",
                "entries",
            ],
            self,
        )
    }
}
impl<'de> Visitor<'de> for Header<'_, '_> {
    type Value = HistoryMetadata;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a saved transcript header")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
        // Preserve serde's accepted struct sequence form for older/private files.
        let version = values
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let id = values
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        let started_at_ms = values
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
        let ended_at_ms = values
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
        let limited = values
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(4, &self))?;
        values
            .next_element_seed(EntryArray(self.0))?
            .ok_or_else(|| de::Error::invalid_length(5, &self))?;
        Ok(HistoryMetadata {
            version,
            id,
            started_at_ms,
            ended_at_ms,
            header_ended_at_ms: ended_at_ms,
            limited,
            entries: self.0.count,
        })
    }
    fn visit_map<A: MapAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
        let (mut version, mut id, mut started, mut ended, mut limited) =
            (None, None, None, None, None);
        let mut entries_seen = false;
        while let Some(field) = values.next_key()? {
            match field {
                Field::Version => {
                    if version.is_some() {
                        return Err(de::Error::duplicate_field("version"));
                    }
                    version = Some(values.next_value()?);
                }
                Field::Id => {
                    if id.is_some() {
                        return Err(de::Error::duplicate_field("id"));
                    }
                    id = Some(values.next_value()?);
                }
                Field::StartedAtMs => {
                    if started.is_some() {
                        return Err(de::Error::duplicate_field("startedAtMs"));
                    }
                    started = Some(values.next_value()?);
                }
                Field::EndedAtMs => {
                    if ended.is_some() {
                        return Err(de::Error::duplicate_field("endedAtMs"));
                    }
                    ended = Some(values.next_value()?);
                }
                Field::Limited => {
                    if limited.is_some() {
                        return Err(de::Error::duplicate_field("limited"));
                    }
                    limited = Some(values.next_value()?);
                }
                Field::Entries => {
                    if entries_seen {
                        return Err(de::Error::duplicate_field("entries"));
                    }
                    entries_seen = true;
                    values.next_value_seed(EntryArray(self.0))?;
                }
                Field::Other => {
                    values.next_value::<de::IgnoredAny>()?;
                }
            }
        }
        if !entries_seen {
            return Err(de::Error::missing_field("entries"));
        }
        let header_ended_at_ms = ended.unwrap_or_default();
        Ok(HistoryMetadata {
            version: version.ok_or_else(|| de::Error::missing_field("version"))?,
            id: id.ok_or_else(|| de::Error::missing_field("id"))?,
            started_at_ms: started.ok_or_else(|| de::Error::missing_field("startedAtMs"))?,
            ended_at_ms: ended.ok_or_else(|| de::Error::missing_field("endedAtMs"))?,
            header_ended_at_ms,
            limited: limited.ok_or_else(|| de::Error::missing_field("limited"))?,
            entries: self.0.count,
        })
    }
}

/// serde's reader can parse one byte at a time. Bound it to this physical line
/// and validate even unknown fields or the discarded tail after malformed JSON.
struct Line<'a, R: BufRead> {
    reader: &'a mut R,
    utf8: Utf8,
    bytes: [u8; 4096],
    position: usize,
    length: usize,
    ended: bool,
}
impl<'a, R: BufRead> Line<'a, R> {
    fn next(reader: &'a mut R) -> io::Result<Option<Self>> {
        if reader.fill_buf()?.is_empty() {
            return Ok(None);
        }
        let mut line = Self {
            reader,
            utf8: Utf8::default(),
            bytes: [0; 4096],
            position: 0,
            length: 0,
            ended: false,
        };
        line.fill()?;
        Ok(Some(line))
    }
    fn parse<T, S>(&mut self, seed: S) -> serde_json::Result<T>
    where
        S: for<'de> DeserializeSeed<'de, Value = T>,
    {
        // Most journal records fit in one fixed block. Borrow their text while
        // validating metadata; large header arrays stay on the streaming path.
        if self.ended {
            decode(
                seed,
                serde_json::Deserializer::from_slice(&self.bytes[..self.length]),
            )
        } else {
            decode(seed, serde_json::Deserializer::from_reader(self))
        }
    }
    fn fill(&mut self) -> io::Result<()> {
        let bytes = self.reader.fill_buf()?;
        let available = bytes.len().min(self.bytes.len());
        let end = bytes[..available].iter().position(|&byte| byte == b'\n');
        self.length = end.unwrap_or(available);
        self.position = 0;
        self.bytes[..self.length].copy_from_slice(&bytes[..self.length]);
        self.utf8.push(&self.bytes[..self.length]);
        self.reader
            .consume(self.length + usize::from(end.is_some()));
        // A short BufRead chunk is not necessarily EOF. Peek after consuming
        // it so complete final JSON without a newline gets the same fast path.
        self.ended = end.is_some() || self.reader.fill_buf()?.is_empty();
        if self.utf8.invalid || self.ended {
            self.utf8.finish()?;
        }
        Ok(())
    }
    fn finish(mut self) -> io::Result<()> {
        while !self.ended {
            self.fill()?;
        }
        self.utf8.finish()
    }
}
impl<R: BufRead> Read for Line<'_, R> {
    #[inline]
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.position == self.length && !self.ended {
            self.fill()?;
        }
        let count = output.len().min(self.length - self.position);
        output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

fn decode<'de, T, S, R>(seed: S, mut decoder: serde_json::Deserializer<R>) -> serde_json::Result<T>
where
    S: DeserializeSeed<'de, Value = T>,
    R: serde_json::de::Read<'de>,
{
    let value = seed.deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}

#[derive(Default)]
struct Utf8 {
    pending: [u8; 4],
    count: usize,
    invalid: bool,
}
impl Utf8 {
    fn push(&mut self, mut bytes: &[u8]) {
        if self.invalid {
            return;
        }
        while self.count > 0 && !bytes.is_empty() {
            self.pending[self.count] = bytes[0];
            self.count += 1;
            bytes = &bytes[1..];
            match std::str::from_utf8(&self.pending[..self.count]) {
                Ok(_) => self.count = 0,
                Err(error) if error.error_len().is_some() => {
                    self.invalid = true;
                    return;
                }
                Err(_) => {}
            }
        }
        if self.count == 0 {
            if let Err(error) = std::str::from_utf8(bytes) {
                if error.error_len().is_some() {
                    self.invalid = true;
                } else {
                    let tail = &bytes[error.valid_up_to()..];
                    self.pending[..tail.len()].copy_from_slice(tail);
                    self.count = tail.len();
                }
            }
        }
    }
    fn finish(&self) -> io::Result<()> {
        if self.invalid || self.count != 0 {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid history encoding",
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session_archive::SavedTranscript;

    fn header() -> Vec<u8> {
        serde_json::to_vec(&SavedTranscript {
            version: 1,
            id: "synthetic".into(),
            started_at_ms: 100,
            ended_at_ms: 200,
            limited: false,
            entries: vec![SubtitlePair::new("日本語 😀".into(), "译文 ä".into(), 150)],
        })
        .unwrap()
    }

    #[test]
    fn header_validation_matches_saved_transcript_including_sequences_and_duplicates() {
        let baseline = header();
        let value: serde_json::Value = serde_json::from_slice(&baseline).unwrap();
        let fields = [
            "version",
            "id",
            "startedAtMs",
            "endedAtMs",
            "limited",
            "entries",
        ];
        let mut variants = vec![
            baseline.clone(),
            serde_json::to_vec(&fields.map(|field| value[field].clone())).unwrap(),
        ];
        for field in fields {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            variants.push(serde_json::to_vec(&missing).unwrap());
            let duplicate = format!("{{\"{field}\":{},", value[field]);
            variants.push([duplicate.as_bytes(), &baseline[1..]].concat());
        }
        variants.push([baseline.as_slice(), b" trailing"].concat());
        variants.push(
            serde_json::to_vec(&serde_json::json!([1, "synthetic", 100, 200, false])).unwrap(),
        );
        for bytes in variants {
            let expected = serde_json::from_slice::<SavedTranscript>(&bytes);
            for full in [false, true] {
                let mut received = Vec::new();
                let mut receive = |_: usize, pair| {
                    received.push(pair);
                    Ok(())
                };
                let result = scan(
                    &mut io::Cursor::new(&bytes),
                    "synthetic",
                    full.then_some(&mut receive),
                );
                assert_eq!(
                    result.is_ok(),
                    expected.is_ok(),
                    "header={}",
                    String::from_utf8_lossy(&bytes)
                );
                if let Ok(metadata) = result {
                    let expected = expected.as_ref().unwrap();
                    assert_eq!(metadata.entries, expected.entries.len());
                    if full {
                        assert_eq!(
                            serde_json::to_value(received).unwrap(),
                            serde_json::to_value(&expected.entries).unwrap()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn utf8_validation_spans_buffers_and_stops_after_invalid_journal_tails() {
        let pair = serde_json::to_vec(&SubtitlePair::new("日本語 😀".into(), "译文 ä".into(), 250))
            .unwrap();
        for capacity in [1, 2, 3, 7, 8192] {
            for invalid in [
                b"{broken \xff".as_slice(),
                b"{\"unknown\":\"\xff\"}",
                b"\xe6\x97",
            ] {
                let mut bytes = header();
                bytes.extend_from_slice(b"\n{broken}\n");
                bytes.extend_from_slice(&pair);
                bytes.push(b'\n');
                bytes.extend_from_slice(invalid);
                bytes.push(b'\n');
                bytes.extend_from_slice(&pair);
                let mut input = io::BufReader::with_capacity(capacity, io::Cursor::new(bytes));
                let metadata = scan(&mut input, "synthetic", None).unwrap();
                assert_eq!((metadata.entries, metadata.ended_at_ms), (2, 250));
            }
            let mut bytes = header();
            bytes.pop();
            bytes.extend_from_slice(b",\"unknown\":\"\xff\"}");
            let mut input = io::BufReader::with_capacity(capacity, io::Cursor::new(bytes));
            assert!(scan(&mut input, "synthetic", None).is_err());
        }
    }

    #[test]
    fn final_headers_without_newlines_preserve_chunk_and_unicode_boundaries() {
        let mut prefix = header();
        prefix.pop();
        prefix.extend_from_slice(b",\"unknown\":\"");
        for length in [512, 4096, 4097, 8192] {
            let mut bytes = prefix.clone();
            bytes.resize(length - 2, b'a');
            bytes.extend_from_slice(b"\"}");
            for capacity in [1, 4095, 4096, 8192] {
                let mut input = io::BufReader::with_capacity(capacity, io::Cursor::new(&bytes));
                assert_eq!(scan(&mut input, "synthetic", None).unwrap().entries, 1);
            }
        }
        prefix.resize(4095, b'a');
        prefix.extend_from_slice("😀\"}".as_bytes());
        for capacity in [1, 4095, 4096, 8192] {
            let mut input = io::BufReader::with_capacity(capacity, io::Cursor::new(&prefix));
            assert_eq!(scan(&mut input, "synthetic", None).unwrap().entries, 1);
        }
    }

    #[test]
    fn rescan_rejects_changed_headers_and_preserves_stable_recovery_boundaries() {
        use std::io::Seek;
        let original = header();
        let pair =
            serde_json::to_vec(&SubtitlePair::new("Later".into(), "译文".into(), 500)).unwrap();
        let bytes = [
            original.as_slice(),
            b"\n",
            &pair,
            b"\n{broken \xff\n",
            &pair,
        ]
        .concat();
        let mut input = io::Cursor::new(&bytes);
        let expected = scan(&mut input, "synthetic", None).unwrap();
        assert_eq!((expected.entries, expected.ended_at_ms), (2, 500));
        input.rewind().unwrap();
        scan_verified(&mut input, &expected, None).unwrap();

        for (field, value) in [
            ("startedAtMs", serde_json::json!(101)),
            // The journal's later timestamp must not hide this header change.
            ("endedAtMs", serde_json::json!(201)),
            ("limited", serde_json::json!(true)),
        ] {
            let mut header: serde_json::Value = serde_json::from_slice(&original).unwrap();
            header[field] = value;
            let changed = serde_json::to_vec(&header).unwrap();
            let bytes = [changed.as_slice(), b"\n", &pair].concat();
            assert!(scan_verified(&mut io::Cursor::new(bytes), &expected, None).is_err());
        }
    }
}
