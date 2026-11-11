//! Nesting schema for the RepLayout struct arrays the walker descends into. A
//! nested array and an opaque leaf both arrive as `handle + payloadBits +
//! bits`; only the schema says "handle 4 at this level is itself an array".
//! Leaves are named by the replay's own declaration.

/// One struct level: which handles are nested arrays, and names for handles.
#[derive(Debug, Clone)]
pub struct ArrayFieldSchema {
    /// `(handle, element schema)` for each handle that is itself an array.
    pub sub_arrays: &'static [(u32, &'static ArrayFieldSchema)],
    /// `(handle, name)` for readable paths; containers are named here too.
    pub field_names: &'static [(u32, &'static str)],
}

impl ArrayFieldSchema {
    /// The sub-array schema for `handle`, if this level declares one.
    #[must_use]
    pub(super) fn sub_array(&self, handle: u32) -> Option<&'static ArrayFieldSchema> {
        self.sub_arrays
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, sub)| *sub)
    }

    /// The name for `handle` at this level, if any.
    #[must_use]
    pub(super) fn field_name(&self, handle: u32) -> Option<&'static str> {
        self.field_names
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, name)| *name)
    }
}

/// A level with no nesting and no names of its own.
static LEAVES: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[],
};

// CombatReport: Rounds[] -> Reports[] at 4 -> Interactions[] at 10 ->
// DealtInteractions[] at 26 / ReceivedInteractions[] at 61 -> Regions[] at 44 / 79.

static DEALT_INTERACTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(44, &LEAVES)],
    field_names: &[(44, "Regions")],
};

static RECEIVED_INTERACTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(79, &LEAVES)],
    field_names: &[(79, "Regions")],
};

static PARTICIPANT_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[
        (26, &DEALT_INTERACTION_SCHEMA),
        (61, &RECEIVED_INTERACTION_SCHEMA),
    ],
    field_names: &[(26, "DealtInteractions"), (61, "ReceivedInteractions")],
};

static CHARACTER_REPORT_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(10, &PARTICIPANT_SCHEMA)],
    field_names: &[(10, "Interactions")],
};

/// `CombatReportComponent.Rounds`.
pub static COMBAT_ROUNDS_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(4, &CHARACTER_REPORT_SCHEMA)],
    field_names: &[(4, "Reports")],
};

// 13.10 declares a new member before the regional arrays. The replay's
// CombatReportComponent export names handles 45 and 81
// RegionalDamageInteractions, and handle 62 ReceivedInteractions. Keep the
// earlier schema separate so old handle 45 is not walked as an array.
static DEALT_INTERACTION_SCHEMA_1310: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(45, &LEAVES)],
    field_names: &[(45, "RegionalDamageInteractions")],
};

static RECEIVED_INTERACTION_SCHEMA_1310: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(81, &LEAVES)],
    field_names: &[(81, "RegionalDamageInteractions")],
};

static PARTICIPANT_SCHEMA_1310: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[
        (26, &DEALT_INTERACTION_SCHEMA_1310),
        (62, &RECEIVED_INTERACTION_SCHEMA_1310),
    ],
    field_names: &[(26, "DealtInteractions"), (62, "ReceivedInteractions")],
};

static CHARACTER_REPORT_SCHEMA_1310: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(10, &PARTICIPANT_SCHEMA_1310)],
    field_names: &[(10, "Interactions")],
};

/// `CombatReportComponent.Rounds` with 13.10's declared regional-array handles.
pub static COMBAT_ROUNDS_SCHEMA_1310: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(4, &CHARACTER_REPORT_SCHEMA_1310)],
    field_names: &[(4, "Reports")],
};

// AbilityCastsThisRound (`Comp_AbilityStatisticsReplicator`, one element per
// cast): Effects[] at 13 -> AffectedTargetsArray[] at 18, whose AffectedPlayer
// (19) is an IntPacked NetGUID of a BombPlayerState actor.

static AFFECTED_TARGET_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[(19, "AffectedPlayer"), (20, "Value")],
};

/// One statistic a cast produced, with the players it applied to. Public: a
/// bare `Effects` payload can be walked without the enclosing cast element.
pub static ABILITY_EFFECTS_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(18, &AFFECTED_TARGET_SCHEMA)],
    field_names: &[
        (14, "Statistic"),
        (15, "LocalizedStat"),
        (16, "Value"),
        (17, "Time"),
        (18, "AffectedTargetsArray"),
    ],
};

/// `Comp_AbilityStatisticsReplicator.AbilityCastsThisRound`.
pub static ABILITY_CASTS_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(13, &ABILITY_EFFECTS_SCHEMA)],
    field_names: &[(13, "Effects")],
};

// `DamageableComponent`'s five life-change RPCs send one struct array of the
// same four members; each RPC parameter numbers its own handles, and the
// export passes no declaration, hence three named schemas.

/// `MulticastNotifyDamage_Point` and `_Base`.
pub static LIFE_CHANGE_DAMAGE_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[
        (10, "ChangedComponent"),
        (11, "LifeResult"),
        (12, "DeltaLife"),
        (13, "bAliveAfterChange"),
    ],
};

/// 13.10 damage RPCs shift all four members one handle. The packed NetGUID at
/// handle 11 resolves to damage section objects in the replay's GUID table.
pub static LIFE_CHANGE_DAMAGE_SCHEMA_1310: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[
        (11, "ChangedComponent"),
        (12, "LifeResult"),
        (13, "DeltaLife"),
        (14, "bAliveAfterChange"),
    ],
};

/// `MulticastSectionLifeChange`, the round-reset broadcast.
pub static LIFE_CHANGE_SECTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[
        (1, "ChangedComponent"),
        (2, "LifeResult"),
        (3, "DeltaLife"),
        (4, "bAliveAfterChange"),
    ],
};

/// `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay`, whose parameter
/// is `LifeChangeBySection`.
pub static LIFE_CHANGE_BY_SECTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[
        (2, "ChangedComponent"),
        (3, "LifeResult"),
        (4, "DeltaLife"),
        (5, "bAliveAfterChange"),
    ],
};
