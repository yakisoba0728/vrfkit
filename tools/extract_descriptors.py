"""Extract field type descriptors from the upstream C# source and emit a Rust
overlay table mapping (group_path, field_name) -> FieldType.

Scans every .cs file under the Replay.Valorant directory tree for descriptor
classes (subclasses of ExportGroupDescriptor) and extracts:
  - The group path (from `override string Path =>`)
  - Each field's export name and type (from AddProperty(...).Type() calls)

For classes that inherit from a base with Configure() (like agent descriptors
inheriting GenericAgentDescriptor), the parent's fields are propagated to each
child path.

Additionally handles:
  - Multi-class files: each class's Configure() method is identified and its
    fields are attributed to the correct class.
  - ClassNetCache descriptors: AddFunction<TParams>/AddFunctionHandle<TParams>
    calls emit Skip entries for the function names, ensuring coverage analysis
    counts them as covered.
  - Internal sealed class parameter descriptors defined alongside their parent
    ClassNetCache descriptor.

Fields using custom IFieldDecoder implementations or RawPayload are classified
as Raw. Fields using .Ignore() are classified as Skip.

Literal AddPropertyHandle declarations also emit a separate handle-to-name
table. Runtime lookup remains name-first; the handle table is only a fallback
for replays whose field name differs from the descriptor's label.

Usage:
    python tools/extract_descriptors.py <replay_valorant_dir> <out.rs>
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from collections import Counter

# Known primitive type method names -> Rust FieldType variant
PRIMITIVE_TYPES = {
    "Int32": "FieldType::Int32",
    "UInt32": "FieldType::UInt32",
    "UInt64": "FieldType::UInt64",
    "Float": "FieldType::Float",
    "Double": "FieldType::Double",
    "Bool": "FieldType::Bool",
    "Byte": "FieldType::Byte",
    "EnumByte": "FieldType::EnumByte",
    "FString": "FieldType::FString",
    "FName": "FieldType::FName",
    "ObjectNetGuid": "FieldType::ObjectNetGuid",
    "Guid": "FieldType::Guid",
    "EnumRemainingBits": "FieldType::EnumRemainingBits",
    "FGameplayTag": "FieldType::GameplayTag",
    "FVector": "FieldType::VectorDouble",
    "FVectorNetQuantize": "FieldType::VectorNetQuantize { scale: 1 }",
    "FVectorNetQuantize10": "FieldType::VectorNetQuantize { scale: 10 }",
    "FVectorNetQuantize100": "FieldType::VectorNetQuantize { scale: 100 }",
    "FVectorNetQuantizeNormal": "FieldType::VectorNetQuantizeNormal",
    "FRotatorShort": "FieldType::RotationShort",
    "Transform": "FieldType::Transform",
    "Ignore": "FieldType::Skip",
}

# Regex for path declaration
PATH_RE = re.compile(
    r'override\s+string\s+Path\s*=>\s*"(?P<path>[^"]+)"'
)

# Regex for class declaration with base class
CLASS_RE = re.compile(
    r'(?:public|internal)\s+(?:sealed\s+)?class\s+(\w+)\s*'
    r'(?::\s*(\w+(?:<[^>]+>)?))?'
)

EXPORT_CATEGORY_NAMES = {
    "None",
    "Movement",
    "Ability",
    "Gunplay",
    "Agent",
    "GameState",
    "Inventory",
    "Economy",
    "Effects",
    "Visibility",
    "Debug",
    "All",
}
CATEGORY_OVERRIDE_MARKER_RE = re.compile(
    r'\boverride\s+ExportCategory\s+Categories\b'
)
CATEGORY_OVERRIDE_RE = re.compile(
    r'\boverride\s+ExportCategory\s+Categories\s*=>\s*'
    r'(?P<expression>[^;]+)\s*;'
)
CATEGORY_EXPRESSION_RE = re.compile(
    r'\s*ExportCategory\.\w+'
    r'(?:\s*\|\s*ExportCategory\.\w+)*\s*'
)

# Regex for AddProperty with explicit export name
ADD_PROP_NAMED_RE = re.compile(
    r'AddProperty\(\s*"(?P<name>[^"]+)"'
    r'[^)]*\)'
    r'\.(?P<type>\w+)\('
)

# Regex for AddProperty with inferred name (from lambda)
ADD_PROP_LAMBDA_RE = re.compile(
    r'AddProperty\(\s*\w+\s*=>\s*(?:\w+\.)?(?P<name>\w+)'
    r'[^)]*\)'
    r'\.(?P<type>\w+)\('
)

# The handle argument of AddPropertyHandle.
#
# Usually a literal, but a descriptor may factor a run of handles into a helper
# that takes the first one: MulticastNotifyDamageBaseParameters.cs:24 declares
# AddDeathFields(uint firstHandle) and calls it as AddDeathFields(32), so its
# six statements read `firstHandle` and `firstHandle + 5`.
#
# Requiring a literal here made those six invisible. The table is keyed on
# (group_path, field_name) and never reads the handle, so the value does not
# need resolving -- only the shape has to be recognised. The trailing comma in
# every caller is what keeps this from swallowing a lambda: `x => x.Prop` has
# no comma after `x`.
HANDLE_ARG = r'(?:\d+|[A-Za-z_]\w*(?:\s*\+\s*\d+)?)'

# SerializedInt(maxValue: N) or SerializedInt(N)
SERIALIZED_INT_RE = re.compile(
    r'\.SerializedInt\(\s*(?:maxValue:\s*)?(\d+)\s*\)'
)

# ByteArray(maxBytes) or ByteArray(N)
BYTE_ARRAY_RE = re.compile(
    r'\.ByteArray\(\s*(?:maxBytes:\s*)?(\d+)\s*\)'
)

# ReplicatedMovement with rotation quantization
REP_MOVEMENT_RE = re.compile(
    r'\.ReplicatedMovement\(\s*ERotatorQuantization\.(?P<quant>\w+)\s*\)'
)

# Simple .ReplicatedMovement() (defaults to ShortComponents)
REP_MOVEMENT_DEFAULT_RE = re.compile(
    r'\.ReplicatedMovement\(\s*\)'
)

# RepLayoutDynamicArray<T>() — captures the inner type for documentation;
# treated as Raw because we cannot decode the TArray wire format generically.
REP_LAYOUT_DYN_ARRAY_RE = re.compile(
    r'\.RepLayoutDynamicArray<\w+>\(\s*\)'
)

# Decode(...) -- custom decoder, classified as Raw
DECODE_RE = re.compile(
    r'\.Decode\('
)

# ClassNetCache AddFunction patterns
ADD_FUNCTION_RE = re.compile(
    r'AddFunction(?:Handle)?(?:<(?P<params>\w+)>)?\s*\(\s*'
    r'(?:(?P<handle>\d+)\s*,\s*)?'
    r'"(?P<name>[^"]+)"'
)

# Expression-bodied wrappers whose implementation delegates to
# AddPropertyHandle(...).Decode(...). DamageParameters<T>.AddRaw is the live
# example. Discover the wrapper from its implementation instead of baking the
# helper's name into the extractor.
RAW_WRAPPER_DEF_RE = re.compile(
    r'(?:public|internal|protected|private)\s+(?:static\s+)?void\s+'
    r'(?P<name>\w+)\s*\([^)]*\)\s*=>\s*'
    r'AddPropertyHandle\([^;]+\)\s*\.Decode\(',
    re.DOTALL,
)


def extract_path(source: str) -> str | None:
    """Extract the Path property from a descriptor class."""
    m = PATH_RE.search(source)
    return m.group("path") if m else None


def extract_class_info(source: str) -> tuple[str | None, str | None]:
    """Extract (class_name, base_class_name) from the source."""
    m = CLASS_RE.search(source)
    if not m:
        return (None, None)
    class_name = m.group(1)
    base = m.group(2)
    # Strip generic parameter
    if base and "<" in base:
        base = base.split("<")[0]
    return (class_name, base)


def extract_fields_from_block(
    block: str, raw_wrapper_names: set[str] | None = None
) -> list[tuple[str, str, int | None]]:
    """Extract (field_export_name, rust_type, literal_handle) tuples.

    Handles AddProperty and AddPropertyHandle patterns, including multi-line
    statements where the .Type() or .Decode() call is on a continuation line.
    ``literal_handle`` is populated only when the declaration supplies a
    concrete decimal handle; unresolved helper parameters remain ``None``.
    """
    if raw_wrapper_names is None:
        raw_wrapper_names = set()
    fields = []

    # Join continuation lines: if a line starts with AddProperty but does not
    # end with ';', concatenate subsequent lines until we see one ending with ';'.
    raw_lines = block.splitlines()
    statements: list[str] = []
    current: list[str] = []
    in_statement = False
    for raw_line in raw_lines:
        stripped = raw_line.strip()
        starts_raw_wrapper = any(
            re.match(rf'{re.escape(name)}\s*\(', stripped)
            for name in raw_wrapper_names
        )
        if stripped.startswith("AddProperty") or starts_raw_wrapper:
            in_statement = True
            current = [stripped]
            if stripped.endswith(";"):
                statements.append(" ".join(current))
                in_statement = False
                current = []
        elif in_statement:
            current.append(stripped)
            if stripped.endswith(";"):
                statements.append(" ".join(current))
                in_statement = False
                current = []
    # Flush any incomplete statement
    if current:
        statements.append(" ".join(current))

    for line in statements:

        raw_wrapper = next(
            (
                name
                for name in raw_wrapper_names
                if re.match(rf'{re.escape(name)}\s*\(', line)
            ),
            None,
        )
        if raw_wrapper is not None:
            name = _extract_lambda_field_name(line)
            if name:
                fields.append((name, "FieldType::Raw", _extract_literal_handle(line)))
            continue

        # Check for SerializedInt with parameter
        sm = SERIALIZED_INT_RE.search(line)
        if sm:
            max_val = sm.group(1)
            name = _extract_field_name(line)
            if name:
                fields.append((
                    name,
                    f"FieldType::SerializedInt {{ max: {max_val} }}",
                    _extract_literal_handle(line),
                ))
            continue

        # Check for ByteArray with parameter
        bm = BYTE_ARRAY_RE.search(line)
        if bm:
            max_bytes = bm.group(1)
            name = _extract_field_name(line)
            if name:
                fields.append((
                    name,
                    f"FieldType::ByteArray {{ max_bytes: {max_bytes} }}",
                    _extract_literal_handle(line),
                ))
            continue

        # Check for ReplicatedMovement with quantization
        rm = REP_MOVEMENT_RE.search(line)
        if rm:
            quant = rm.group("quant")
            rust_quant = ("RotatorQuantization::ByteComponents"
                          if quant == "ByteComponents"
                          else "RotatorQuantization::ShortComponents")
            name = _extract_field_name(line)
            if name:
                fields.append((
                    name,
                    f"FieldType::RepMovement {{ rotation: {rust_quant} }}",
                    _extract_literal_handle(line),
                ))
            continue

        # Simple ReplicatedMovement()
        if REP_MOVEMENT_DEFAULT_RE.search(line):
            name = _extract_field_name(line)
            if name:
                fields.append((
                    name,
                    "FieldType::RepMovement { rotation: RotatorQuantization::ShortComponents }",
                    _extract_literal_handle(line),
                ))
            continue

        # RepLayoutDynamicArray<T>() -- treated as Raw (opaque TArray)
        if REP_LAYOUT_DYN_ARRAY_RE.search(line):
            name = _extract_field_name(line)
            if name:
                fields.append((name, "FieldType::Raw", _extract_literal_handle(line)))
            continue

        # Check for Decode(...) -- custom/raw
        if DECODE_RE.search(line):
            name = _extract_field_name(line)
            if name:
                fields.append((name, "FieldType::Raw", _extract_literal_handle(line)))
            continue

        # Try simple primitive type
        type_name = _extract_type_name(line)
        if type_name and type_name in PRIMITIVE_TYPES:
            name = _extract_field_name(line)
            if name:
                fields.append((
                    name,
                    PRIMITIVE_TYPES[type_name],
                    _extract_literal_handle(line),
                ))
            continue

    return fields


def extract_cnc_functions(block: str) -> list[str]:
    """Extract function names from a ClassNetCache Configure() block.

    Returns list of function export names (e.g. "MulticastEndRound").
    """
    names = []
    for m in ADD_FUNCTION_RE.finditer(block):
        names.append(m.group("name"))
    return names


def extract_called_cnc_helper_functions(
    class_body: str, configure_body: str
) -> list[str]:
    """Extract AddFunction calls from parameterless helpers Configure invokes.

    Limiting the scan to called helpers prevents an unused method from silently
    becoming a generated descriptor entry.
    """
    names = []
    helper_calls = re.findall(r'(?m)^\s*(\w+)\s*\(\s*\)\s*;', configure_body)
    for helper_name in helper_calls:
        declaration = re.search(
            rf'(?:public|internal|protected|private)\s+(?:static\s+)?void\s+'
            rf'{re.escape(helper_name)}\s*\(\s*\)',
            class_body,
        )
        if declaration is None:
            continue
        body_start = class_body.find('{', declaration.end())
        if body_start == -1:
            continue
        _, body_end = find_class_body_range(class_body, declaration.start())
        names.extend(extract_cnc_functions(class_body[body_start:body_end]))
    return names


def _extract_field_name(line: str) -> str | None:
    """Extract the export name from an AddProperty line.

    Handles both single-line and joined multi-line statements.
    """
    # Try named variant first: AddProperty("ExportName", ...)
    m = re.search(r'AddProperty\w*\(\s*"([^"]+)"', line)
    if m:
        return m.group(1)
    # Try handle variant with explicit string name:
    # AddPropertyHandle(N, "ExportName", ...)
    m = re.search(r'AddPropertyHandle\(\s*' + HANDLE_ARG + r'\s*,\s*"([^"]+)"', line)
    if m:
        return m.group(1)
    # Try handle variant with lambda:
    # AddPropertyHandle(N, x => x.Name, ...)
    m = re.search(
        r'AddPropertyHandle\(\s*' + HANDLE_ARG + r'\s*,\s*\w+\s*=>\s*(?:\w+\.)?(\w+)', line
    )
    if m:
        return m.group(1)
    # Try lambda variant: AddProperty(x => x.Name, ...) or AddProperty(x => x.Name)
    m = re.search(r'AddProperty\(\s*\w+\s*=>\s*(?:\w+\.)?(\w+)', line)
    if m:
        return m.group(1)
    return None


def _extract_lambda_field_name(line: str) -> str | None:
    """Extract the property leaf from any helper invocation containing a lambda."""
    match = re.search(r'\w+\s*=>\s*(?:\w+\.)?(\w+)', line)
    return match.group(1) if match else None


def _extract_literal_handle(line: str) -> int | None:
    """Return a declaration's first argument when it is a decimal literal."""
    match = re.search(r'\w+\s*\(\s*(\d+)\s*,', line)
    return int(match.group(1)) if match else None


