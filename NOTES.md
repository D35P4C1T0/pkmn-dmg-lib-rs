# Port Notes

## Source Of Truth

- Active Champions page: `reference/NCP-VGC-Damage-Calculator/index.html`
- Modern entrypoint: `script_res/damage_SV.js`
- Shared arithmetic/modifiers: `script_res/damage_MASTER.js`
- Champions stats: `script_res/stat_data.js`
- Type chart: `script_res/type_data.js`

## Intentional Scope For This Pass

The crate implements the core Champions damage path for fixture-covered
mechanics. It avoids guessing on data-heavy or UI-dependent branches. When a
mechanic needs unported reference data, the library returns
`CalcError::UnsupportedMechanic`.

## Unsupported Or Partial Mechanics

- Champions-only parity is estimated at roughly 83% against the JS calculator.
  The core damage path is fixture-covered; remaining work is mostly input
  modeling and targeted JS-only metadata.
- counter-style moves (`Counter`, `Mirror Coat`, `Metal Burst`, `Comeuppance`)
  need an explicit Rust input model for the previous/countered move result
- remaining JS-only calculator metadata not covered by `champout`, such as
  move/item/ability flags, aliases, and hardcoded edge-case branches
- dynamic setdex imports
- species-locked item tables and complete Mega/Z/Primal item restrictions
- ability branches present in Champions data but not fully modeled yet, notably
  `Magic Guard`, `Disguise`, `Ripen`, `Sand Spit`, `Plus`, and `Minus`
- remaining Neutralizing Gas / Trace exception parity for every JS
  unsuppressible or uncopyable ability
- remaining team-entry preprocessing from `CALCULATE_ALL_MOVES_SV` for any
  still-unmodeled UI-driven setup branches
- Z-Moves, Max Moves, Legends Z-A cooldown and damage-reduction branches
- UI-only toggles such as manual Ruin aura checkboxes
- normalized high-level Rust data API over generated `champions-data.json`
- optimizer search and spread ranking beyond module placeholders

## Mismatch Policy

If a future fixture mismatch appears, classify it as one of:

- Rust port bug
- intentional correction
- ambiguity in the JavaScript implementation

No intentional corrections are currently recorded.
