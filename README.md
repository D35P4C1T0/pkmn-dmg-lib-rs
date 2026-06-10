# pkmn-dmg-lib

Rust library port of the Nimbasa City Post web damage calculator, focused on the
Pokemon Champions ruleset.

The JavaScript implementation in `reference/NCP-VGC-Damage-Calculator` is the
source of truth. The Rust code mirrors the Champions path through
`script_res/damage_SV.js` and shared helpers in `script_res/damage_MASTER.js`,
especially integer floors, Game Freak rounding, and modifier chaining.

## Status

Estimated Champions parity: **about 89%** against the JavaScript calculator's
Champions-relevant behavior.

The core damage path is in good shape: stat calculation, type effectiveness,
rounding, chained modifiers, weather/terrain, common items/abilities,
Terastal/STAB, fixed damage, multi-hit follow-ups, and the Regulation M-A
`champout` data import path are covered by regression tests.

The remaining work is mostly not arithmetic; it is behavior that needs extra
input modeling or more normalized metadata from the JS UI/data layer. See
[Known Gaps](#known-gaps).

## Example

```rust
use damage_calc::{
    calculate_damage, CalcInput, Category, Field, Move, Nature, Pokemon, PokemonType,
    Ruleset, StatTable,
};

let attacker = Pokemon::champions(
    "Attacker",
    [Some(PokemonType::Fire), None],
    StatTable::new(100, 100, 100, 100, 100, 100),
    StatTable::new(0, 0, 0, 0, 0, 0),
    Nature::Hardy,
);
let defender = Pokemon::champions(
    "Defender",
    [Some(PokemonType::Grass), None],
    StatTable::new(100, 100, 100, 100, 100, 100),
    StatTable::new(0, 0, 0, 0, 0, 0),
    Nature::Hardy,
);
let move_ = Move::new("Flamethrower", 90, PokemonType::Fire, Category::Special);

let result = calculate_damage(CalcInput {
    attacker,
    defender,
    move_,
    field: Field::default(),
    ruleset: Ruleset::Champions,
})?;

assert_eq!(result.damage_rolls.len(), 16);
assert_eq!(result.hit_rolls.len(), 1);
# Ok::<(), damage_calc::CalcError>(())
```

## Champions Data API

The crate vendors pinned Champions data so downstream tools do not need to
fetch Pokemon, item, or ability lists.

```rust
use damage_calc::data::champions::{
    champions_ability, champions_item, regulation_m_a_pokemon, CHAMPIONS_ABILITIES,
    CHAMPIONS_ITEMS, CHAMPIONS_SPECIES, REGULATION_M_A_POKEMON,
};

assert!(CHAMPIONS_ITEMS.contains(&"Light Ball"));
assert!(REGULATION_M_A_POKEMON.contains(&"Hydrapple"));
assert_eq!(champions_item("Scope Lens"), Some("Scope Lens"));
assert_eq!(regulation_m_a_pokemon("Venusaur"), Some("Venusaur"));
assert_eq!(champions_ability("Overgrow").unwrap().id, 65);

// Lightweight full lists for menus and optimizer setup.
let item_count = CHAMPIONS_ITEMS.len();
let ability_count = CHAMPIONS_ABILITIES.len();
let species_form_count = CHAMPIONS_SPECIES.len();
```

For richer imports, `damage_calc::data::CHAMPIONS_DATA_JSON` still exposes the
full normalized JSON with species/forms, base stats, legal moves, moves, and
ability descriptions.

## Current Parity Coverage

Implemented and covered by tests:

- Champions stat formula (`CALC_HP_CHAMP`, `CALC_STAT_CHAMP`)
- modern damage base formula and random rolls
- JS `pokeRound` and `chainMods`
- STAB, Adaptability-style STAB, and Champions Terastal STAB
- modern type chart
- type items, gems, Life Orb, Expert Belt, resist berries, Choice items
- Champions raw item list in `data/champions/items.json`, with typed enum
  coverage for every listed name
- vendored Project Pokemon `champout` raw dumps for Pokemon/form data, moves,
  learnsets, and English text, with a source manifest and SHA-256 validation
- normalized `champions-data.json` generated from `champout`, exposed as
  `damage_calc::data::CHAMPIONS_DATA_JSON`
- public zero-fetch list constants in `damage_calc::data::champions` for
  Champions items, Regulation M-A Pokemon, Pokemon/form summaries, and
  abilities
- Light Ball, Shell Bell/Fling power, and Champions Mega-stone Fling/Knock Off
  restrictions
- common offensive and defensive ability modifiers
- type-changing abilities such as Pixilate/Aerilate/Galvanize/Refrigerate
- variable base-power moves such as Low Kick, Gyro Ball, Heavy Slam, Fling,
  Electro Ball, Stored Power, Hex, and related `basePowerFunc` branches
- named move BP/effectiveness/failure branches such as Solar Beam/Solar Blade,
  Nihil Light, Dream Eater, Sky Drop, Synchronoise, Damp, and Sturdy OHKO
- Electromorphosis/Wind Power Electric BP doubling and Wind Rider wind-move
  immunity
- Heatproof Fire reduction, Infiltrator screen bypass, and contact suppression
  from Long Reach, Protective Pads, and Punching Glove
- Natural Gift berry type/power table
- move type changes for Weather Ball, Terrain Pulse, Natural Gift, Judgment,
  Techno Blast, Multi-Attack, Revelation Dance, Aura Wheel, Raging Bull, Ivy
  Cudgel, Tera Blast, and Tera Starstorm
- `CALCULATE_ALL_MOVES_SV` preprocessing for Trace, Neutralizing Gas, Forecast,
  Mimicry, Air Lock/Cloud Nine, Klutz, terrain seeds, Intimidate, Download,
  Embody Aspect, Battle Bond, Intrepid Sword, Dauntless Shield, Wind Rider, and
  Supersweet Syrup
- Protosynthesis/Quark Drive activation from Sun/Electric Terrain or Booster
  Energy, including highest-stat attack/defense/speed modifiers
- Champions entry toggles for Evo/Tatsugiri boosts, Clangorous Soul, and
  Weakness Policy
- fixed and HP-dependent damage moves such as Super Fang, Endeavor, Final
  Gambit, Seismic Toss, and OHKO moves
- counter-style damage moves (`Counter`, `Mirror Coat`, `Metal Burst`, and
  `Comeuppance`) when the caller supplies the countered damage rolls/category
- multi-hit totals and per-hit rolls, including Triple Kick/Triple Axel hit
  power, Parental Bond second-hit reduction, and Stamina/Weak Armor between-hit
  recalculation
- multi-hit first-hit consumables/effects for resist berries, Kee Berry,
  Maranga Berry, Multiscale, Shadow Shield, Gooey, Tangling Hair, Cotton Down,
  Spicy Spray, Sand Spit weather activation, Defiant/Competitive follow-up
  boosts, and burn-heal berries
- Ruin field modifiers
- priority-blocking abilities and Psychic Terrain priority prevention
- Champions type-effectiveness overrides for Thousand Arrows, Stellar vs Tera,
  Tera Shell, Ring Target, Foresight/Scrappy/Mind's Eye
- final speed modifiers for speed-based moves and Analytic ordering
- special stat-source moves such as Foul Play, Body Press, and Psyshock-style
  physical-defense special moves
- Champions ability branches for Plus, Minus, Ripen-enhanced berries, and
  Disguise direct-damage replacement
- JS custom modifier hooks for BP/Attack/Defense/Final modifiers
- Protect quartering only for JS-qualified move/ability paths
- Sun/Rain damage modifiers
- Electric/Grassy/Psychic/Misty terrain power modifiers
- critical hit boost-ignore behavior
- burn, Reflect, Light Screen, Aurora Veil

Fixtures live in `fixtures/js_outputs/champions_cases.json`, and regression
tests live in `tests/fixtures.rs`.

## Known Gaps

This is not yet a full port of every Champions-relevant browser calculator
branch. The biggest remaining gaps are:

- Battle-state effects outside direct damage, such as Magic Guard preventing
  indirect damage/recoil. These need a battle-event model rather than a single
  damage calculation.
- Counter-style moves require callers to provide the previous/countered damage
  rolls and category on `Move`; the library does not infer them from a turn
  history.
- Full Neutralizing Gas / Trace exception parity for every unsuppressible or
  uncopyable ability in the JS lists.
- Remaining browser/UI-only setup toggles that are not yet represented as typed
  `Field` or `Pokemon` inputs.
- Species-locked item legality beyond the Champions Mega-stone/Fling/Knock Off
  cases already covered.
- Z-Move, Max Move, Dynamax, and Legends Z-A cooldown/plus-move branches. These
  exist in the shared JS files but are outside the first-pass Champions library
  scope unless Champions formats require them.
- High-level constructors from normalized Champions data into calculator
  `Pokemon` and `Move` structs. The pinned lists and full JSON are exposed, but
  callers still assemble battle-ready typed inputs directly.
- Optimizer search and spread ranking beyond module placeholders.

Local JS reference checkouts may live under `reference/` for behavior audits,
but that folder is ignored and intentionally not published in this repository.
The library should prefer `champout` plus targeted JS-only metadata rather than
a full raw import of `pokedex.js`, `move_data.js`, `item_data.js`, and
`ability_data.js`.

## Thanks To

- Nimbasa City Post / `NCP-VGC-Damage-Calculator`, whose JavaScript calculator
  is the behavior reference for this Rust port.
- Project Pokemon / `champout`, for the Pokemon Champions static data dumps
  used to generate and validate the vendored Champions data.
- Pokemon Showdown and the `@pkmn` ecosystem, for normalized Pokemon data and
  mechanics references used to cross-check ambiguous cases.

## Repository

GitHub remote:
<https://github.com/D35P4C1T0/pkmn-dmg-lib-rs>

The crate is prepared with package metadata, an MIT license, pinned Champions
data, and a `.gitignore` that excludes build output.

## Development

```sh
cargo test
```

The crate has optional serde derives behind the `serde` feature:

```sh
cargo test --features serde
```

Before publishing or tagging a release, run:

```sh
cargo fmt --check
cargo test
cargo test --features serde
cargo package --list
```