def _extract_type_name(line: str) -> str | None:
    """Extract the type method name (the .Type() call) from a line."""
    m = re.search(r'\)\s*\.(\w+)\(', line)
    return m.group(1) if m else None


def find_class_body_range(source: str, class_start: int) -> tuple[int, int]:
    """Find the range of the class body (between opening and closing braces).

    Returns (body_start, body_end) where body_start is right after the opening
    brace and body_end is at the closing brace.
    """
    # Find the opening brace after the class declaration
    brace_pos = source.find('{', class_start)
    if brace_pos == -1:
        return (class_start, len(source))

    depth = 1
    pos = brace_pos + 1
    while pos < len(source) and depth > 0:
        ch = source[pos]
        if ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
        pos += 1

    return (brace_pos + 1, pos - 1)


def extract_parameterless_method_body(source: str, method_name: str) -> str | None:
    """Return only the named parameterless method's block or expression body."""
    declaration_re = re.compile(
        rf'\b(?:public|internal|protected|private)\s+(?:static\s+)?'
        rf'[\w.<>,?\[\]]+\s+{re.escape(method_name)}\s*\(\s*\)\s*'
    )
    declaration = declaration_re.search(source)
    if declaration is None:
        return None

    body_start = declaration.end()
    if source.startswith("=>", body_start):
        expression_start = body_start + 2
        expression_end = source.find(";", expression_start)
        if expression_end == -1:
            return None
        return source[expression_start:expression_end]

    if body_start >= len(source) or source[body_start] != "{":
        return None

    depth = 1
    pos = body_start + 1
    while pos < len(source) and depth > 0:
        if source[pos] == "{":
            depth += 1
        elif source[pos] == "}":
            depth -= 1
        pos += 1

    if depth != 0:
        return None
    return source[body_start + 1:pos - 1]


