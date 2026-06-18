#!/usr/bin/env python3
"""Generate Rust summary constants from vendored Champions JSON."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "data" / "champions" / "generated" / "champions-data.json"
ITEMS = ROOT / "data" / "champions" / "items.json"
ROSTER_M_A = ROOT / "data" / "champions" / "regulation_m_a_pokemon.json"
ROSTER_M_B_ADDITIONS = ROOT / "data" / "champions" / "regulation_m_b_additions.json"
OUT = ROOT / "src" / "data" / "champions.rs"


def rust_str(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def push_string_list(lines: list[str], values: list[str]) -> None:
    for value in values:
        lines.append(f"    {rust_str(value)},")


def regulation_m_b_roster(roster_m_a: list[str]) -> list[str]:
    additions = json.loads(ROSTER_M_B_ADDITIONS.read_text())
    return roster_m_a + additions["regular"] + additions["mega"]


def main() -> None:
    data = json.loads(DATA.read_text())
    items = json.loads(ITEMS.read_text())
    roster_m_a = json.loads(ROSTER_M_A.read_text())
    roster_m_b = regulation_m_b_roster(roster_m_a)

    lines = [
        "//! Pinned Pokemon Champions data lists exposed for downstream tools.",
        "//!",
        "//! These constants are generated from the vendored Champions data in `data/champions`.",
        "//! They intentionally avoid runtime fetching so optimizer crates can treat this crate as",
        "//! local ground truth for legal names and lookup menus.",
        "",
        "/// Lightweight Pokemon/form entry from normalized Champions data.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub struct ChampionSpeciesSummary {",
        "    /// Internal normalized form id from the generated Champions data.",
        "    pub id: &'static str,",
        "    /// Display name, including suffixes where applicable.",
        "    pub display_name: &'static str,",
        "    /// Whether this species/form is usable under the vendored Regulation M-A roster.",
        "    pub is_regulation_m_a: bool,",
        "    /// Whether this species/form is usable under the vendored Regulation M-B roster.",
        "    pub is_regulation_m_b: bool,",
        "}",
        "",
        "/// Lightweight ability entry from normalized Champions data.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub struct ChampionAbilitySummary {",
        "    /// Champions ability id.",
        "    pub id: u16,",
        "    /// English ability name.",
        "    pub name: &'static str,",
        "    /// English ability description from `champout`.",
        "    pub description: &'static str,",
        "}",
        "",
        "/// Raw JSON list of Champions held item names.",
        'pub const CHAMPIONS_ITEMS_JSON: &str = include_str!("../../data/champions/items.json");',
        "",
        "/// Raw JSON list of Regulation M-A legal Pokemon roster names.",
        "pub const REGULATION_M_A_POKEMON_JSON: &str =",
        '    include_str!("../../data/champions/regulation_m_a_pokemon.json");',
        "",
        "/// Raw JSON object with Regulation M-B additions over M-A.",
        "pub const REGULATION_M_B_ADDITIONS_JSON: &str =",
        '    include_str!("../../data/champions/regulation_m_b_additions.json");',
        "",
        "/// Champions held item names currently available in the vendored item list.",
        "pub const CHAMPIONS_ITEMS: &[&str] = &[",
    ]
    push_string_list(lines, items)
    lines.extend(
        [
            "];",
            "",
            "/// Regulation M-A legal Pokemon roster names.",
            "pub const REGULATION_M_A_POKEMON: &[&str] = &[",
        ]
    )
    push_string_list(lines, roster_m_a)
    lines.extend(
        [
            "];",
            "",
            "/// Regulation M-B legal Pokemon roster names.",
            "pub const REGULATION_M_B_POKEMON: &[&str] = &[",
        ]
    )
    push_string_list(lines, roster_m_b)
    lines.extend(
        [
            "];",
            "",
            "/// Pokemon/form summaries from normalized Champions data.",
            "pub const CHAMPIONS_SPECIES: &[ChampionSpeciesSummary] = &[",
        ]
    )
    for entry in data["species"]:
        lines.extend(
            [
                "    ChampionSpeciesSummary {",
                f"        id: {rust_str(entry['id'])},",
                f"        display_name: {rust_str(entry['displayName'])},",
                f"        is_regulation_m_a: {str(entry['isRegulationMA']).lower()},",
                f"        is_regulation_m_b: {str(entry['isRegulationMB']).lower()},",
                "    },",
            ]
        )
    lines.extend(
        [
            "];",
            "",
            "/// Ability summaries from normalized Champions data.",
            "pub const CHAMPIONS_ABILITIES: &[ChampionAbilitySummary] = &[",
        ]
    )
    for entry in data["abilities"]:
        lines.extend(
            [
                "    ChampionAbilitySummary {",
                f"        id: {entry['id']},",
                f"        name: {rust_str(entry['name'])},",
                f"        description: {rust_str(entry['description'])},",
                "    },",
            ]
        )
    lines.extend(
        [
            "];",
            "",
            "/// Find a Champions item name by exact English name.",
            "pub fn champions_item(name: &str) -> Option<&'static str> {",
            "    CHAMPIONS_ITEMS.iter().copied().find(|item| *item == name)",
            "}",
            "",
            "/// Find a Regulation M-A Pokemon roster name by exact English name.",
            "pub fn regulation_m_a_pokemon(name: &str) -> Option<&'static str> {",
            "    REGULATION_M_A_POKEMON",
            "        .iter()",
            "        .copied()",
            "        .find(|pokemon| *pokemon == name)",
            "}",
            "",
            "/// Find a Regulation M-B Pokemon roster name by exact English name.",
            "pub fn regulation_m_b_pokemon(name: &str) -> Option<&'static str> {",
            "    REGULATION_M_B_POKEMON",
            "        .iter()",
            "        .copied()",
            "        .find(|pokemon| *pokemon == name)",
            "}",
            "",
            "/// Find a Champions Pokemon/form summary by exact display name.",
            "pub fn champions_species(display_name: &str) -> Option<ChampionSpeciesSummary> {",
            "    CHAMPIONS_SPECIES",
            "        .iter()",
            "        .copied()",
            "        .find(|species| species.display_name == display_name)",
            "}",
            "",
            "/// Find a Champions ability summary by exact English name.",
            "pub fn champions_ability(name: &str) -> Option<ChampionAbilitySummary> {",
            "    CHAMPIONS_ABILITIES",
            "        .iter()",
            "        .copied()",
            "        .find(|ability| ability.name == name)",
            "}",
            "",
        ]
    )
    OUT.write_text("\n".join(lines))
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
