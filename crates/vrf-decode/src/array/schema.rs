//! Nesting schema for the RepLayout struct arrays the walker knows how to
//! descend into.
//!
//! The walker in [`super`] can flatten any array's framing without help, but it
//! cannot tell a nested array apart from an opaque leaf payload: both arrive as
//! `handle + payloadBits + bits`. The schema is what says "handle 4 at this
//! level is itself an array" -- so it decides the shape of the emitted paths,
//! not just their labels.

/// Schema for array fields: maps handle -> sub-array schema (if the field is
/// itself a nested array) or None (leaf/primitive field).
///
/// Built from the C# descriptor knowledge. The handle numbers are from the
/// `CombatRoundReportsDecoder` and related descriptors.
#[derive(Debug, Clone)]
pub struct ArrayFieldSchema {
    /// For each handle in this struct level, whether it's a sub-array.
    /// Key = handle, Value = schema for the sub-array's element struct.
    pub sub_arrays: &'static [(u32, &'static ArrayFieldSchema)],
    /// Optional handle -> field name mapping for human-readable output.
    /// Only leaf fields need names here; sub-array fields get their name from
    /// the path segment they introduce.
    pub field_names: &'static [(u32, &'static str)],
}

impl ArrayFieldSchema {
    /// The sub-array schema for `handle`, if this level declares one.
    ///
    /// Linear: the longest level here has two entries, so an index would cost
    /// more than it saves.
    #[must_use]
    pub(super) fn sub_array(&self, handle: u32) -> Option<&'static ArrayFieldSchema> {
        self.sub_arrays
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, sub)| *sub)
    }

    /// The declared name for `handle` at this level, if there is one.
    ///
    /// Container handles are listed in `field_names` alongside the leaves --
    /// handle 4 is both a `sub_arrays` entry and the name `Reports` -- so this
    /// one map answers for both kinds.
    #[must_use]
    pub(super) fn field_name(&self, handle: u32) -> Option<&'static str> {
        self.field_names
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, name)| *name)
    }
}

// -- CombatRoundReports schema ------------------------------------------------
//
// Derived from CombatRoundReports.cs (handles confirmed against the manifest):
//
// Rounds[] (top array)
//   handle 3: RoundNumber (Int32)
//   handle 4: Reports[] (sub-array)
//     handle 5: RoundNumber (Int32)
//     handle 10: Interactions[] (sub-array)
//       handle 11: Subject (FString)
//       handle 12: Team (FName)
//       handle 13: CharacterIcon (ObjectNetGuid)
//       handle 18: DamageDealt (Float)
//       handle 19: HitsDealt (Int32)
//       handle 20: DamageReceived (Float)
//       handle 21: HitsReceived (Int32)
//       handle 22: DidKill (Bool)
//       handle 23: AssistType (EnumByte)
//       handle 24: KillerPlayerState (ObjectNetGuid)
//       handle 25: WasKiller (Bool)
//       handle 26: DealtInteractions[] (sub-array)
//         handle 44: Regions[] (sub-array)
//           handle 45: Region (EnumByte)
//           handle 46: Hits (Int32)
//           handle 47: Damage (Float)
//           handle 48: IsWallPen (Bool)
//           handle 49: IsKill (Bool)
//           handle 50: DestroyedArmor (ObjectNetGuid)
//       handle 61: ReceivedInteractions[] (sub-array)
//         handle 79: Regions[] (sub-array)
//           handle 80: Region (EnumByte)
//           handle 81: Hits (Int32)
//           handle 82: Damage (Float)
//           handle 83: IsWallPen (Bool)
//           handle 84: IsKill (Bool)
//           handle 85: DestroyedArmor (ObjectNetGuid)
//       handle 96: CombatReportIndex (Int32)
//       handle 98: ResurrectorPlayerState (ObjectNetGuid)
//       handle 103: Died (Bool)

/// Regional damage interaction -- leaf level (no sub-arrays).
static REGION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[
        (45, "Region"),
        (46, "Hits"),
        (47, "Damage"),
        (48, "IsWallPen"),
        (49, "IsKill"),
        (50, "DestroyedArmor"),
        // ReceivedInteractions uses different handles for the same fields:
        (80, "Region"),
        (81, "Hits"),
        (82, "Damage"),
        (83, "IsWallPen"),
        (84, "IsKill"),
        (85, "DestroyedArmor"),
    ],
};

/// Dealt interaction regions: handle 44 -> Regions sub-array.
static DEALT_INTERACTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(44, &REGION_SCHEMA)],
    field_names: &[(44, "Regions")],
};

/// Received interaction regions: handle 79 -> Regions sub-array.
static RECEIVED_INTERACTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(79, &REGION_SCHEMA)],
    field_names: &[(79, "Regions")],
};

/// Participant interaction: handles 26, 61 are sub-arrays.
static PARTICIPANT_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[
        (26, &DEALT_INTERACTION_SCHEMA),
        (61, &RECEIVED_INTERACTION_SCHEMA),
    ],
    field_names: &[
        (11, "Subject"),
        (12, "Team"),
        (13, "CharacterIcon"),
        (18, "DamageDealt"),
        (19, "HitsDealt"),
        (20, "DamageReceived"),
        (21, "HitsReceived"),
        (22, "DidKill"),
        (23, "AssistType"),
        (24, "KillerPlayerState"),
        (25, "WasKiller"),
        (26, "DealtInteractions"),
        (61, "ReceivedInteractions"),
        (96, "CombatReportIndex"),
        (98, "ResurrectorPlayerState"),
        (103, "Died"),
    ],
};

/// Character combat report: handle 10 -> Interactions sub-array.
static CHARACTER_REPORT_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(10, &PARTICIPANT_SCHEMA)],
    field_names: &[
        (5, "RoundNumber"),
        (10, "Interactions"),
        (98, "ResurrectorPlayerState"),
        (103, "Died"),
    ],
};

/// Round-level schema: handle 4 -> Reports sub-array.
pub static COMBAT_ROUNDS_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(4, &CHARACTER_REPORT_SCHEMA)],
    field_names: &[(3, "RoundNumber"), (4, "Reports")],
};
