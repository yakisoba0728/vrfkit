"""Build the main-stream section timeline of one export: each section state's
observed predecessor, with a strict (time-ordered) and a packet-ordered
continuity decision. Adjacency is observation order, not effective HP, game
life or component life."""
from __future__ import annotations

import collections
import math
import struct
from contextlib import closing
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc

import extract_section_observations
from atomic_io import run_json_cli, sha256_file as sha
from wire_bits import iter_selected, text

RESET = "MulticastSectionLifeChange"
SIGNS = {"MulticastNotifyDamage_Base": 1, "MulticastNotifyDamage_Point": 1,
         "MulticastNotifyHeal": 1, "MulticastNotifyOverhealDecay": -1}
#: Strict reasons that packet order can resolve; every other strict reason stays.
REMOVED_STRICT_REASONS = {"same_time_tie", "prior_tie_censor", "lifecycle_unresolved",
                          "prior_lifecycle_unresolved", "actor_channel_instance_changed"}
INPUT_NAMES = ("manifest.json", "fields.parquet", "checkpoint_fields.parquet",
               "net_guids.parquet", "actors.parquet")
SOURCES = [Path(__file__).resolve(), *(Path(__file__).with_name(n) for n in (
    "extract_section_observations.py", "wire_bits.py", "atomic_io.py"))]


def _traces(rows):
    actors, channels = collections.defaultdict(list), collections.defaultdict(list)
    for ordinal, row in rows:
        if row.get("event") in ("open", "close"):
            actors[row["actor_net_guid"]].append((ordinal, row))
            channels[row["channel_index"]].append((ordinal, row))
    return actors, channels


def active_at(history, coordinate):
    """The open record active at a coordinate's time. The whole trace must be
    ordered first: sorting would hide damaged lifetime evidence, and the order
    of two tables' rows within one ms is unknown."""
    clocks = [(r.get("time_ms"), r.get("packet_id")) for _, r in history]
    if any(t is None or p is None for t, p in clocks):
        return None, "missing_lifecycle_clock"
    if any(b <= a for a, b in zip(clocks, clocks[1:])):
        return None, "lifecycle_clock_duplicate_or_regression"
    if any(t == coordinate["time_ms"] for t, _ in clocks):
        return None, "lifecycle_boundary_same_time"
    active = None
    for ordinal, row in history:
        if row["time_ms"] > coordinate["time_ms"]:
            break
        if row["event"] == "open":
            if active is not None:
                return None, "reopen_without_close"
            active = {**row, "physical_row_ordinal": ordinal}
        elif row["event"] == "close":
            if active is None:
                return None, "close_without_open"
            if any(active[k] != row[k] for k in ("actor_net_guid", "channel_index")):
                return None, "close_identity_mismatch"
            active = None
    if active is None:
        return None, "no_active_actor"
    if any(active[k] != coordinate[k] for k in ("actor_net_guid", "channel_index")):
        return None, "active_identity_mismatch"
    return active, "active"


def active_at_packet(history, coordinate):
    """As active_at, by main packet id; events in the coordinate's own packet
    are unordered. The whole trace is validated, then only its prefix replayed."""
    last = active = reason = None
    for ordinal, row in history:
        packet = row.get("packet_id")
        if packet is None:
            return None, "missing_packet_clock"
        if last is not None and packet <= last:
            return None, "packet_clock_duplicate_or_regression"
        last = packet
        if packet == coordinate["packet_id"]:
            reason = "same_packet_boundary"
        if row["event"] == "open":
            if active is not None:
                return None, "reopen_without_close"
            active = {**row, "physical_row_ordinal": ordinal}
        elif row["event"] == "close":
            if active is None:
                return None, "close_without_open"
            if any(active[k] != row[k] for k in ("actor_net_guid", "channel_index")):
                return None, "close_identity_mismatch"
            active = None
    if reason:
        return None, reason
    active = None
    for ordinal, row in history:
        if row["packet_id"] >= coordinate["packet_id"]:
            break
        active = {**row, "physical_row_ordinal": ordinal} if row["event"] == "open" else None
    if active is None:
        return None, "no_active_actor"
    if any(active[k] != coordinate[k] for k in ("actor_net_guid", "channel_index")):
        return None, "active_identity_mismatch"
    return active, "active"


def _lifetime(actors, channels, ident, active=active_at):
    actor, a_status = active(actors.get(ident["actor_net_guid"], []), ident)
    channel, c_status = active(channels.get(ident["channel_index"], []), ident)
    matched = actor is not None and channel is not None and actor == channel
    return {"status": "active" if matched else "unresolved", "actor_status": a_status,
            "channel_status": c_status, "actor_open": actor, "channel_open": channel}


