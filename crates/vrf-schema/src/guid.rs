//! NetGUID value types: the identifier itself, its export flags, and the
//! per-GUID record the cache hands back.
//!
//! Split out from the cache because these are pure wire-level vocabulary --
//! they carry no state and no lookup logic -- and both the cache and the
//! stream readers need them.

/// A 32-bit network GUID referencing a replicated object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkGuid(pub u32);

impl NetworkGuid {
    /// The zero GUID is invalid (never assigned by the engine).
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }

    /// GUID 1 is the "default" object.
    #[must_use]
    pub const fn is_default(self) -> bool {
        self.0 == 1
    }

    /// Dynamic objects have an even GUID (bit 0 clear).
    #[must_use]
    pub const fn is_dynamic(self) -> bool {
        self.is_valid() && (self.0 & 1) == 0
    }
}

/// Flags on an exported NetGUID payload, controlling which optional fields
/// follow the GUID value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFlags(pub u8);

impl ExportFlags {
    pub const NONE: Self = Self(0);
    pub const HAS_PATH: Self = Self(1 << 0);
    #[allow(dead_code)]
    pub const NO_LOAD: Self = Self(1 << 1);
    pub const HAS_NETWORK_CHECKSUM: Self = Self(1 << 2);

    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }
}

/// One registered NetGUID and what the replay said about it.
///
/// Produced by [`NetGuidCache::net_guid_entries`](crate::NetGuidCache::net_guid_entries)
/// so exporters can persist the containment hierarchy. Downstream consumers
/// need it to walk from a subobject (e.g. a weapon's `FiringState`) to the
/// actor that owns it; that chain is the only route from a shot event to the
/// equippable that fired it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetGuidEntry<'a> {
    /// The GUID itself.
    pub net_guid: u32,
    /// Object path as the replay declared it.
    pub path: &'a str,
    /// Containing object's GUID, when the replay declared one.
    pub outer_net_guid: Option<u32>,
}