def extract_category_override(
    class_name: str, class_body: str
) -> frozenset[str] | None:
    """Parse a class's explicit category flags, rejecting unsupported forms."""
    markers = list(CATEGORY_OVERRIDE_MARKER_RE.finditer(class_body))
    if not markers:
        return None

    overrides = list(CATEGORY_OVERRIDE_RE.finditer(class_body))
    if (
        len(markers) != 1
        or len(overrides) != 1
        or markers[0].start() != overrides[0].start()
    ):
        raise SystemExit(
            f"{class_name}: unsupported ExportCategory override; "
            "expected an expression joined with |"
        )

    expression = overrides[0].group("expression")
    if CATEGORY_EXPRESSION_RE.fullmatch(expression) is None:
        raise SystemExit(
            f"{class_name}: unsupported ExportCategory override expression "
            f"{expression.strip()!r}"
        )

    categories = frozenset(re.findall(r'ExportCategory\.(\w+)', expression))
    unknown = sorted(categories - EXPORT_CATEGORY_NAMES)
    if unknown:
        raise SystemExit(
            f"{class_name}: unknown ExportCategory {', '.join(unknown)}"
        )
    return categories


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        raise SystemExit(__doc__)

    src_dir = Path(argv[1])
    out_path = Path(argv[2])

    if not src_dir.is_dir():
        raise SystemExit(f"source directory not found: {src_dir}")

    # Phase 1: scan all files, build class -> (path, fields) map
    # and class -> base_class map for inheritance.
    # Some files contain multiple class declarations (e.g. weapon descriptors).
    class_paths: dict[str, str] = {}        # class_name -> path
    class_fields: dict[str, list[tuple[str, str, int | None]]] = {}
    class_bases: dict[str, str] = {}        # class_name -> base_class_name
    class_category_overrides: dict[str, frozenset[str]] = {}
    # ClassNetCache function names -> Skip entries
    cnc_functions: dict[str, list[str]] = {}  # class_name -> function names
    runtime_cnc_specs: list[tuple[str, str]] = []  # (path suffix, function name)

    # Multi-class regex: find ALL class declarations in a file.
    # Handles generic type parameters (<T>) and optional `where` constraint
    # clauses that appear between the base-class list and the opening brace.
    MULTI_CLASS_RE = re.compile(
        r'(?:public|internal)\s+(?:sealed\s+)?(?:abstract\s+)?class\s+(\w+)'
        r'(?:<[^>]+>)?\s*'
        r'(?::\s*([\w.<>,\s]+?))?'
        r'(?:\s+where\s+[^\{]+?)?'
        r'\s*\{',
        re.DOTALL,
    )

    cs_files = sorted(src_dir.rglob("*.cs"))
    sources = [
        (cs_file, cs_file.read_text(encoding="utf-8-sig"))
        for cs_file in cs_files
    ]
    raw_wrapper_names = {
        match.group("name")
        for _, source in sources
        for match in RAW_WRAPPER_DEF_RE.finditer(source)
    }

    # Runtime-created ClassNetCaches do not have descriptor classes or literal
    # paths. Discover the constructor shape, resolve its RpcDescriptor.Name,
    # and later apply it to descriptor paths in the Agent category.
    runtime_cache_re = re.compile(
        r'\w+\.Path\s*\+\s*"(?P<suffix>[^"]+)"\s*,\s*'
        r'\[\s*(?P<factory>\w+)\s*\(\s*\)\s*\]',
        re.DOTALL,
    )
    const_string_re = re.compile(
        r'\bconst\s+string\s+(?P<name>\w+)\s*=\s*"(?P<value>[^"]+)"'
    )
    for _, source in sources:
        constants = {
            match.group("name"): match.group("value")
            for match in const_string_re.finditer(source)
        }
        for runtime_match in runtime_cache_re.finditer(source):
            factory_name = runtime_match.group("factory")
            factory_body = extract_parameterless_method_body(source, factory_name)
            if factory_body is None:
                raise SystemExit(
                    f"runtime ClassNetCache factory {factory_name}: "
                    "method body not found"
                )
            name_match = re.search(
                r'\bName\s*=\s*(?:"(?P<literal>[^"]+)"|(?P<constant>\w+))',
                factory_body,
            )
            if name_match is None:
                raise SystemExit(
                    f"runtime ClassNetCache factory {factory_name}: "
                    "RpcDescriptor.Name not found in method body"
                )
            function_name = name_match.group("literal")
            if function_name is None:
                function_name = constants.get(name_match.group("constant"))
            if function_name is None:
                constant_name = name_match.group("constant")
                raise SystemExit(
                    f"runtime ClassNetCache factory {factory_name}: "
                    f"RpcDescriptor.Name constant {constant_name} could not be resolved"
                )
            runtime_cnc_specs.append(
                (runtime_match.group("suffix"), function_name)
            )

    for cs_file, source in sources:

        # Find all class declarations in this file
        class_matches = list(MULTI_CLASS_RE.finditer(source))

        for cm in class_matches:
            class_name = cm.group(1)
            raw_base = cm.group(2)
            if raw_base:
                # Strip generic params: "WeaponEquippableDescriptor<Foo>" -> "WeaponEquippableDescriptor"
                base_class = raw_base.strip().split("<")[0].strip()
                # Handle comma-separated bases: take the first one
                base_class = base_class.split(",")[0].strip()
                class_bases[class_name] = base_class

        # Build class positions with body ranges
        class_positions = [(cm.start(), cm.group(1)) for cm in class_matches]

        # Extract Path for each class (find Path declarations and associate
        # with the class whose body contains them)
        for cm in class_matches:
            class_name = cm.group(1)
            body_start, body_end = find_class_body_range(source, cm.start())
            class_body = source[body_start:body_end]

            category_override = extract_category_override(class_name, class_body)
            if category_override is not None:
                class_category_overrides[class_name] = category_override

            # Extract Path declarations within this class body
            path_match = PATH_RE.search(class_body)
            if path_match:
                class_paths[class_name] = path_match.group("path")

            # Check if this is a ClassNetCache descriptor
            raw_base = cm.group(2)
            is_cnc = raw_base and "ClassNetCacheDescriptor" in raw_base

            # Find Configure() methods within this class body
            configure_re = re.compile(
                r'(?:protected\s+)?override\s+void\s+Configure\(\)'
            )
            configure_match = configure_re.search(class_body)
            if configure_match:
                # Find the body of Configure() -- it starts at the next { after
                # the match.
                cfg_start = class_body.find('{', configure_match.end())
                if cfg_start != -1:
                    # Find matching closing brace
                    depth = 1
                    pos = cfg_start + 1
                    while pos < len(class_body) and depth > 0:
                        if class_body[pos] == '{':
                            depth += 1
                        elif class_body[pos] == '}':
                            depth -= 1
                        pos += 1
                    configure_body = class_body[cfg_start:pos]

                    if is_cnc:
                        # Extract function names for CNC
                        funcs = extract_cnc_functions(configure_body)
                        funcs.extend(
                            extract_called_cnc_helper_functions(
                                class_body, configure_body
                            )
                        )
                        if funcs:
                            cnc_functions[class_name] = funcs
                    else:
                        # Extract fields for ExportGroupDescriptor
                        fields = extract_fields_from_block(
                            configure_body, raw_wrapper_names
                        )
                        if fields:
                            class_fields[class_name] = fields

            # Also look for helper methods (AddSharedFields, AddDeathFields,
            # etc.) that define fields -- these are called from Configure() but
            # defined as separate methods in the same class body.
            if not is_cnc:
                # Look for methods that contain AddProperty calls but aren't
                # Configure(). These are helper methods.
                helper_fields = extract_fields_from_block(
                    class_body, raw_wrapper_names
                )
                if helper_fields and class_name not in class_fields:
                    class_fields[class_name] = helper_fields
                elif helper_fields and class_name in class_fields:
                    # Merge helper fields not already captured by Configure()
                    existing_names = {n for n, _, _ in class_fields[class_name]}
                    for n, t, h in helper_fields:
                        if n not in existing_names:
                            class_fields[class_name].append((n, t, h))
                            existing_names.add(n)

    # Phase 2: resolve inheritance -- propagate fields from base classes
    # For classes with a Path but no fields, inherit from their base.
    # For classes that DO have fields, also merge parent fields (handles the
    # RPC parameter pattern where child.Configure() calls parent.AddSharedFields()
    # which declares additional fields not in the child's own Configure()).
    def get_fields(
        cls: str, visited: set[str] | None = None
    ) -> list[tuple[str, str, int | None]]:
        if visited is None:
            visited = set()
        if cls in visited:
            return []
        visited.add(cls)
        own_fields = class_fields.get(cls, [])
        base = class_bases.get(cls)
        if base:
            base_plain = base.split("<")[0] if "<" in base else base
            parent_fields = get_fields(base_plain, visited)
            if parent_fields and own_fields:
                # Merge: parent fields first, then child (child may override).
                own_names = {name for name, _, _ in own_fields}
                merged = [
                    (n, t, h)
                    for n, t, h in parent_fields
                    if n not in own_names
                ]
                merged.extend(own_fields)
                return merged
            if parent_fields:
                return parent_fields
        return own_fields

    # Phase 3: build final entries
    entries: list[tuple[str, str, str]] = []  # (group_path, field_name, rust_type)
    handle_entries: list[tuple[str, int, str]] = []
    type_counts: Counter[str] = Counter()
    raw_count = 0
    skip_count = 0
    groups_seen: set[str] = set()

    # 3a: ExportGroupDescriptor entries (RepLayout + RPC parameter groups)
    for class_name, path in sorted(class_paths.items()):
        fields = get_fields(class_name)
        if not fields:
            continue
        # Skip classes that are CNC descriptors (they go to 3b)
        base = class_bases.get(class_name, "")
        if "ClassNetCacheDescriptor" in base:
            continue
        groups_seen.add(path)
        for field_name, rust_type, literal_handle in fields:
            entries.append((path, field_name, rust_type))
            if literal_handle is not None:
                handle_entries.append((path, literal_handle, field_name))
            if rust_type == "FieldType::Raw":
                raw_count += 1
            elif rust_type == "FieldType::Skip":
                skip_count += 1
            else:
                base_type = rust_type.split("{")[0].strip().replace("FieldType::", "")
                type_counts[base_type] += 1

    # 3b: ClassNetCache function entries
    # Each function in a CNC group gets a Skip entry. This tells analyze_coverage
    # that the group IS covered, and preserves the function name in the overlay
    # table for documentation. The actual type decoding for RPC parameters
    # happens via the parameter group entries from 3a.
    for class_name, funcs in sorted(cnc_functions.items()):
        path = class_paths.get(class_name)
        if not path:
            continue
        groups_seen.add(path)
        for func_name in funcs:
            entries.append((path, func_name, "FieldType::Skip"))
            skip_count += 1

    def has_effective_agent_category(cls: str) -> bool:
        visited: set[str] = set()
        current = cls
        while current not in visited:
            visited.add(current)
            categories = class_category_overrides.get(current)
            if categories is not None:
                return "Agent" in categories or "All" in categories
            base = class_bases.get(current)
            if base is None:
                return False
            current = base
        return False

    # 3c: runtime-created ClassNetCache entries. The live factory builds one
    # cache for every Agent-category descriptor using `descriptor.Path +
    # "_ClassNetCache"` and a shared RpcDescriptor. The constructor suffix and
    # RPC name were parsed from source above; no replay path or alias is baked
    # into the generator.
    for suffix, function_name in sorted(set(runtime_cnc_specs)):
        for class_name, path in sorted(class_paths.items()):
            if not has_effective_agent_category(class_name):
                continue
            cache_path = path + suffix
            groups_seen.add(cache_path)
            entries.append((cache_path, function_name, "FieldType::Skip"))
            skip_count += 1

    # Deduplicate entries (same path + field_name can appear if parent+child both declare)
    seen_keys: set[tuple[str, str]] = set()
    deduped: list[tuple[str, str, str]] = []
    for entry in entries:
        key = (entry[0], entry[1])
        if key not in seen_keys:
            seen_keys.add(key)
            deduped.append(entry)
    entries = deduped

    handle_by_key: dict[tuple[str, int], str] = {}
    for group_path, handle, field_name in handle_entries:
        key = (group_path, handle)
        previous = handle_by_key.get(key)
        if previous is not None and previous != field_name:
            raise SystemExit(
                "conflicting explicit handles for "
                f"{group_path} handle {handle}: {previous!r} vs {field_name!r}"
            )
        handle_by_key[key] = field_name
    handle_entries = [
        (group_path, handle, field_name)
        for (group_path, handle), field_name in handle_by_key.items()
    ]

    # Sort by (group_path, field_name) for binary search
    entries.sort(key=lambda e: (e[0], e[1]))
    handle_entries.sort(key=lambda e: (e[0], e[1]))

    # Recount after dedup
    raw_count = sum(1 for _, _, t in entries if t == "FieldType::Raw")
    skip_count = sum(1 for _, _, t in entries if t == "FieldType::Skip")
    type_counts = Counter()
    for _, _, t in entries:
        if t != "FieldType::Raw" and t != "FieldType::Skip":
            base_type = t.split("{")[0].strip().replace("FieldType::", "")
            type_counts[base_type] += 1

    # Report
    print(f"Groups: {len(groups_seen)}")
    print(f"Fields: {len(entries)}")
    print(f"  Raw (custom decoder): {raw_count}")
    print(f"  Skip (ignored): {skip_count}")
    print(f"  Typed: {len(entries) - raw_count - skip_count}")
    print(f"Handle aliases: {len(handle_entries)}")
    print("Type distribution:")
    for t, c in type_counts.most_common():
        print(f"  {t}: {c}")

    # Generate Rust source
    lines = [
        "// Overlay table mapping (group_path, field_name) -> FieldType.",
        "//",
        "// GENERATED by tools/extract_descriptors.py -- do not edit by hand.",
        f"// {len(entries)} entries from {len(groups_seen)} groups.",
        f"// Raw/Custom: {raw_count}, Skip: {skip_count}, Typed: {len(entries) - raw_count - skip_count}.",
        "",
        "use crate::decode::FieldType;",
        "use crate::overlay::{OverlayEntry, OverlayHandleEntry};",
        "use crate::types::RotatorQuantization;",
        "",
        f"pub static OVERLAY_TABLE: [OverlayEntry; {len(entries)}] = [",
    ]
    for group_path, field_name, rust_type in entries:
        gp = group_path.replace("\\", "\\\\").replace('"', '\\"')
        fn = field_name.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(
            f'    OverlayEntry {{ group_path: "{gp}", '
            f'field_name: "{fn}", '
            f'field_type: {rust_type} }},'
        )
    lines.append("];")
    lines.append("")
    lines.append(
        f"pub static OVERLAY_HANDLE_TABLE: [OverlayHandleEntry; "
        f"{len(handle_entries)}] = ["
    )
    for group_path, handle, field_name in handle_entries:
        gp = group_path.replace("\\", "\\\\").replace('"', '\\"')
        fn = field_name.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(
            f'    OverlayHandleEntry {{ group_path: "{gp}", '
            f'handle: {handle}, field_name: "{fn}" }},'
        )
    lines.append("];")
    lines.append("")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(lines), encoding="utf-8")
    print(f"\nwrote {out_path} ({len(entries)} entries)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