def _token_comparison(prior, current, previous_route, current_route):
    """Compare identical raw roles only; never assign a shared respawn domain."""
    result = []
    for role in sorted(set(prior) | set(current)):
        left, right = prior.get(role, []), current.get(role, [])
        same_domain = previous_route == current_route
        known = (same_domain and len(left) == len(right) == 1 and all(
            r.get("raw_bits_hex") is not None and r.get("bit_count") is not None for r in left + right))
        equal = ((left[0]["bit_count"], left[0]["raw_bits_hex"]) ==
                 (right[0]["bit_count"], right[0]["raw_bits_hex"])) if known else None
        result.append({"role": role, "previous_route": previous_route, "current_route": current_route,
                       "previous": left, "current": right, "raw_equal": equal,
                       "same_route_domain": same_domain})
    return result


def _arithmetic(prior, section, route):
    result = {"expected_sign": SIGNS.get(route), "predicted_result": None,
              "matches": None, "status": "no_predecessor"}
    if route == RESET:
        result["status"] = "absolute_reset_no_delta_edge"
    elif prior is not None and route in SIGNS:
        value = prior["life_result"] + SIGNS[route] * section["delta_life"]
        try:
            prediction = struct.unpack("<f", struct.pack("<f", value))[0]
        except OverflowError:
            prediction = math.inf
        if math.isfinite(prediction):
            result.update(predicted_result=prediction, matches=prediction == section["life_result"],
                          status="observed_arithmetic_only")
        else:
            result["status"] = "f32_prediction_overflow"
    return result


def _arithmetic_counts(nodes, key):
    out = {f"{key}_{scope}arithmetic_{suffix}": 0
           for scope in ("", "eligible_") for suffix in ("true", "false", "unknown")}
    for node in nodes:
        value = node["route_arithmetic"]["matches"]
        suffix = "true" if value is True else "false" if value is False else "unknown"
        out[f"{key}_arithmetic_{suffix}"] += 1
        if node[key]["eligible"]:
            out[f"{key}_eligible_arithmetic_{suffix}"] += 1
    return out


