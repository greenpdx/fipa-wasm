//! Typed-block container — a single byte stream carrying several typed data
//! blocks, in the spirit of a classic Macintosh resource fork.
//!
//! One container, many blocks, each tagged by a 4-byte type. Used in two places:
//! - the **agent bundle** (many blocks): `WASM` (code), `HEAD` (agent header),
//!   `LLM ` (an embedded reasoning model *or* a reference to one), `UNL ` (the
//!   agent's vocabulary / decode *rules*), `DATA` (agent data), …;
//! - a **message** (two blocks): `UNL ` (semantic content) + `DATA` (the payload
//!   the UNL describes).
//!
//! This layer is content-agnostic: the node reads blocks **by tag**; the bytes
//! inside each block are interpreted by whichever layer owns that tag (the UNL
//! layer deserializes the `UNL ` block, the LLM layer the `LLM ` block, etc.).
//! That keeps the data (agent's blocks) separate from the process (the node's
//! engine).

/// A 4-byte block type tag (e.g. `*b"UNL "`, `*b"WASM"`, `*b"DATA"`).
pub type Tag = [u8; 4];

/// A WASM code block in an agent bundle.
pub const TAG_WASM: Tag = *b"WASM";
/// The agent header block (metadata).
pub const TAG_HEADER: Tag = *b"HEAD";
/// The reasoning-model block: an embedded model or a reference to one.
pub const TAG_LLM: Tag = *b"LLM ";
/// The UNL block: the agent's vocabulary rules, or a message's semantic content.
pub const TAG_UNL: Tag = *b"UNL ";
/// The data/payload block (agent data, or a message body the UNL describes).
pub const TAG_DATA: Tag = *b"DATA";
/// A UTF-8 string block.
pub const TAG_STRING: Tag = *b"STR ";

// The tag set is OPEN: any 4-byte tag is valid. The constants above name the
// common types; add more (here or at the call site, e.g. `*b"XML "`) as needed —
// the container never enumerates or rejects unknown tags.

const MAGIC: &[u8; 4] = b"FBLK";
const VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub tag: Tag,
    pub data: Vec<u8>,
}

/// A container of typed blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockFile {
    pub blocks: Vec<Block>,
}

#[derive(Debug, thiserror::Error)]
pub enum BlockError {
    #[error("not a block container (bad magic)")]
    BadMagic,
    #[error("unsupported block-container version: {0}")]
    Version(u8),
    #[error("truncated block container")]
    Truncated,
}

impl BlockFile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a block (builder form).
    pub fn with(mut self, tag: Tag, data: impl Into<Vec<u8>>) -> Self {
        self.blocks.push(Block { tag, data: data.into() });
        self
    }

    /// The bytes of the first block with this tag, if present.
    pub fn get(&self, tag: Tag) -> Option<&[u8]> {
        self.blocks.iter().find(|b| b.tag == tag).map(|b| b.data.as_slice())
    }

    pub fn has(&self, tag: Tag) -> bool {
        self.blocks.iter().any(|b| b.tag == tag)
    }

    /// Encode to the wire form: magic, version, u16 block count, then each block
    /// as `tag(4) | len(u32 BE) | data`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&(self.blocks.len() as u16).to_be_bytes());
        for b in &self.blocks {
            out.extend_from_slice(&b.tag);
            out.extend_from_slice(&(b.data.len() as u32).to_be_bytes());
            out.extend_from_slice(&b.data);
        }
        out
    }

    /// Decode from the wire form.
    pub fn decode(bytes: &[u8]) -> Result<Self, BlockError> {
        let mut p = 0usize;
        let take = |p: &mut usize, n: usize| -> Result<&[u8], BlockError> {
            let end = p.checked_add(n).ok_or(BlockError::Truncated)?;
            let slice = bytes.get(*p..end).ok_or(BlockError::Truncated)?;
            *p = end;
            Ok(slice)
        };

        if take(&mut p, 4)? != MAGIC {
            return Err(BlockError::BadMagic);
        }
        let version = take(&mut p, 1)?[0];
        if version != VERSION {
            return Err(BlockError::Version(version));
        }
        let count = u16::from_be_bytes(take(&mut p, 2)?.try_into().unwrap());

        let mut blocks = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let tag: Tag = take(&mut p, 4)?.try_into().unwrap();
            let len = u32::from_be_bytes(take(&mut p, 4)?.try_into().unwrap()) as usize;
            let data = take(&mut p, len)?.to_vec();
            blocks.push(Block { tag, data });
        }
        Ok(BlockFile { blocks })
    }

    /// Whether a byte stream looks like a block container (cheap prefix check).
    pub fn is_block_container(bytes: &[u8]) -> bool {
        bytes.starts_with(MAGIC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_bundle_many_blocks() {
        // An agent bundle: code, header, model reference, vocabulary, data.
        let agent = BlockFile::new()
            .with(TAG_WASM, vec![0, 1, 2, 3])
            .with(TAG_HEADER, b"name=gate-sensor".to_vec())
            .with(TAG_LLM, b"ollama:llama3.1".to_vec()) // a reference, not an embedded model
            .with(TAG_UNL, b"vocab-bytes".to_vec())
            .with(TAG_DATA, vec![0xAA]);
        let back = BlockFile::decode(&agent.encode()).unwrap();
        assert_eq!(back, agent);
        assert_eq!(back.get(TAG_UNL), Some(b"vocab-bytes".as_slice()));
        assert_eq!(back.get(TAG_LLM), Some(b"ollama:llama3.1".as_slice()));
        assert!(back.has(TAG_WASM) && back.has(TAG_HEADER) && back.has(TAG_DATA));
        assert_eq!(back.get(*b"NONE"), None);
    }

    #[test]
    fn message_shape_unl_plus_data() {
        // A message: the UNL semantic block + the data payload it describes.
        let msg = BlockFile::new()
            .with(TAG_UNL, b"agt(detect, gate)".to_vec())
            .with(TAG_DATA, vec![0x17]); // e.g. a sensor reading
        let back = BlockFile::decode(&msg.encode()).unwrap();
        assert_eq!(back.get(TAG_UNL), Some(b"agt(detect, gate)".as_slice()));
        assert_eq!(back.get(TAG_DATA), Some([0x17].as_slice()));
    }

    #[test]
    fn tag_set_is_open() {
        // Named and ad-hoc tags coexist; unknown tags are stored and read back.
        let f = BlockFile::new()
            .with(TAG_STRING, b"hello".to_vec())
            .with(*b"XML ", b"<x/>".to_vec()); // a tag with no named constant
        let back = BlockFile::decode(&f.encode()).unwrap();
        assert_eq!(back.get(TAG_STRING), Some(b"hello".as_slice()));
        assert_eq!(back.get(*b"XML "), Some(b"<x/>".as_slice()));
    }

    #[test]
    fn empty_container_roundtrips() {
        let bytes = BlockFile::new().encode();
        assert_eq!(BlockFile::decode(&bytes).unwrap(), BlockFile::new());
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        assert!(matches!(BlockFile::decode(b"XXXX...."), Err(BlockError::BadMagic)));
        assert!(matches!(BlockFile::decode(b"FB"), Err(BlockError::Truncated)));
        // Declares one block but no block data follows.
        let mut bytes = BlockFile::new().with(TAG_UNL, vec![1, 2, 3]).encode();
        bytes.truncate(bytes.len() - 2); // chop the data
        assert!(matches!(BlockFile::decode(&bytes), Err(BlockError::Truncated)));
    }
}
