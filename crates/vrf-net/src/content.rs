//! Content block header parsing and framing loop.
//!
//! A bunch's payload is a sequence of content blocks. Each block has a header
//! that identifies what it describes (the actor itself, a subobject, or a
//! deletion), followed by a payload whose structure depends on the header:
//!
//! - **RepLayout** (has_rep_layout = true): property field stream
//! - **ClassNetCache** (has_rep_layout = false): RPC function stream
//!
//! # Content block header bit layout
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ hasRepLayout        : 1 bit                                      │
//! │ isActor             : 1 bit                                      │
//! │ [if isActor → return immediately, actor uses channel state]      │
//! │ objectNetGuid       : InternalLoadObject(...)                     │
//! │ isStablyNamed       : 1 bit                                      │
//! │ [if isStablyNamed → return: stably-named subobject]              │
//! │ isDeleted           : 1 bit                                      │
//! │ [if isDeleted]                                                   │
//! │   deleteFlags       : 1 byte (8 bits)                            │
//! │   → return as deleted                                            │
//! │ classNetGuid        : InternalLoadObject(...)                     │
//! │ [if classNetGuid invalid → return as deleted (flags=0)]          │
//! │ bUseActorOuter      : 1 bit                                      │
//! │ [if !bUseActorOuter]                                             │
//! │   outerNetGuid      : InternalLoadObject(...)                    │
//! │ → return as subobject                                            │
//! └──────────────────────────────────────────────────────────────────┘
//! ```

use vrf_bitio::BitReader;

use crate::error::Result;
use crate::net_guid::{self, GuidPathSink};
use crate::types::NetworkGuid;

/// Parsed content block header.
#[derive(Debug, Clone)]
pub struct ContentBlockHeader {
    /// Whether the payload uses RepLayout (properties) vs ClassNetCache (RPCs).
    pub has_rep_layout: bool,
    /// This block describes the actor itself (not a subobject).
    pub is_actor: bool,
    /// This block is a deletion notification.
    pub is_deleted: bool,
    /// Net GUID of the object (subobject case).
    pub object_net_guid: NetworkGuid,
    /// Net GUID of the class (subobject case, when present).
    pub class_net_guid: NetworkGuid,
    /// Net GUID of the outer object.
    pub outer_net_guid: NetworkGuid,
    /// Whether the subobject is stably named.
    pub is_stably_named: bool,
    /// Deletion flags (only valid when is_deleted).
    pub delete_flags: u8,
}

impl Default for ContentBlockHeader {
    fn default() -> Self {
        Self {
            has_rep_layout: false,
            is_actor: false,
            is_deleted: false,
            object_net_guid: NetworkGuid(0),
            class_net_guid: NetworkGuid(0),
            outer_net_guid: NetworkGuid(0),
            is_stably_named: false,
            delete_flags: 0,
        }
    }
}

/// Read a content block header from the stream.
///
/// `actor_net_guid` is the channel's actor GUID (used as default outer).
/// `is_exporting` should be false for normal content block headers.
pub fn read_content_block_header(
    reader: &mut BitReader<'_>,
    actor_net_guid: NetworkGuid,
    sink: &mut dyn GuidPathSink,
) -> Result<ContentBlockHeader> {
    let has_rep_layout = reader.read_bit()?;

    // Is this block about the actor itself?
    if reader.read_bit()? {
        return Ok(ContentBlockHeader {
            has_rep_layout,
            is_actor: true,
            outer_net_guid: actor_net_guid,
            ..Default::default()
        });
    }

    // Subobject: read object net GUID
    let object_net_guid = net_guid::internal_load_object(reader, false, 0, sink)?;
    let is_stably_named = reader.read_bit()?;

    if is_stably_named {
        return Ok(ContentBlockHeader {
            has_rep_layout,
            is_actor: false,
            object_net_guid,
            outer_net_guid: actor_net_guid,
            is_stably_named: true,
            ..Default::default()
        });
    }

    // Check if deleted
    if reader.read_bit()? {
        let delete_flags = reader.read_u8()?;
        return Ok(ContentBlockHeader {
            has_rep_layout,
            is_actor: false,
            is_deleted: true,
            object_net_guid,
            outer_net_guid: actor_net_guid,
            delete_flags,
            ..Default::default()
        });
    }

    // Read class net GUID
    let class_net_guid = net_guid::internal_load_object(reader, false, 0, sink)?;
    if !class_net_guid.is_valid() {
        // Invalid class GUID means "deleted" with flags = 0
        return Ok(ContentBlockHeader {
            has_rep_layout,
            is_actor: false,
            is_deleted: true,
            object_net_guid,
            outer_net_guid: actor_net_guid,
            delete_flags: 0,
            ..Default::default()
        });
    }

    // Outer
    let outer_net_guid = if reader.read_bit()? {
        // bUseActorOuter = true
        actor_net_guid
    } else {
        net_guid::internal_load_object(reader, false, 0, sink)?
    };

    Ok(ContentBlockHeader {
        has_rep_layout,
        is_actor: false,
        object_net_guid,
        class_net_guid,
        outer_net_guid,
        is_stably_named: false,
        is_deleted: false,
        delete_flags: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct NullSink;
    impl GuidPathSink for NullSink {
        fn register_path(&mut self, _: u32, _: &str, _: NetworkGuid) {}
    }

    fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            for i in 0..8 {
                bits.push((next_byte & (1 << i)) != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let byte_count = bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }

    #[test]
    fn actor_block_returns_immediately() {
        let bits = vec![true, true]; // hasRepLayout=true, isActor=true
        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = NullSink;
        let hdr = read_content_block_header(&mut reader, NetworkGuid(18), &mut sink).unwrap();
        assert!(hdr.is_actor);
        assert!(hdr.has_rep_layout);
        assert_eq!(hdr.outer_net_guid, NetworkGuid(18));
    }

    #[test]
    fn subobject_stably_named() {
        let mut bits = Vec::new();
        bits.push(false); // hasRepLayout
        bits.push(false); // isActor = false
        write_int_packed(&mut bits, 50); // objectNetGuid
        bits.push(true); // isStablyNamed
        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = NullSink;
        let hdr = read_content_block_header(&mut reader, NetworkGuid(18), &mut sink).unwrap();
        assert!(!hdr.is_actor);
        assert!(hdr.is_stably_named);
        assert_eq!(hdr.object_net_guid, NetworkGuid(50));
    }

    #[test]
    fn deleted_block_reads_flags() {
        let mut bits = Vec::new();
        bits.push(false); // hasRepLayout
        bits.push(false); // isActor
        write_int_packed(&mut bits, 60); // objectNetGuid
        bits.push(false); // isStablyNamed
        bits.push(true); // isDeleted
        // deleteFlags byte: 0x03
        for i in 0..8 {
            bits.push((0x03u8 & (1 << i)) != 0);
        }
        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = NullSink;
        let hdr = read_content_block_header(&mut reader, NetworkGuid(18), &mut sink).unwrap();
        assert!(hdr.is_deleted);
        assert_eq!(hdr.delete_flags, 0x03);
        assert_eq!(hdr.object_net_guid, NetworkGuid(60));
    }
}