def build(raw, actor_rows):
    """Strict nodes and barriers from section observations, then each node's
    packet-ordered `packet_view`."""
    actors, channels = _traces(actor_rows)
    observations = sorted(raw["observations"], key=lambda o: min(
        r["physical_row_ordinal"] for r in o["source_rows"]))
    scope_epochs, actor_epochs = collections.Counter(), collections.Counter()
    last_objects, last_clocks, previous = {}, {}, {}
    time_counts = collections.Counter()
    for obs in observations:
        ident = obs["identity"]
        if obs["section_state"]["status"] == "validated_array" and not obs.get("schema_errors"):
            for section in obs["section_state"]["sections"]:
                time_counts[(ident["actor_net_guid"], ident["object_net_guid"],
                             section["changed_component_ref"], ident["time_ms"])] += 1
    nodes, barriers, warnings = [], [], []
    for obs in observations:
        ident, state = obs["identity"], obs["section_state"]
        actor_id, object_id = ident["actor_net_guid"], ident["object_net_guid"]
        scope = actor_id, object_id
        clock = ident["time_ms"], ident["packet_id"]
        object_changed = actor_id in last_objects and last_objects[actor_id] != object_id
        clock_regressed = actor_id in last_clocks and clock < last_clocks[actor_id]
        if object_changed or clock_regressed:
            actor_epochs[actor_id] += 1
        last_objects[actor_id], last_clocks[actor_id] = object_id, clock
        ordinals = [r["physical_row_ordinal"] for r in obs["source_rows"]]
        origin = {"physical_row_ordinals": ordinals, "route": obs["route"], "coordinate": ident}
        if state["status"] != "validated_array" or obs.get("schema_errors"):
            scope_epochs[scope] += 1
            barriers.append({"kind": "schema_invalid" if obs.get("schema_errors") else state["status"],
                "actor_object": list(scope), "epoch_after": scope_epochs[scope], "origin": origin,
                "schema_errors": obs.get("schema_errors", []),
                "raw_lifecycle_tokens": obs.get("raw_lifecycle_tokens", {})})
            continue
        reset = obs["route"] == RESET
        raw_ambiguities = [r for r in obs.get("ambiguity_reasons", [])
                           if r != "scalar_delta_relation_mismatch"]
        if reset or raw_ambiguities:
            scope_epochs[scope] += 1
        relation = state.get("relation")
        if isinstance(relation, dict) and relation.get("matches") is False:
            warnings.append({"kind": "scalar_relation_mismatch", "origin": origin, "relation": relation})
        lifetime = _lifetime(actors, channels, ident)
        for section in state["sections"]:
            key = (*scope, section["changed_component_ref"])
            prior = previous.get(key)
            reasons = []
            tied = time_counts[(*key, ident["time_ms"])] > 1
            if tied:
                reasons.append("same_time_tie")
            if reset:
                reasons.append("reset_state")
            if raw_ambiguities:
                reasons.append("raw_observation_ambiguous")
            if clock_regressed:
                reasons.append("clock_regression")
            if lifetime["status"] != "active":
                reasons.append("lifecycle_unresolved")
            comparisons = []
            if prior is None:
                reasons.append("no_prior_node")
            else:
                if prior["epoch"] != scope_epochs[scope] or prior["actor_epoch"] != actor_epochs[actor_id]:
                    reasons.append("persistent_barrier_epoch")
                if prior["same_time_tie"]:
                    reasons.append("prior_tie_censor")
                if prior["clock_regression"]:
                    reasons.append("prior_clock_regression")
                if prior["raw_observation_ambiguities"]:
                    reasons.append("prior_raw_observation_ambiguous")
                if prior["actor_lifecycle"]["status"] != "active":
                    reasons.append("prior_lifecycle_unresolved")
                elif lifetime["status"] == "active" and prior["actor_lifecycle"] != lifetime:
                    reasons.append("actor_channel_instance_changed")
                p_coord = prior["origin"]["coordinate"]
                if p_coord["channel_index"] != ident["channel_index"]:
                    reasons.append("channel_changed")
                if clock <= (p_coord["time_ms"], p_coord["packet_id"]):
                    reasons.append("nonincreasing_clock")
                if prior["state_key"]["changed_component_path"] != section.get("changed_component_path"):
                    reasons.append("section_path_changed")
                comparisons = _token_comparison(prior["raw_lifecycle_tokens"], obs.get("raw_lifecycle_tokens", {}),
                                                prior["origin"]["route"], obs["route"])
                if any(c["raw_equal"] is False for c in comparisons):
                    reasons.append("opaque_token_changed")
                if any(c["raw_equal"] is None for c in comparisons):
                    reasons.append("opaque_token_gap")
            node = {"node_id": [min(ordinals), section["index"]],
                "state_key": {"actor_net_guid": actor_id, "object_net_guid": object_id,
                    "changed_component_ref": key[2], "changed_component_path": section.get("changed_component_path")},
                "epoch": scope_epochs[scope], "actor_epoch": actor_epochs[actor_id], "origin": origin,
                "life_result": section["life_result"], "delta_life": section["delta_life"],
                "alive_after_change": section["alive_after_change"], "same_time_tie": tied,
                "raw_observation_ambiguities": raw_ambiguities,
                "clock_regression": clock_regressed, "previous_node_id": prior["node_id"] if prior else None,
                "observed_before_after_difference": section["life_result"] - prior["life_result"] if prior else None,
                "continuity": {"eligible": not reasons, "reasons": sorted(set(reasons)),
                    "scope": "ordered_observation_comparison_only", "game_life": "unproved", "component_life": "unproved"},
                "actor_lifecycle": lifetime, "raw_lifecycle_tokens": obs.get("raw_lifecycle_tokens", {}),
                "raw_token_comparisons": comparisons, "scalar_relation": relation,
                "route_arithmetic": _arithmetic(prior, section, obs["route"])}
            nodes.append(node)
            previous[key] = node
    counts = {"nodes": len(nodes), "barriers": len(barriers), "scalar_warnings": len(warnings),
              "continuity_eligible": sum(n["continuity"]["eligible"] for n in nodes),
              "continuity_ineligible": sum(not n["continuity"]["eligible"] for n in nodes)}
    packet_ties = collections.Counter()
    time_groups = collections.defaultdict(list)
    for n in nodes:
        c, k = n["origin"]["coordinate"], n["state_key"]
        state = k["actor_net_guid"], k["object_net_guid"], k["changed_component_ref"]
        packet_ties[(*state, c["packet_id"])] += 1
        time_groups[(*state, c["time_ms"])].append(c["packet_id"])
    by_id = {tuple(n["node_id"]): n for n in nodes}
    for node in nodes:
        c, k = node["origin"]["coordinate"], node["state_key"]
        state = k["actor_net_guid"], k["object_net_guid"], k["changed_component_ref"]
        life = _lifetime(actors, channels, c, active_at_packet)
        prior = by_id.get(tuple(node["previous_node_id"])) if node["previous_node_id"] else None
        strict = node["continuity"]["reasons"]
        reasons = [x for x in strict if x not in REMOVED_STRICT_REASONS]
        tied = packet_ties[(*state, c["packet_id"])] > 1
        if tied:
            reasons.append("same_packet_tie")
        if prior and prior.get("packet_view", {}).get("same_packet_tie"):
            reasons.append("prior_packet_tie_censor")
        if life["status"] != "active":
            reasons.append("packet_lifecycle_unresolved")
        if prior:
            prior_life = prior["packet_view"]["actor_lifecycle"]
            if prior_life["status"] != "active":
                reasons.append("prior_packet_lifecycle_unresolved")
            elif life["status"] == "active" and prior_life != life:
                reasons.append("packet_actor_channel_instance_changed")
            if c["packet_id"] <= prior["origin"]["coordinate"]["packet_id"]:
                reasons.append("nonincreasing_packet")
        reasons = sorted(set(reasons))
        packets = time_groups[(*state, c["time_ms"])]
        node["packet_view"] = {
            "eligible": not reasons, "reasons": reasons,
            "removed_strict_reasons": sorted(x for x in strict if x in REMOVED_STRICT_REASONS),
            "scope": "main_packet_ordered_observation_comparison_only",
            "game_life": "unproved", "component_life": "unproved", "same_packet_tie": tied,
            "actor_lifecycle": life, "actor_channel_lifecycle": life,
            "predecessor_lifecycle": prior["packet_view"]["actor_channel_lifecycle"] if prior else None,
            "same_time_group": {"group_size": len(packets), "same_packet": len(set(packets)) < len(packets),
                                "packets": sorted(packets)},
            "predecessor_same_time_group": prior["packet_view"]["same_time_group"] if prior else None,
            "strict_previous_node_id": node["previous_node_id"],
            "arithmetic_matches": node["route_arithmetic"]["matches"]}
    strict_counts = {"eligible": counts["continuity_eligible"], "ineligible": counts["continuity_ineligible"],
                     **_arithmetic_counts(nodes, "continuity")}
    packet_counts = {"eligible": sum(n["packet_view"]["eligible"] for n in nodes),
                     "ineligible": sum(not n["packet_view"]["eligible"] for n in nodes),
                     "resolved_from_strict_ineligible": sum(
                         n["packet_view"]["eligible"] and not n["continuity"]["eligible"] for n in nodes),
                     **_arithmetic_counts(nodes, "packet_view")}
    return {"nodes": nodes, "barriers": barriers, "scalar_warnings": warnings, "counts": counts,
            "strict_counts_retained": {**counts, **strict_counts}, "packet_counts": packet_counts}


