//! Net field export group and individual field descriptors.
//!
//! A [`NetFieldExportGroup`] is one path in the game's object hierarchy (e.g.
//! `/Game/Abilities/GrenadeExplodeIndicator.GrenadeExplodeIndicator_C:MulticastTriggerExplodeIndicator`)
//! and contains a sparse vector of [`NetFieldExport`] entries keyed by numeric
//! handle.
//!
//! These are **not** hard-coded -- the replay stream declares them at runtime,
//! and handles may shift between game builds.

/// Apply an `FName`'s instance number to its string, Unreal's way.
///
/// An `FName` is a (string, number) pair and the number is stored as **the
/// displayed suffix plus one**: 0 renders the bare name, and `N != 0` renders
/// `Name_{N-1}`. Both schema readers used to read the number into `let _number`
/// and drop it, so two handles whose base string matched arrived under one
/// name and the manifest's handle-to-name mapping could not tell them apart --
/// with nothing reporting the collision.
///
/// A negative number cannot come off a well-formed wire and has no display
/// form, so it is appended as it is rather than wrapped into a plausible
/// positive suffix.
///
/// Shared by [`crate::read_net_field_exports`]'s reader and the checkpoint
/// one, which are deliberately separate functions (their leading bytes mean
/// opposite things) but must render a name identically. `vrf-decode`'s
/// `scalar::render_fname` is the same function over the same rule, so a name
/// means the same thing whichever layer produced it; the two crates share no
/// dependency edge, which is why it is written twice rather than imported.
pub(crate) fn render_fname(name: String, number: i32) -> String {
    match number {
        0 => name,
        n => format!("{name}_{}", n.wrapping_sub(1)),
    }
}

/// A single replicated-field descriptor within a group.
///
/// The `handle` is the key used in content blocks to identify which field is
/// being replicated. The `name` is purely informational for the parser (used to
/// route the payload to the correct decoder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetFieldExport {
    /// Numeric handle assigned by the server. Content blocks reference this
    /// number; the parser looks it up here to learn the field's name.
    pub handle: u32,
    /// Checksum the engine uses to detect schema drift between client and server.
    pub compatible_checksum: u32,
    /// Human-readable field name (e.g. `"IndicatorLocation"`).
    pub name: String,
}

/// A group of field exports sharing a common object path.
///
/// The engine sends groups incrementally: the first time a path is seen it
/// carries the path string and declares the array length; subsequent appearances
/// may re-use the `path_name_index` and only add or overwrite individual fields.
///
/// Fields are stored in a `Vec` indexed directly by handle for O(1) access.
/// Slots that have not yet been populated are `None`.
#[derive(Debug, Clone)]
pub struct NetFieldExportGroup {
    /// The full path identifying this group in the Unreal object hierarchy.
    pub path: String,
    /// Numeric index used on subsequent frames to reference this group without
    /// re-transmitting the path string.
    pub path_name_index: u32,
    /// Sparse field table. Index = handle, value = export descriptor.
    /// The length equals the declared `num_exports` (may grow on re-export).
    pub fields: Vec<Option<NetFieldExport>>,
}

impl NetFieldExportGroup {
    /// Create a new group with `capacity` empty field slots.
    pub fn new(path: String, path_name_index: u32, capacity: u32) -> Self {
        Self {
            path,
            path_name_index,
            fields: vec![None; capacity as usize],
        }
    }

    /// Create a group while reporting allocation failure to an untrusted-wire
    /// caller instead of relying on infallible `vec!` growth.
    pub(crate) fn try_new(
        path: String,
        path_name_index: u32,
        capacity: u32,
    ) -> crate::error::Result<Self> {
        let capacity_usize = usize::try_from(capacity)
            .map_err(|_| crate::error::SchemaError::FieldAllocationFailed { count: capacity })?;
        let mut fields = Vec::new();
        fields
            .try_reserve_exact(capacity_usize)
            .map_err(|_| crate::error::SchemaError::FieldAllocationFailed { count: capacity })?;
        fields.resize_with(capacity_usize, || None);
        Ok(Self {
            path,
            path_name_index,
            fields,
        })
    }

    /// Number of declared field slots (including unfilled ones).
    #[must_use]
    pub fn len(&self) -> u32 {
        self.fields.len() as u32
    }

    /// Whether no field slots have been declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Look up a field by its numeric handle. Returns `None` if the handle is
    /// out of range or the slot is unpopulated.
    #[must_use]
    pub fn get_field(&self, handle: u32) -> Option<&NetFieldExport> {
        self.fields.get(handle as usize)?.as_ref()
    }

    /// Insert or overwrite a field at the given handle.
    ///
    /// Returns `true` if the handle was within range and the write succeeded.
    /// Returns `false` (without panicking) if the handle exceeds the declared
    /// group length -- the C# reference simply logs a warning and skips.
    pub fn set_field(&mut self, field: NetFieldExport) -> bool {
        let idx = field.handle as usize;
        if idx >= self.fields.len() {
            return false;
        }
        self.fields[idx] = Some(field);
        true
    }

    /// Iterate all populated (non-`None`) fields in handle order.
    ///
    /// Useful for manifest serialization where only declared fields are relevant.
    pub fn populated_fields(&self) -> impl Iterator<Item = &NetFieldExport> {
        self.fields.iter().filter_map(|slot| slot.as_ref())
    }

    /// Merge another group's fields into this one, growing the field vector if
    /// the incoming group is larger. Existing non-`None` fields in `other`
    /// overwrite those in `self`.
    pub fn merge_from(&mut self, other: &NetFieldExportGroup) {
        if other.fields.len() > self.fields.len() {
            self.fields.resize(other.fields.len(), None);
        }
        for (i, slot) in other.fields.iter().enumerate() {
            if let Some(field) = slot {
                self.fields[i] = Some(field.clone());
            }
        }
    }
}