def actor_rows(path):
    """(physical row ordinal, row) for each open/close row of actors.parquet."""
    columns = ["time_ms", "packet_id", "channel_index", "actor_net_guid", "event", "class_path"]
    with closing(iter_selected(path, columns, lambda b: pc.is_in(
            text(b, "event"), value_set=pa.array(["open", "close"])))) as selected:
        for ordinal, row in selected:
            row["_ordinal"] = ordinal
            yield ordinal, row


def extract(export):
    inputs = [export / n for n in INPUT_NAMES]
    before = {p.name: sha(p) for p in inputs}
    raw = extract_section_observations.extract(export)
    timeline = build(raw, list(actor_rows(export / "actors.parquet")))
    after = {p.name: sha(p) for p in inputs}
    if before != after:
        raise ValueError("input changed during extraction")
    return {"schema_version": 2, "kind": "vrfkit_section_packet_timeline",
            "export_id": export.name, "source": str(export.resolve()),
            "provenance": {"input_sha256_before": before, "input_sha256_after": after,
                           "implementation_sha256": {p.name: sha(p) for p in SOURCES},
                           "raw_observation_counts": raw["counts"],
                           "replay_build": raw["provenance"]["replay_build"],
                           "population": "main_only"},
            **timeline}


def main(argv=None):
    return run_json_cli(__doc__, extract, lambda d, out: [
        f"wrote {out} ({d['packet_counts']['eligible']} packet-eligible, "
        f"{d['packet_counts']['resolved_from_strict_ineligible']} resolved)"],
        argv, sources=SOURCES, indent=2, sort_keys=True, allow_nan=False)


if __name__ == "__main__":
    raise SystemExit(main())
