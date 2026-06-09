use crate::data::items::{
    berry_resist_type, can_fling, drive_type, fling_power, gem_type, is_gem, item_boost_type,
    locked_item_for_species, memory_type, natural_gift,
};
use crate::data::type_chart::move_effectiveness;
use crate::mechanics::modifiers::{
    apply_mod, chain_mods, modified_stat, poke_round_ratio, MODIFIER_DENOMINATOR,
    MOD_0_67_DOUBLES_SCREEN, MOD_1_1, MOD_1_1_ALT, MOD_1_2, MOD_1_25, MOD_1_3, MOD_1_33, MOD_1_5,
    MOD_DOUBLE, MOD_HALF, MOD_LIFE_ORB, MOD_THREE_QUARTERS,
};
use crate::stats::calculate_stats;
use crate::types::{
    Ability, CalcError, Category, Field, Format, Item, Move, Pokemon, PokemonType, Ruleset, Stat,
    StatusCondition, Weather,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CalcInput {
    pub attacker: Pokemon,
    pub defender: Pokemon,
    pub move_: Move,
    pub field: Field,
    pub ruleset: Ruleset,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModifierBreakdown {
    pub label: String,
    pub modifier: i32,
}

impl ModifierBreakdown {
    fn new(label: impl Into<String>, modifier: i32) -> Self {
        Self {
            label: label.into(),
            modifier,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DamageResult {
    pub min_damage: u16,
    pub max_damage: u16,
    pub damage_rolls: Vec<u16>,
    pub hit_rolls: Vec<Vec<u16>>,
    pub percent_range: (f32, f32),
    pub ko_chance: Option<f32>,
    pub applied_modifiers: Vec<ModifierBreakdown>,
    pub debug: Vec<String>,
}

pub fn calculate_damage(input: CalcInput) -> Result<DamageResult, CalcError> {
    match input.ruleset {
        Ruleset::Champions => calculate_champions_damage(input),
    }
}

fn calculate_champions_damage(mut input: CalcInput) -> Result<DamageResult, CalcError> {
    let mut entry_modifiers = Vec::new();
    preprocess_battle_state(
        &mut input.attacker,
        &mut input.defender,
        &mut input.field,
        &mut entry_modifiers,
    )?;

    let requested_hits = input.move_.hits.max(1);
    let parental_bond_hits = input.attacker.ability == Ability::ParentalBond
        && requested_hits == 1
        && (input.field.format == Format::Singles || !input.move_.is_spread);
    let hit_count = if parental_bond_hits {
        2
    } else {
        requested_hits
    };

    if hit_count == 1 {
        return calculate_champions_single_hit(input, entry_modifiers);
    }

    let defender_max_hp = input
        .defender
        .max_hp_override
        .unwrap_or(calculate_stats(&input.defender, Ruleset::Champions)?.hp);
    let defender_current_hp = input.defender.current_hp.unwrap_or(defender_max_hp);
    let mut hit_rolls = Vec::with_capacity(hit_count as usize);
    let mut applied_modifiers = entry_modifiers;
    let mut debug = vec![format!("hits={hit_count}")];
    let mut attacker = input.attacker;
    let mut defender = input.defender;
    let mut field = input.field;

    for hit_index in 0..hit_count {
        let mut hit_move = input.move_.clone();
        if matches!(hit_move.name.as_str(), "Triple Kick" | "Triple Axel") {
            hit_move.current_triple_hit = Some(hit_index + 1);
        }
        let mut hit_attacker = attacker.clone();
        if parental_bond_hits && hit_index == 1 {
            hit_attacker.custom_final_mods.push(MOD_HALF);
        }
        let hit_result = calculate_champions_single_hit(
            CalcInput {
                attacker: hit_attacker,
                defender: defender.clone(),
                move_: hit_move,
                field,
                ruleset: input.ruleset,
            },
            Vec::new(),
        )?;
        applied_modifiers.extend(hit_result.applied_modifiers.clone());
        debug.extend(hit_result.debug);
        hit_rolls.push(hit_result.damage_rolls);

        if hit_index + 1 < hit_count {
            apply_between_hit_effects(
                &mut attacker,
                &mut defender,
                &input.move_,
                &mut field,
                &hit_result.applied_modifiers,
            );
        }
    }

    let damage_rolls = combine_hit_rolls(&hit_rolls);
    let min_damage = *damage_rolls.first().unwrap_or(&0);
    let max_damage = *damage_rolls.last().unwrap_or(&0);
    let percent_range = (
        min_damage as f32 * 100.0 / defender_max_hp as f32,
        max_damage as f32 * 100.0 / defender_max_hp as f32,
    );
    let ko_rolls = damage_rolls
        .iter()
        .filter(|&&damage| damage >= defender_current_hp)
        .count();
    let ko_chance = Some(ko_rolls as f32 / damage_rolls.len() as f32);

    Ok(DamageResult {
        min_damage,
        max_damage,
        damage_rolls,
        hit_rolls,
        percent_range,
        ko_chance,
        applied_modifiers,
        debug,
    })
}

fn calculate_champions_single_hit(
    input: CalcInput,
    initial_modifiers: Vec<ModifierBreakdown>,
) -> Result<DamageResult, CalcError> {
    let mut attacker = input.attacker;
    let mut defender = input.defender;
    let mut move_ = input.move_;
    let field = input.field;
    let mut modifiers = initial_modifiers;
    let mut debug = Vec::new();
    let original_attacker_types = attacker.types;

    let attacker_stats = calculate_stats(&attacker, Ruleset::Champions)?;
    let defender_stats = calculate_stats(&defender, Ruleset::Champions)?;
    let defender_max_hp = defender.max_hp_override.unwrap_or(defender_stats.hp);
    let defender_current_hp = defender.current_hp.unwrap_or(defender_max_hp);
    let attacker_max_hp = attacker.max_hp_override.unwrap_or(attacker_stats.hp);
    let attacker_current_hp = attacker.current_hp.unwrap_or(attacker_max_hp);

    if attacker.is_terastalized {
        if let Some(tera_type) = attacker.tera_type {
            if tera_type != PokemonType::Stellar {
                attacker.types = [Some(tera_type), None];
            }
        }
    }
    if defender.is_terastalized {
        if let Some(tera_type) = defender.tera_type {
            if tera_type != PokemonType::Stellar {
                defender.types = [Some(tera_type), None];
            }
        }
    }

    apply_move_type_changes(&mut move_, &attacker, &field);
    let ate_ize_boosted = apply_ability_type_change(&mut move_, &attacker, &mut modifiers);

    if move_.base_power == 0 || move_.category == Category::Status {
        return Ok(DamageResult {
            min_damage: 0,
            max_damage: 0,
            damage_rolls: vec![0],
            hit_rolls: vec![vec![0]],
            percent_range: (0.0, 0.0),
            ko_chance: Some(0.0),
            applied_modifiers: modifiers,
            debug,
        });
    }

    let attack_modified_stats = ModifiedStats::from(&attacker, attacker_stats)?;
    let defense_modified_stats = ModifiedStats::from(&defender, defender_stats)?;
    let attacker_highest_stat = highest_stat(&attacker, attack_modified_stats);
    let defender_highest_stat = highest_stat(&defender, defense_modified_stats);
    let attacker_speed = final_speed(
        &attacker,
        attack_modified_stats.speed,
        field.attacker_tailwind,
        field.attacker_swamp,
        attacker_highest_stat,
    );
    let defender_speed = final_speed(
        &defender,
        defense_modified_stats.speed,
        field.defender_tailwind,
        field.defender_swamp,
        defender_highest_stat,
    );
    let attacker_moves_first = attacker_speed > defender_speed;

    let def_ability =
        ability_after_ignore(attacker.ability, defender.ability, &move_, &mut modifiers);
    let mut type_effectiveness = move_effectiveness(
        &move_.name,
        move_.type_,
        defender.types,
        field.foresight,
        matches!(attacker.ability, Ability::Scrappy | Ability::MindEye),
        field.gravity,
        defender.item == Item::IronBall,
        defender.item == Item::RingTarget,
        field.weather == Weather::StrongWinds,
    );
    if move_.type_ == PokemonType::Ground
        && move_.name == "Thousand Arrows"
        && defender.has_type(PokemonType::Flying)
        && !field.gravity
    {
        type_effectiveness = 1.0;
        modifiers.push(ModifierBreakdown::new("Thousand Arrows type override", 0));
    } else if move_.type_ == PokemonType::Stellar && defender.is_terastalized {
        type_effectiveness = 2.0;
        modifiers.push(ModifierBreakdown::new("Stellar vs Tera", 0));
    } else if def_ability == Ability::TeraShell
        && defender_current_hp == defender_max_hp
        && type_effectiveness > 0.5
    {
        type_effectiveness = 0.5;
        modifiers.push(ModifierBreakdown::new("Tera Shell type override", 0));
    }

    if is_immune(
        &move_,
        &attacker,
        &defender,
        def_ability,
        &field,
        type_effectiveness,
        &mut modifiers,
    ) {
        return Ok(zero_damage(defender_max_hp, modifiers, debug));
    }

    if def_ability == Ability::Disguise && defender.ability_on {
        modifiers.push(ModifierBreakdown::new("Disguise", 0));
        return Ok(single_damage_result(
            (defender_max_hp / 8).max(1),
            defender_max_hp,
            defender_current_hp,
            modifiers,
            debug,
        ));
    }

    if let Some(result) = set_damage_result(
        &move_,
        &attacker,
        &defender,
        attacker_current_hp,
        defender_current_hp,
        defender_max_hp,
        field.protect,
        modifiers.clone(),
        debug.clone(),
    ) {
        return Ok(result);
    }

    let is_critical = move_.is_critical;
    let base_power = calc_base_power(
        &move_,
        &attacker,
        &defender,
        attacker_current_hp,
        attacker_max_hp,
        defender_current_hp,
        defender_max_hp,
        &field,
        attacker_speed,
        defender_speed,
        &mut modifiers,
    )?;
    if base_power == 0 {
        return Ok(zero_damage(defender_max_hp, modifiers, debug));
    }
    let bp_mods = calc_bp_mods(
        &move_,
        &attacker,
        &defender,
        &field,
        base_power,
        ate_ize_boosted,
        attacker_moves_first,
        defender_current_hp,
        defender_max_hp,
        &mut modifiers,
    );
    let base_power = apply_mod(base_power as i32, chain_mods(&bp_mods)).max(1);

    let hits_physical = move_.category == Category::Physical || move_.deals_physical_damage;
    let attack_stat = if move_.name == "Body Press" {
        Stat::Defense
    } else if move_.category == Category::Physical {
        Stat::Attack
    } else {
        Stat::SpecialAttack
    };
    let defense_stat = if hits_physical {
        Stat::Defense
    } else {
        Stat::SpecialDefense
    };

    let (attack_source_raw, attack_source_mod, attack_source_boosts) = if move_.name == "Foul Play"
    {
        (defender_stats, defense_modified_stats, defender.boosts)
    } else {
        (attacker_stats, attack_modified_stats, attacker.boosts)
    };

    let mut attack =
        if def_ability == Ability::Unaware && attack_source_boosts.get(attack_stat) != 0 {
            modifiers.push(ModifierBreakdown::new("Unaware ignores attack boost", 0));
            attack_source_raw.get(attack_stat)
        } else if is_critical && attack_source_boosts.get(attack_stat) < 0 {
            attack_source_raw.get(attack_stat)
        } else {
            attack_source_mod.get(attack_stat)
        } as i32;

    if attacker.ability == Ability::Hustle && move_.category == Category::Physical {
        attack = poke_round_ratio(attack as i64 * 3, 2);
        modifiers.push(ModifierBreakdown::new("Hustle direct attack", MOD_1_5));
    }

    let at_mods = calc_attack_mods(
        &move_,
        &attacker,
        attacker_highest_stat,
        def_ability,
        &field,
        &mut modifiers,
    );
    attack = apply_mod(attack, chain_mods(&at_mods)).max(1);

    let mut defense =
        if attacker.ability == Ability::Unaware && defender.boosts.get(defense_stat) != 0 {
            modifiers.push(ModifierBreakdown::new("Unaware ignores defense boost", 0));
            defender_stats.get(defense_stat)
        } else if move_.ignores_defense_boosts && defender.boosts.get(defense_stat) != 0 {
            modifiers.push(ModifierBreakdown::new("move ignores defense boosts", 0));
            defender_stats.get(defense_stat)
        } else if is_critical && defender.boosts.get(defense_stat) > 0 {
            defender_stats.get(defense_stat)
        } else {
            defense_modified_stats.get(defense_stat)
        } as i32;

    if ((field.weather == Weather::Sand && defender.has_type(PokemonType::Rock) && !hits_physical)
        || (field.weather == Weather::Snow && defender.has_type(PokemonType::Ice) && hits_physical))
        && attacker.ability != Ability::MegaSol
    {
        defense = poke_round_ratio(defense as i64 * 3, 2);
        modifiers.push(ModifierBreakdown::new("weather defense", MOD_1_5));
    }

    let df_mods = calc_defense_mods(
        &move_,
        &defender,
        defender_highest_stat,
        def_ability,
        &field,
        hits_physical,
        &mut modifiers,
    );
    defense = apply_mod(defense, chain_mods(&df_mods)).max(1);

    let mut base_damage =
        ((((2 * attacker.level as i32) / 5 + 2) * base_power * attack) / defense) / 50 + 2;
    debug.push(format!(
        "base_damage={base_damage}, bp={base_power}, attack={attack}, defense={defense}"
    ));

    if field.format != Format::Singles && move_.is_spread {
        base_damage = apply_mod(base_damage, MOD_THREE_QUARTERS);
        modifiers.push(ModifierBreakdown::new("spread", MOD_THREE_QUARTERS));
    }

    if weather_damage_boost(
        &move_,
        attacker.ability,
        field.weather,
        attacker.item,
        defender.item,
    ) {
        base_damage = apply_mod(base_damage, MOD_1_5);
        modifiers.push(ModifierBreakdown::new("weather damage boost", MOD_1_5));
    } else if weather_damage_drop(&move_, attacker.ability, field.weather, defender.item) {
        base_damage = apply_mod(base_damage, MOD_HALF);
        modifiers.push(ModifierBreakdown::new("weather damage drop", MOD_HALF));
    }

    if field.glaive_rush {
        base_damage = apply_mod(base_damage, MOD_DOUBLE);
        modifiers.push(ModifierBreakdown::new("Glaive Rush", MOD_DOUBLE));
    }

    if is_critical {
        base_damage = base_damage * 3 / 2;
        modifiers.push(ModifierBreakdown::new("critical", MOD_1_5));
    }

    let stab_mod = stab_modifier(&attacker, original_attacker_types, &move_, &mut modifiers);
    let apply_burn = attacker.status == StatusCondition::Burned
        && move_.category == Category::Physical
        && attacker.ability != Ability::Guts
        && !move_.ignores_burn;
    if apply_burn {
        modifiers.push(ModifierBreakdown::new("burn", MOD_HALF));
    }

    let final_mods = calc_final_mods(
        &move_,
        &attacker,
        &defender,
        def_ability,
        &field,
        is_critical,
        type_effectiveness,
        defender_current_hp,
        defender_max_hp,
        &mut modifiers,
    );
    let final_mod = chain_mods(&final_mods);
    let quartered_by_protect = field.protect
        && (move_.is_z
            || move_.is_max
            || move_.breaks_protect
            || matches!(
                attacker.ability,
                Ability::PiercingDrill | Ability::UnseenFist
            ));
    if quartered_by_protect {
        modifiers.push(ModifierBreakdown::new("Protect", 0x0400));
    }

    let mut rolls = Vec::with_capacity(16);
    for random_factor in 85..=100 {
        let mut damage = base_damage * random_factor / 100;
        damage = apply_mod(damage, stab_mod);
        damage = (damage as f32 * type_effectiveness).floor() as i32;
        if apply_burn {
            damage /= 2;
        }
        damage = apply_mod(damage, final_mod);
        if quartered_by_protect {
            damage = apply_mod(damage, 0x0400);
        }
        damage = damage.max(1);
        if damage > 65535 {
            damage %= 65536;
        }
        rolls.push(damage as u16);
    }
    rolls.sort_unstable();

    let min_damage = *rolls.first().unwrap_or(&0);
    let max_damage = *rolls.last().unwrap_or(&0);
    let percent_range = (
        min_damage as f32 * 100.0 / defender_max_hp as f32,
        max_damage as f32 * 100.0 / defender_max_hp as f32,
    );
    let ko_rolls = rolls
        .iter()
        .filter(|&&damage| damage >= defender_current_hp)
        .count();
    let ko_chance = Some(ko_rolls as f32 / rolls.len() as f32);

    Ok(DamageResult {
        min_damage,
        max_damage,
        damage_rolls: rolls.clone(),
        hit_rolls: vec![rolls],
        percent_range,
        ko_chance,
        applied_modifiers: modifiers,
        debug,
    })
}

fn combine_hit_rolls(hit_rolls: &[Vec<u16>]) -> Vec<u16> {
    let mut totals = vec![0u16];
    for rolls in hit_rolls {
        let mut next = Vec::with_capacity(totals.len() * rolls.len());
        for total in &totals {
            for roll in rolls {
                next.push(total.saturating_add(*roll));
            }
        }
        totals = next;
    }
    totals.sort_unstable();
    totals
}

fn apply_between_hit_effects(
    attacker: &mut Pokemon,
    defender: &mut Pokemon,
    move_: &Move,
    field: &mut Field,
    hit_modifiers: &[ModifierBreakdown],
) {
    if hit_modifiers
        .iter()
        .any(|modifier| modifier.label == "resist berry")
    {
        defender.item = Item::None;
    }
    if hit_modifiers
        .iter()
        .any(|modifier| modifier.label == "Multiscale")
    {
        defender.ability = Ability::None;
    }
    if defender.item == Item::KeeBerry
        && physical_hit_for_between_hit_effects(move_)
        && defender.boosts.defense < 6
    {
        defender.boosts.defense += 1;
        defender.item = Item::None;
    } else if defender.item == Item::MarangaBerry
        && !physical_hit_for_between_hit_effects(move_)
        && defender.boosts.special_defense < 6
    {
        defender.boosts.special_defense += 1;
        defender.item = Item::None;
    }
    if defender.ability == Ability::Stamina && physical_hit_for_between_hit_effects(move_) {
        defender.boosts.defense = (defender.boosts.defense + 1).min(6);
    }
    if defender.ability == Ability::WeakArmor && physical_hit_for_between_hit_effects(move_) {
        defender.boosts.defense = (defender.boosts.defense - 1).max(-6);
        defender.boosts.speed = (defender.boosts.speed + 2).min(6);
    }
    if gooey_like_between_hit_effect_applies(attacker, defender, move_) {
        attacker.boosts.speed = (attacker.boosts.speed - 1).max(-6);
        if attacker.ability == Ability::Defiant {
            attacker.boosts.attack = (attacker.boosts.attack + 2).min(6);
        } else if attacker.ability == Ability::Competitive {
            attacker.boosts.special_attack = (attacker.boosts.special_attack + 2).min(6);
        }
    }
    if spicy_spray_between_hit_effect_applies(attacker, defender, move_, field) {
        if matches!(attacker.item, Item::RawstBerry | Item::LumBerry) {
            attacker.item = Item::None;
        } else {
            attacker.status = StatusCondition::Burned;
        }
    }
    if defender.ability == Ability::SandSpit && field.weather != Weather::Sand {
        field.weather = Weather::Sand;
    }
}

fn physical_hit_for_between_hit_effects(move_: &Move) -> bool {
    move_.category == Category::Physical || move_.deals_physical_damage
}

fn gooey_like_between_hit_effect_applies(
    attacker: &Pokemon,
    defender: &Pokemon,
    move_: &Move,
) -> bool {
    if matches!(defender.ability, Ability::Gooey | Ability::TanglingHair)
        && makes_effective_contact(attacker, move_)
    {
        return true;
    }
    defender.ability == Ability::CottonDown
        && defender.boosts.speed > -6
        && (matches!(attacker.ability, Ability::Defiant | Ability::Competitive)
            || matches!(move_.name.as_str(), "Electro Ball" | "Gyro Ball"))
}

fn spicy_spray_between_hit_effect_applies(
    attacker: &Pokemon,
    defender: &Pokemon,
    move_: &Move,
    field: &Field,
) -> bool {
    defender.ability == Ability::SpicySpray
        && (attacker.ability == Ability::FlareBoost || move_.category == Category::Physical)
        && can_be_burned(attacker, move_, field)
}

fn can_be_burned(attacker: &Pokemon, move_: &Move, field: &Field) -> bool {
    attacker.status != StatusCondition::Burned
        && !attacker.has_type(PokemonType::Fire)
        && !(matches!(attacker.ability, Ability::Protean | Ability::Libero)
            && attacker.ability_on
            && move_.type_ == PokemonType::Fire)
        && !(attacker.ability == Ability::LeafGuard && field.weather.is_sun())
        && !matches!(
            attacker.ability,
            Ability::WaterVeil
                | Ability::WaterBubble
                | Ability::Comatose
                | Ability::ThermalExchange
                | Ability::PurifyingSalt
        )
        && (field.terrain != crate::types::Terrain::Misty || !is_grounded(attacker, field))
}

fn makes_effective_contact(attacker: &Pokemon, move_: &Move) -> bool {
    if move_.makes_contact
        && (attacker.item == Item::ProtectivePads
            || (attacker.item == Item::PunchingGlove && move_.is_punch)
            || attacker.ability == Ability::LongReach)
    {
        return false;
    }
    if move_.name == "Shell Side Arm" && move_.category == Category::Physical {
        return true;
    }
    move_.makes_contact
}

#[derive(Debug, Clone, Copy)]
struct ModifiedStats {
    attack: u16,
    defense: u16,
    special_attack: u16,
    special_defense: u16,
    speed: u16,
}

impl ModifiedStats {
    fn from(pokemon: &Pokemon, raw: crate::types::StatTable) -> Result<Self, CalcError> {
        Ok(Self {
            attack: modified_stat(raw.attack, pokemon.boosts.attack)?,
            defense: modified_stat(raw.defense, pokemon.boosts.defense)?,
            special_attack: modified_stat(raw.special_attack, pokemon.boosts.special_attack)?,
            special_defense: modified_stat(raw.special_defense, pokemon.boosts.special_defense)?,
            speed: modified_stat(raw.speed, pokemon.boosts.speed)?,
        })
    }

    fn get(self, stat: Stat) -> u16 {
        match stat {
            Stat::Attack => self.attack,
            Stat::Defense => self.defense,
            Stat::SpecialAttack => self.special_attack,
            Stat::SpecialDefense => self.special_defense,
            Stat::Speed => self.speed,
            Stat::Hp => unreachable!("HP is never stage-modified"),
        }
    }
}

fn highest_stat(pokemon: &Pokemon, stats: ModifiedStats) -> Stat {
    if let Some(stat) = pokemon.highest_stat_override {
        return stat;
    }
    let ordered = [
        (Stat::Attack, stats.attack),
        (Stat::Defense, stats.defense),
        (Stat::SpecialAttack, stats.special_attack),
        (Stat::SpecialDefense, stats.special_defense),
        (Stat::Speed, stats.speed),
    ];
    let mut best = ordered[0];
    for candidate in ordered.into_iter().skip(1) {
        if candidate.1 > best.1 {
            best = candidate;
        }
    }
    best.0
}

fn preprocess_battle_state(
    attacker: &mut Pokemon,
    defender: &mut Pokemon,
    field: &mut Field,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Result<(), CalcError> {
    check_trace(attacker, defender, modifiers);
    check_trace(defender, attacker, modifiers);
    check_neutralizing_gas(attacker, defender, field, modifiers);

    preprocess_pokemon(attacker, field.weather, field.terrain);
    preprocess_pokemon(defender, field.weather, field.terrain);
    if matches!(attacker.ability, Ability::AirLock | Ability::CloudNine)
        || matches!(defender.ability, Ability::AirLock | Ability::CloudNine)
    {
        field.weather = Weather::None;
        modifiers.push(ModifierBreakdown::new("weather suppressed", 0));
    }

    check_klutz(attacker, modifiers);
    check_klutz(defender, modifiers);
    check_entry_boost_toggles(attacker, defender, field, modifiers);
    check_paradox_abilities(attacker, field.terrain, field.weather, modifiers);
    check_paradox_abilities(defender, field.terrain, field.weather, modifiers);
    check_seeds(attacker, field.terrain, modifiers);
    check_seeds(defender, field.terrain, modifiers);
    check_sword_shield(attacker, modifiers);
    check_sword_shield(defender, modifiers);
    check_wind_rider(attacker, field.attacker_tailwind, modifiers);
    check_wind_rider(defender, field.defender_tailwind, modifiers);
    check_intimidate(attacker, defender, modifiers);
    check_intimidate(defender, attacker, modifiers);
    check_supersweet_syrup(attacker, defender, modifiers);
    check_supersweet_syrup(defender, attacker, modifiers);
    check_download(attacker, defender, modifiers)?;
    check_download(defender, attacker, modifiers)?;
    check_embody_aspect(attacker, modifiers);
    check_embody_aspect(defender, modifiers);
    check_battle_bond(attacker, modifiers);
    check_battle_bond(defender, modifiers);

    Ok(())
}

fn check_entry_boost_toggles(
    attacker: &mut Pokemon,
    defender: &mut Pokemon,
    field: &Field,
    modifiers: &mut Vec<ModifierBreakdown>,
) {
    if field.attacker_evo_boost {
        apply_all_non_hp_boost(attacker, 2, 6);
        modifiers.push(ModifierBreakdown::new("attacker Evo/Tatsugiri boost", 0));
    }
    if field.defender_evo_boost {
        apply_all_non_hp_boost(defender, 2, 6);
        modifiers.push(ModifierBreakdown::new("defender Evo/Tatsugiri boost", 0));
    }
    if field.attacker_clangorous_soul {
        apply_special_clangorous_boost(attacker);
        modifiers.push(ModifierBreakdown::new("attacker Clangorous Soul", 0));
    }
    if field.defender_clangorous_soul {
        apply_special_clangorous_boost(defender);
        modifiers.push(ModifierBreakdown::new("defender Clangorous Soul", 0));
    }
    if field.attacker_weakness_policy {
        apply_weakness_policy_boost(attacker);
        modifiers.push(ModifierBreakdown::new("attacker Weakness Policy", 0));
    }
    if field.defender_weakness_policy {
        apply_weakness_policy_boost(defender);
        modifiers.push(ModifierBreakdown::new("defender Weakness Policy", 0));
    }
}

fn apply_all_non_hp_boost(pokemon: &mut Pokemon, stages: i8, cap: i8) {
    pokemon.boosts.attack = (pokemon.boosts.attack + stages).min(cap);
    pokemon.boosts.defense = (pokemon.boosts.defense + stages).min(cap);
    pokemon.boosts.special_attack = (pokemon.boosts.special_attack + stages).min(cap);
    pokemon.boosts.special_defense = (pokemon.boosts.special_defense + stages).min(cap);
    pokemon.boosts.speed = (pokemon.boosts.speed + stages).min(cap);
}

fn apply_special_clangorous_boost(pokemon: &mut Pokemon) {
    pokemon.boosts.special_attack = (pokemon.boosts.special_attack + 2).min(6);
    pokemon.boosts.special_defense = (pokemon.boosts.special_defense + 2).min(6);
    pokemon.boosts.speed = (pokemon.boosts.speed + 2).min(6);
}

fn apply_weakness_policy_boost(pokemon: &mut Pokemon) {
    pokemon.boosts.attack = (pokemon.boosts.attack + 2).min(6);
    pokemon.boosts.special_attack = (pokemon.boosts.special_attack + 2).min(6);
}

fn check_trace(source: &mut Pokemon, target: &Pokemon, modifiers: &mut Vec<ModifierBreakdown>) {
    if source.ability != Ability::Trace
        || !source.ability_on
        || source.item == Item::AbilityShield
        || cannot_trace(target.ability)
    {
        return;
    }
    source.ability = target.ability;
    modifiers.push(ModifierBreakdown::new("Trace", 0));
}

fn cannot_trace(ability: Ability) -> bool {
    matches!(
        ability,
        Ability::AsOne
            | Ability::BattleBond
            | Ability::EmbodyAspect
            | Ability::FlowerGift
            | Ability::Forecast
            | Ability::Mimicry
            | Ability::NeutralizingGas
            | Ability::Protosynthesis
            | Ability::QuarkDrive
            | Ability::Trace
            | Ability::WonderGuard
    )
}

fn check_neutralizing_gas(
    p1: &mut Pokemon,
    p2: &mut Pokemon,
    field: &Field,
    modifiers: &mut Vec<ModifierBreakdown>,
) {
    if !(field.neutralizing_gas
        || p1.ability == Ability::NeutralizingGas
        || p2.ability == Ability::NeutralizingGas)
    {
        return;
    }
    if !cannot_suppress_ability(p1.ability) && p1.item != Item::AbilityShield {
        p1.ability = Ability::None;
    }
    if !cannot_suppress_ability(p2.ability) && p2.item != Item::AbilityShield {
        p2.ability = Ability::None;
    }
    modifiers.push(ModifierBreakdown::new("Neutralizing Gas", 0));
}

fn cannot_suppress_ability(ability: Ability) -> bool {
    matches!(ability, Ability::AsOne | Ability::BattleBond)
}

fn check_klutz(pokemon: &mut Pokemon, modifiers: &mut Vec<ModifierBreakdown>) {
    if pokemon.ability == Ability::Klutz && pokemon.item != Item::None {
        pokemon.item = Item::KlutzSuppressed;
        modifiers.push(ModifierBreakdown::new("Klutz", 0));
    }
}

fn check_paradox_abilities(
    pokemon: &mut Pokemon,
    terrain: crate::types::Terrain,
    weather: Weather,
    modifiers: &mut Vec<ModifierBreakdown>,
) {
    if !matches!(
        pokemon.ability,
        Ability::Protosynthesis | Ability::QuarkDrive
    ) {
        return;
    }
    let field_active = (pokemon.ability == Ability::Protosynthesis && weather == Weather::Sun)
        || (pokemon.ability == Ability::QuarkDrive && terrain == crate::types::Terrain::Electric);
    if field_active {
        pokemon.paradox_ability_boost = true;
        modifiers.push(ModifierBreakdown::new("Paradox ability", 0));
    } else if pokemon.item == Item::BoosterEnergy {
        pokemon.paradox_ability_boost = true;
        pokemon.item = Item::None;
        modifiers.push(ModifierBreakdown::new("Booster Energy", 0));
    }
}

fn check_seeds(
    pokemon: &mut Pokemon,
    terrain: crate::types::Terrain,
    modifiers: &mut Vec<ModifierBreakdown>,
) {
    let triggered = matches!(
        (pokemon.item, terrain),
        (Item::ElectricSeed, crate::types::Terrain::Electric)
            | (Item::GrassySeed, crate::types::Terrain::Grassy)
            | (Item::MistySeed, crate::types::Terrain::Misty)
            | (Item::PsychicSeed, crate::types::Terrain::Psychic)
    );
    if !triggered {
        return;
    }
    if matches!(
        terrain,
        crate::types::Terrain::Electric | crate::types::Terrain::Grassy
    ) {
        boost_stat(pokemon, Stat::Defense, 1);
    } else {
        boost_stat(pokemon, Stat::SpecialDefense, 1);
    }
    pokemon.item = Item::None;
    modifiers.push(ModifierBreakdown::new("terrain seed", 0));
}

fn check_sword_shield(pokemon: &mut Pokemon, modifiers: &mut Vec<ModifierBreakdown>) {
    if pokemon.ability == Ability::IntrepidSword && pokemon.ability_on {
        boost_stat(pokemon, Stat::Attack, 1);
        modifiers.push(ModifierBreakdown::new("Intrepid Sword", 0));
    } else if pokemon.ability == Ability::DauntlessShield && pokemon.ability_on {
        boost_stat(pokemon, Stat::Defense, 1);
        modifiers.push(ModifierBreakdown::new("Dauntless Shield", 0));
    }
}

fn check_wind_rider(pokemon: &mut Pokemon, tailwind: bool, modifiers: &mut Vec<ModifierBreakdown>) {
    if pokemon.ability == Ability::WindRider && tailwind {
        boost_stat(pokemon, Stat::Attack, 1);
        modifiers.push(ModifierBreakdown::new("Wind Rider", 0));
    }
}

fn check_intimidate(
    source: &mut Pokemon,
    target: &mut Pokemon,
    modifiers: &mut Vec<ModifierBreakdown>,
) {
    if source.ability != Ability::Intimidate || !source.ability_on {
        return;
    }
    if matches!(target.ability, Ability::Contrary | Ability::GuardDog) {
        boost_stat(target, Stat::Attack, 1);
    } else if matches!(
        target.ability,
        Ability::ClearBody
            | Ability::WhiteSmoke
            | Ability::HyperCutter
            | Ability::FullMetalBody
            | Ability::InnerFocus
            | Ability::Oblivious
            | Ability::OwnTempo
            | Ability::Scrappy
    ) || target.item == Item::ClearAmulet
    {
        modifiers.push(ModifierBreakdown::new("Intimidate blocked", 0));
        return;
    } else if target.ability == Ability::MirrorArmor {
        boost_stat(source, Stat::Attack, -1);
    } else {
        let stages = if target.ability == Ability::Simple {
            -2
        } else {
            -1
        };
        boost_stat(target, Stat::Attack, stages);
        if target.ability == Ability::Defiant {
            boost_stat(target, Stat::Attack, 2);
        } else if target.ability == Ability::Competitive {
            boost_stat(target, Stat::SpecialAttack, 2);
        }
    }
    if target.item == Item::AdrenalineOrb && target.ability != Ability::MirrorArmor {
        let stages = if target.ability == Ability::Simple {
            2
        } else {
            1
        };
        boost_stat(target, Stat::Speed, stages);
        target.item = Item::None;
    }
    if target.ability == Ability::Rattled && target.item != Item::ClearAmulet {
        boost_stat(target, Stat::Speed, 1);
    }
    modifiers.push(ModifierBreakdown::new("Intimidate", 0));
}

fn check_supersweet_syrup(
    source: &Pokemon,
    target: &mut Pokemon,
    modifiers: &mut Vec<ModifierBreakdown>,
) {
    if source.ability == Ability::SupersweetSyrup
        && source.ability_on
        && target.item != Item::ClearAmulet
    {
        if target.ability == Ability::Defiant {
            boost_stat(target, Stat::Attack, 2);
        } else if target.ability == Ability::Competitive {
            boost_stat(target, Stat::SpecialAttack, 2);
        }
        modifiers.push(ModifierBreakdown::new("Supersweet Syrup", 0));
    }
}

fn check_download(
    source: &mut Pokemon,
    target: &Pokemon,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Result<(), CalcError> {
    if source.ability != Ability::Download {
        return Ok(());
    }
    let target_stats = calculate_stats(target, Ruleset::Champions)?;
    let defense = modified_stat(target_stats.defense, target.boosts.defense)?;
    let special_defense =
        modified_stat(target_stats.special_defense, target.boosts.special_defense)?;
    if special_defense <= defense {
        boost_stat(source, Stat::SpecialAttack, 1);
    } else {
        boost_stat(source, Stat::Attack, 1);
    }
    modifiers.push(ModifierBreakdown::new("Download", 0));
    Ok(())
}

fn check_embody_aspect(pokemon: &mut Pokemon, modifiers: &mut Vec<ModifierBreakdown>) {
    if pokemon.ability != Ability::EmbodyAspect {
        return;
    }
    match (pokemon.name.as_str(), pokemon.item) {
        ("Ogerpon", _) => boost_stat(pokemon, Stat::Speed, 1),
        ("Ogerpon-Wellspring", Item::WellspringMask) => {
            boost_stat(pokemon, Stat::SpecialDefense, 1)
        }
        ("Ogerpon-Hearthflame", Item::HearthflameMask) => boost_stat(pokemon, Stat::Attack, 1),
        ("Ogerpon-Cornerstone", Item::CornerstoneMask) => boost_stat(pokemon, Stat::Defense, 1),
        _ => return,
    }
    modifiers.push(ModifierBreakdown::new("Embody Aspect", 0));
}

fn check_battle_bond(pokemon: &mut Pokemon, modifiers: &mut Vec<ModifierBreakdown>) {
    if pokemon.ability == Ability::BattleBond && pokemon.ability_on {
        boost_stat(pokemon, Stat::Attack, 1);
        boost_stat(pokemon, Stat::SpecialAttack, 1);
        boost_stat(pokemon, Stat::Speed, 1);
        modifiers.push(ModifierBreakdown::new("Battle Bond", 0));
    }
}

fn boost_stat(pokemon: &mut Pokemon, stat: Stat, stages: i8) {
    let clamp = |value: i8| value.clamp(-6, 6);
    match stat {
        Stat::Attack => pokemon.boosts.attack = clamp(pokemon.boosts.attack + stages),
        Stat::Defense => pokemon.boosts.defense = clamp(pokemon.boosts.defense + stages),
        Stat::SpecialAttack => {
            pokemon.boosts.special_attack = clamp(pokemon.boosts.special_attack + stages)
        }
        Stat::SpecialDefense => {
            pokemon.boosts.special_defense = clamp(pokemon.boosts.special_defense + stages)
        }
        Stat::Speed => pokemon.boosts.speed = clamp(pokemon.boosts.speed + stages),
        Stat::Hp => {}
    }
}

fn preprocess_pokemon(pokemon: &mut Pokemon, weather: Weather, terrain: crate::types::Terrain) {
    if pokemon.ability == Ability::Forecast && pokemon.name == "Castform" {
        pokemon.types = [
            Some(if weather.is_sun() {
                PokemonType::Fire
            } else if weather.is_rain() {
                PokemonType::Water
            } else if matches!(weather, Weather::Hail | Weather::Snow) {
                PokemonType::Ice
            } else {
                PokemonType::Normal
            }),
            None,
        ];
    }
    if pokemon.ability == Ability::Mimicry && terrain != crate::types::Terrain::None {
        pokemon.types = [
            Some(match terrain {
                crate::types::Terrain::Electric => PokemonType::Electric,
                crate::types::Terrain::Grassy => PokemonType::Grass,
                crate::types::Terrain::Misty => PokemonType::Fairy,
                crate::types::Terrain::Psychic => PokemonType::Psychic,
                crate::types::Terrain::None => PokemonType::Normal,
            }),
            None,
        ];
    }
}

fn apply_move_type_changes(move_: &mut Move, attacker: &Pokemon, field: &Field) {
    match move_.name.as_str() {
        "Weather Ball" => {
            move_.type_ = if (field.weather.is_sun() && attacker.item != Item::UtilityUmbrella)
                || attacker.ability == Ability::MegaSol
            {
                PokemonType::Fire
            } else if field.weather.is_rain() && attacker.item != Item::UtilityUmbrella {
                PokemonType::Water
            } else if field.weather == Weather::Sand {
                PokemonType::Rock
            } else if matches!(field.weather, Weather::Hail | Weather::Snow) {
                PokemonType::Ice
            } else {
                PokemonType::Normal
            };
        }
        "Natural Gift" => {
            if let Some((type_, _power)) = natural_gift(attacker.item) {
                move_.type_ = type_;
            }
        }
        "Techno Blast" => {
            move_.type_ = drive_type(attacker.item).unwrap_or(PokemonType::Normal);
        }
        "Multi-Attack" => {
            if let Some(type_) = memory_type(attacker.item) {
                move_.type_ = type_;
            }
        }
        "Judgment" => {
            if let Some(type_) = item_boost_type(attacker.item) {
                move_.type_ = type_;
            }
        }
        "Revelation Dance" => {
            move_.type_ = attacker.types[0]
                .or(attacker.types[1])
                .unwrap_or(PokemonType::Typeless);
        }
        "Aura Wheel" if attacker.name == "Morpeko-Hangry" => {
            move_.type_ = PokemonType::Dark;
        }
        "Raging Bull" => {
            move_.type_ = match attacker.name.as_str() {
                "Tauros-Paldea-Combat" => PokemonType::Fighting,
                "Tauros-Paldea-Blaze" => PokemonType::Fire,
                "Tauros-Paldea-Aqua" => PokemonType::Water,
                _ => PokemonType::Normal,
            };
        }
        "Ivy Cudgel" => {
            move_.type_ = match attacker.name.as_str() {
                "Ogerpon-Wellspring" => PokemonType::Water,
                "Ogerpon-Hearthflame" => PokemonType::Fire,
                "Ogerpon-Cornerstone" => PokemonType::Rock,
                _ => PokemonType::Grass,
            };
        }
        "Terrain Pulse" => {
            move_.type_ = match field.terrain {
                crate::types::Terrain::Electric => PokemonType::Electric,
                crate::types::Terrain::Grassy => PokemonType::Grass,
                crate::types::Terrain::Misty => PokemonType::Fairy,
                crate::types::Terrain::Psychic => PokemonType::Psychic,
                crate::types::Terrain::None => PokemonType::Normal,
            };
        }
        "Tera Blast" if attacker.is_terastalized => {
            if let Some(tera_type) = attacker.tera_type {
                move_.type_ = tera_type;
            }
        }
        "Tera Starstorm" if attacker.name == "Terapagos-Stellar" => {
            move_.type_ = PokemonType::Stellar;
            move_.is_spread = true;
        }
        _ => {}
    }
}

fn apply_ability_type_change(
    move_: &mut Move,
    attacker: &Pokemon,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> bool {
    if matches!(
        move_.name.as_str(),
        "Hidden Power"
            | "Weather Ball"
            | "Natural Gift"
            | "Judgment"
            | "Techno Blast"
            | "Revelation Dance"
            | "Multi-Attack"
            | "Terrain Pulse"
    ) {
        return false;
    }
    if attacker.ability == Ability::LiquidVoice && move_.is_sound {
        move_.type_ = PokemonType::Water;
        modifiers.push(ModifierBreakdown::new("Liquid Voice type change", 0));
        return false;
    }
    if attacker.ability == Ability::Normalize {
        move_.type_ = PokemonType::Normal;
        modifiers.push(ModifierBreakdown::new("Normalize type change", 0));
        return true;
    }
    if move_.type_ != PokemonType::Normal {
        return false;
    }
    let new_type = match attacker.ability {
        Ability::Aerilate => PokemonType::Flying,
        Ability::Pixilate => PokemonType::Fairy,
        Ability::Refrigerate => PokemonType::Ice,
        Ability::Galvanize => PokemonType::Electric,
        Ability::Dragonize => PokemonType::Dragon,
        _ => return false,
    };
    move_.type_ = new_type;
    modifiers.push(ModifierBreakdown::new("type-changing ability", 0));
    true
}

fn ability_after_ignore(
    attacker_ability: Ability,
    defender_ability: Ability,
    move_: &Move,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Ability {
    if attacker_ability == Ability::MoldBreaker
        && !matches!(
            defender_ability,
            Ability::PrismArmor
                | Ability::BeadsOfRuin
                | Ability::ShadowShield
                | Ability::SwordOfRuin
                | Ability::TabletsOfRuin
                | Ability::TeraShell
                | Ability::VesselOfRuin
        )
    {
        modifiers.push(ModifierBreakdown::new("Mold Breaker ignores ability", 0));
        Ability::None
    } else if matches!(
        move_.name.as_str(),
        "Moongeist Beam" | "Sunsteel Strike" | "Photon Geyser"
    ) {
        Ability::None
    } else {
        defender_ability
    }
}

fn is_immune(
    move_: &Move,
    attacker: &Pokemon,
    defender: &Pokemon,
    def_ability: Ability,
    field: &Field,
    type_effectiveness: f32,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> bool {
    if type_effectiveness == 0.0 {
        return true;
    }
    if def_ability == Ability::WonderGuard
        && type_effectiveness <= 1.0
        && move_.type_ != PokemonType::Typeless
    {
        modifiers.push(ModifierBreakdown::new("Wonder Guard", 0));
        return true;
    }
    if (move_.type_ == PokemonType::Grass && def_ability == Ability::SapSipper)
        || (move_.type_ == PokemonType::Fire && matches!(def_ability, Ability::FlashFire))
        || (move_.type_ == PokemonType::Water
            && matches!(
                def_ability,
                Ability::DrySkin
                    | Ability::StormDrain
                    | Ability::WaterAbsorb
                    | Ability::WaterBubble
            ))
        || (move_.type_ == PokemonType::Electric
            && matches!(
                def_ability,
                Ability::LightningRod | Ability::MotorDrive | Ability::VoltAbsorb
            ))
        || (move_.is_bullet && def_ability == Ability::Bulletproof)
        || (move_.is_sound && def_ability == Ability::Soundproof)
        || (move_.is_wind && def_ability == Ability::WindRider)
    {
        modifiers.push(ModifierBreakdown::new("defensive immunity ability", 0));
        return true;
    }
    if move_.type_ == PokemonType::Ground
        && !field.gravity
        && defender.item != Item::IronBall
        && defender.item != Item::AirBalloon
        && matches!(def_ability, Ability::Levitate | Ability::EarthEater)
        && attacker.ability != Ability::MoldBreaker
    {
        modifiers.push(ModifierBreakdown::new("Levitate", 0));
        return true;
    }
    if move_.type_ == PokemonType::Ground
        && !field.gravity
        && defender.item == Item::AirBalloon
        && move_.name != "Thousand Arrows"
    {
        modifiers.push(ModifierBreakdown::new("Air Balloon", 0));
        return true;
    }
    if matches!(
        def_ability,
        Ability::QueenlyMajesty | Ability::Dazzling | Ability::ArmorTail
    ) && move_.is_priority
    {
        modifiers.push(ModifierBreakdown::new("priority-blocking ability", 0));
        return true;
    }
    if field.terrain == crate::types::Terrain::Psychic
        && move_.is_priority
        && is_grounded(defender, field)
    {
        modifiers.push(ModifierBreakdown::new("Psychic Terrain priority block", 0));
        return true;
    }
    if move_.name == "Sky Drop"
        && (defender.has_type(PokemonType::Flying) || defender.weight_kg >= 200.0 || field.gravity)
    {
        modifiers.push(ModifierBreakdown::new("Sky Drop failed", 0));
        return true;
    }
    if move_.name == "Synchronoise" && !shares_any_type(attacker, defender) {
        modifiers.push(ModifierBreakdown::new("Synchronoise failed", 0));
        return true;
    }
    if matches!(
        move_.name.as_str(),
        "Self-Destruct" | "Explosion" | "Mind Blown" | "Misty Explosion"
    ) && (def_ability == Ability::Damp || attacker.ability == Ability::Damp)
    {
        modifiers.push(ModifierBreakdown::new("Damp", 0));
        return true;
    }
    if move_.is_ohko && def_ability == Ability::Sturdy {
        modifiers.push(ModifierBreakdown::new("Sturdy", 0));
        return true;
    }
    if move_.name == "Dream Eater"
        && defender.status != StatusCondition::Asleep
        && def_ability != Ability::Comatose
    {
        modifiers.push(ModifierBreakdown::new("Dream Eater failed", 0));
        return true;
    }
    defender.name.is_empty()
}

fn shares_any_type(attacker: &Pokemon, defender: &Pokemon) -> bool {
    attacker
        .types
        .iter()
        .flatten()
        .any(|attacker_type| defender.has_type(*attacker_type))
}

fn zero_damage(
    defender_max_hp: u16,
    modifiers: Vec<ModifierBreakdown>,
    debug: Vec<String>,
) -> DamageResult {
    let _ = defender_max_hp;
    DamageResult {
        min_damage: 0,
        max_damage: 0,
        damage_rolls: vec![0],
        hit_rolls: vec![vec![0]],
        percent_range: (0.0, 0.0),
        ko_chance: Some(0.0),
        applied_modifiers: modifiers,
        debug,
    }
}

#[allow(clippy::too_many_arguments)]
fn set_damage_result(
    move_: &Move,
    attacker: &Pokemon,
    defender: &Pokemon,
    attacker_current_hp: u16,
    defender_current_hp: u16,
    defender_max_hp: u16,
    protect: bool,
    modifiers: Vec<ModifierBreakdown>,
    debug: Vec<String>,
) -> Option<DamageResult> {
    if let Some(result) = counter_damage_result(
        move_,
        defender_max_hp,
        defender_current_hp,
        modifiers.clone(),
        debug.clone(),
    ) {
        return Some(result);
    }
    let damage = match move_.name.as_str() {
        "Super Fang" | "Nature's Madness" | "Ruination" => defender_current_hp / 2,
        "Guardian of Alola" => {
            if protect {
                defender_current_hp * 3 / 16
            } else {
                defender_current_hp * 3 / 4
            }
        }
        "Endeavor" => defender_current_hp.saturating_sub(attacker_current_hp),
        "Final Gambit" => attacker_current_hp,
        "Sonic Boom" => 20,
        "Dragon Rage" => 40,
        "Seismic Toss" | "Night Shade" => attacker.level as u16,
        _ if move_.is_ohko => {
            if move_.name == "Sheer Cold" && defender.has_type(PokemonType::Ice) {
                0
            } else {
                defender_current_hp
            }
        }
        _ => return None,
    };
    Some(single_damage_result(
        damage,
        defender_max_hp,
        defender_current_hp,
        modifiers,
        debug,
    ))
}

fn counter_damage_result(
    move_: &Move,
    defender_max_hp: u16,
    defender_current_hp: u16,
    mut modifiers: Vec<ModifierBreakdown>,
    debug: Vec<String>,
) -> Option<DamageResult> {
    let rolls = move_.countered_damage_rolls.as_ref()?;
    let category = move_.countered_move_category?;
    let multiplier = match move_.name.as_str() {
        "Counter" if category == Category::Physical => 2.0,
        "Mirror Coat" if category == Category::Special => 2.0,
        "Metal Burst" | "Comeuppance" if category != Category::Status => 1.5,
        "Counter" | "Mirror Coat" | "Metal Burst" | "Comeuppance" => 0.0,
        _ => return None,
    };
    if multiplier == 0.0 {
        return Some(single_damage_result(
            0,
            defender_max_hp,
            defender_current_hp,
            modifiers,
            debug,
        ));
    }
    let mut damage_rolls = rolls
        .iter()
        .map(|roll| {
            if multiplier == 2.0 {
                roll.saturating_mul(2)
            } else {
                ((*roll as u32 * 3) / 2).min(u16::MAX as u32) as u16
            }
        })
        .collect::<Vec<_>>();
    damage_rolls.sort_unstable();
    let min_damage = *damage_rolls.first().unwrap_or(&0);
    let max_damage = *damage_rolls.last().unwrap_or(&0);
    let percent_range = (
        min_damage as f32 * 100.0 / defender_max_hp as f32,
        max_damage as f32 * 100.0 / defender_max_hp as f32,
    );
    let ko_chance = Some(
        damage_rolls
            .iter()
            .filter(|&&damage| damage >= defender_current_hp)
            .count() as f32
            / damage_rolls.len() as f32,
    );
    modifiers.push(ModifierBreakdown::new("counter-style damage", 0));
    Some(DamageResult {
        min_damage,
        max_damage,
        hit_rolls: vec![damage_rolls.clone()],
        damage_rolls,
        percent_range,
        ko_chance,
        applied_modifiers: modifiers,
        debug,
    })
}

fn single_damage_result(
    damage: u16,
    defender_max_hp: u16,
    defender_current_hp: u16,
    modifiers: Vec<ModifierBreakdown>,
    debug: Vec<String>,
) -> DamageResult {
    let percent = damage as f32 * 100.0 / defender_max_hp as f32;
    DamageResult {
        min_damage: damage,
        max_damage: damage,
        damage_rolls: vec![damage],
        hit_rolls: vec![vec![damage]],
        percent_range: (percent, percent),
        ko_chance: Some(if damage >= defender_current_hp {
            1.0
        } else {
            0.0
        }),
        applied_modifiers: modifiers,
        debug,
    }
}

#[allow(clippy::too_many_arguments)]
fn calc_base_power(
    move_: &Move,
    attacker: &Pokemon,
    defender: &Pokemon,
    attacker_current_hp: u16,
    attacker_max_hp: u16,
    defender_current_hp: u16,
    defender_max_hp: u16,
    field: &Field,
    attacker_speed: u16,
    defender_speed: u16,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Result<u16, CalcError> {
    let bp = match move_.name.as_str() {
        "Gyro Ball" => {
            let speed = attacker_speed.max(1);
            let bp = (25 * defender_speed / speed).min(150);
            modifiers.push(ModifierBreakdown::new("Gyro Ball base power", 0));
            bp
        }
        "Electro Ball" => {
            let ratio = if defender_speed == 0 {
                0
            } else {
                attacker_speed / defender_speed
            };
            let bp = if ratio >= 4 {
                150
            } else if ratio >= 3 {
                120
            } else if ratio >= 2 {
                80
            } else if ratio >= 1 {
                60
            } else {
                40
            };
            modifiers.push(ModifierBreakdown::new("Electro Ball base power", 0));
            bp
        }
        "Low Kick" | "Grass Knot" => {
            let weight = effective_weight(defender);
            let bp = if weight >= 200.0 {
                120
            } else if weight >= 100.0 {
                100
            } else if weight >= 50.0 {
                80
            } else if weight >= 25.0 {
                60
            } else if weight >= 10.0 {
                40
            } else {
                20
            };
            modifiers.push(ModifierBreakdown::new("weight-based base power", 0));
            bp
        }
        "Heavy Slam" | "Heat Crash" => {
            let defender_weight = effective_weight(defender).max(0.1);
            let ratio = effective_weight(attacker) / defender_weight;
            let bp = if ratio >= 5.0 {
                120
            } else if ratio >= 4.0 {
                100
            } else if ratio >= 3.0 {
                80
            } else if ratio >= 2.0 {
                60
            } else {
                40
            };
            modifiers.push(ModifierBreakdown::new("weight-ratio base power", 0));
            bp
        }
        "Eruption" | "Water Spout" | "Dragon Energy" => {
            let bp = (150 * attacker_current_hp / attacker_max_hp).max(1);
            modifiers.push(ModifierBreakdown::new("HP-based base power", 0));
            bp
        }
        "Flail" | "Reversal" => {
            let p = 48 * attacker_current_hp / attacker_max_hp;
            let bp = if p <= 1 {
                200
            } else if p <= 4 {
                150
            } else if p <= 9 {
                100
            } else if p <= 16 {
                80
            } else if p <= 32 {
                40
            } else {
                20
            };
            modifiers.push(ModifierBreakdown::new("HP-based base power", 0));
            bp
        }
        "Crush Grip" | "Wring Out" => {
            let hp_ratio =
                (defender_current_hp as i64 * MODIFIER_DENOMINATOR as i64) / defender_max_hp as i64;
            let rounded = poke_round_ratio(120 * 100 * hp_ratio, MODIFIER_DENOMINATOR as i64);
            let bp = (rounded / 100).max(1) as u16;
            modifiers.push(ModifierBreakdown::new("HP-ratio base power", 0));
            bp
        }
        "Hard Press" => {
            let hp_ratio =
                (defender_current_hp as i64 * MODIFIER_DENOMINATOR as i64) / defender_max_hp as i64;
            let rounded = poke_round_ratio(100 * 100 * hp_ratio, MODIFIER_DENOMINATOR as i64);
            let bp = (rounded / 100).max(1) as u16;
            modifiers.push(ModifierBreakdown::new("HP-ratio base power", 0));
            bp
        }
        "Stored Power" | "Power Trip" => {
            let bp = 20 + 20 * count_positive_boosts(attacker.boosts);
            modifiers.push(ModifierBreakdown::new("boost-count base power", 0));
            bp
        }
        "Punishment" => {
            let bp = (60 + 20 * count_positive_boosts(defender.boosts)).min(200);
            modifiers.push(ModifierBreakdown::new("target boost-count base power", 0));
            bp
        }
        "Acrobatics" => {
            let bp = if matches!(attacker.item, Item::None | Item::FlyingGem) {
                110
            } else {
                move_.base_power
            };
            if bp != move_.base_power {
                modifiers.push(ModifierBreakdown::new("Acrobatics base power", MOD_DOUBLE));
            }
            bp
        }
        "Hex" | "Infernal Parade" => {
            let bp = if defender.status != StatusCondition::Healthy {
                move_.base_power * 2
            } else {
                move_.base_power
            };
            if bp != move_.base_power {
                modifiers.push(ModifierBreakdown::new("status base power", MOD_DOUBLE));
            }
            bp
        }
        "Smelling Salts" => {
            let bp = if defender.status == StatusCondition::Paralyzed {
                move_.base_power * 2
            } else {
                move_.base_power
            };
            if bp != move_.base_power {
                modifiers.push(ModifierBreakdown::new(
                    "Smelling Salts base power",
                    MOD_DOUBLE,
                ));
            }
            bp
        }
        "Wake-Up Slap" => {
            let bp = if defender.status == StatusCondition::Asleep {
                move_.base_power * 2
            } else {
                move_.base_power
            };
            if bp != move_.base_power {
                modifiers.push(ModifierBreakdown::new(
                    "Wake-Up Slap base power",
                    MOD_DOUBLE,
                ));
            }
            bp
        }
        "Weather Ball"
            if field.weather != Weather::None && field.weather != Weather::StrongWinds =>
        {
            modifiers.push(ModifierBreakdown::new(
                "Weather Ball base power",
                MOD_DOUBLE,
            ));
            move_.base_power * 2
        }
        "Terrain Pulse" if field.terrain != crate::types::Terrain::None => {
            modifiers.push(ModifierBreakdown::new(
                "Terrain Pulse base power",
                MOD_DOUBLE,
            ));
            move_.base_power * 2
        }
        "Rising Voltage"
            if field.terrain == crate::types::Terrain::Electric && is_grounded(defender, field) =>
        {
            modifiers.push(ModifierBreakdown::new(
                "Rising Voltage base power",
                MOD_DOUBLE,
            ));
            move_.base_power * 2
        }
        "Tera Blast" if move_.type_ == PokemonType::Stellar => {
            modifiers.push(ModifierBreakdown::new("Stellar Tera Blast base power", 0));
            100
        }
        "Fling" => {
            if !can_fling(attacker.item, &attacker.name, defender.ability) {
                return Ok(0);
            }
            let bp = fling_power(attacker.item).unwrap_or(10);
            modifiers.push(ModifierBreakdown::new("Fling base power", 0));
            bp
        }
        "Natural Gift" => {
            if let Some((_type, power)) = natural_gift(attacker.item) {
                modifiers.push(ModifierBreakdown::new("Natural Gift base power", 0));
                power
            } else {
                0
            }
        }
        "Triple Kick" | "Triple Axel" => {
            move_.base_power * move_.current_triple_hit.unwrap_or(1) as u16
        }
        "Last Respects" | "Rage Fist" => {
            let bp = move_.base_power * (move_.times_affected as u16 + 1);
            if move_.times_affected > 0 {
                modifiers.push(ModifierBreakdown::new("times-affected base power", 0));
            }
            bp
        }
        _ if move_.is_double_power
            && !matches!(
                move_.name.as_str(),
                "Retaliate" | "Fusion Bolt" | "Fusion Flare" | "Lash Out"
            ) =>
        {
            modifiers.push(ModifierBreakdown::new("double-power move flag", MOD_DOUBLE));
            move_.base_power * 2
        }
        _ => move_.base_power,
    };
    Ok(bp)
}

fn calc_bp_mods(
    move_: &Move,
    attacker: &Pokemon,
    defender: &Pokemon,
    field: &Field,
    base_power: u16,
    ate_ize_boosted: bool,
    attacker_moves_first: bool,
    defender_current_hp: u16,
    defender_max_hp: u16,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Vec<i32> {
    let mut mods = Vec::new();

    if ate_ize_boosted {
        push_mod(&mut mods, modifiers, "type-changing ability boost", MOD_1_2);
    }

    if !ate_ize_boosted
        && ((attacker.ability == Ability::Reckless && (move_.has_recoil || move_.has_crash))
            || (attacker.ability == Ability::IronFist && move_.is_punch))
    {
        push_mod(&mut mods, modifiers, "base power ability 1.2", MOD_1_2);
    }

    if field.battery && move_.category == Category::Special {
        push_mod(&mut mods, modifiers, "Battery", MOD_1_3);
    }
    if field.power_spot {
        push_mod(&mut mods, modifiers, "Power Spot", MOD_1_3);
    }
    if field.steely_spirit && move_.type_ == PokemonType::Steel {
        push_mod(&mut mods, modifiers, "Ally Steely Spirit", MOD_1_5);
    }

    if (attacker.ability == Ability::SheerForce && move_.has_secondary_effect)
        || (attacker.ability == Ability::SandForce
            && field.weather == Weather::Sand
            && matches!(
                move_.type_,
                PokemonType::Rock | PokemonType::Ground | PokemonType::Steel
            ))
        || (attacker.ability == Ability::Analytic && !attacker_moves_first)
        || (attacker.ability == Ability::ToughClaws && makes_effective_contact(attacker, move_))
        || (attacker.ability == Ability::PunkRock && move_.is_sound)
    {
        push_mod(&mut mods, modifiers, "base power ability 1.3", MOD_1_3);
    }

    let temp_bp = apply_mod(base_power as i32, chain_mods(&mods));
    if (attacker.ability == Ability::Technician && temp_bp <= 60)
        || (attacker.ability == Ability::MegaLauncher && move_.is_pulse)
        || (attacker.ability == Ability::StrongJaw && move_.is_bite)
        || (attacker.ability == Ability::SteelySpirit && move_.type_ == PokemonType::Steel)
    {
        push_mod(&mut mods, modifiers, "base power ability 1.5", MOD_1_5);
    }

    if (attacker.item == Item::MuscleBand && move_.category == Category::Physical)
        || (attacker.item == Item::WiseGlasses && move_.category == Category::Special)
    {
        push_mod(&mut mods, modifiers, "1.1x item", MOD_1_1);
    } else if item_boost_type(attacker.item) == Some(move_.type_) {
        push_mod(&mut mods, modifiers, "type item", MOD_1_2);
    } else if gem_type(attacker.item) == Some(move_.type_) && is_gem(attacker.item) {
        push_mod(&mut mods, modifiers, "gem", MOD_1_3);
    }

    if field.helping_hand {
        push_mod(&mut mods, modifiers, "Helping Hand", MOD_1_5);
    }
    if (field.charge
        || (matches!(
            attacker.ability,
            Ability::Electromorphosis | Ability::WindPower
        ) && attacker.ability_on))
        && move_.type_ == PokemonType::Electric
    {
        push_mod(&mut mods, modifiers, "Charge", MOD_DOUBLE);
    }
    if matches!(move_.name.as_str(), "Solar Beam" | "Solar Blade")
        && !matches!(
            field.weather,
            Weather::None | Weather::Sun | Weather::HarshSun | Weather::StrongWinds
        )
        && attacker.item != Item::UtilityUmbrella
        && attacker.ability != Ability::MegaSol
    {
        push_mod(&mut mods, modifiers, "bad-weather Solar move", MOD_HALF);
    }

    if is_grounded(attacker, field) {
        let terrain_modifier = MOD_1_3;
        if field.terrain == crate::types::Terrain::Electric && move_.type_ == PokemonType::Electric
        {
            push_mod(&mut mods, modifiers, "Electric Terrain", terrain_modifier);
        } else if field.terrain == crate::types::Terrain::Grassy
            && move_.type_ == PokemonType::Grass
        {
            push_mod(&mut mods, modifiers, "Grassy Terrain", terrain_modifier);
        } else if field.terrain == crate::types::Terrain::Psychic
            && move_.type_ == PokemonType::Psychic
        {
            push_mod(&mut mods, modifiers, "Psychic Terrain", terrain_modifier);
        }
    }
    if is_grounded(defender, field) {
        if (field.terrain == crate::types::Terrain::Misty && move_.type_ == PokemonType::Dragon)
            || (field.terrain == crate::types::Terrain::Grassy
                && matches!(move_.name.as_str(), "Earthquake" | "Bulldoze"))
        {
            push_mod(&mut mods, modifiers, "defensive terrain", MOD_HALF);
        }
    }

    if matches!(
        attacker.status,
        StatusCondition::Burned
            | StatusCondition::Paralyzed
            | StatusCondition::Poisoned
            | StatusCondition::BadlyPoisoned
    ) && move_.name == "Facade"
    {
        push_mod(&mut mods, modifiers, "Facade", MOD_DOUBLE);
    }
    if defender_current_hp <= defender_max_hp / 2 && move_.name == "Brine" {
        push_mod(&mut mods, modifiers, "Brine", MOD_DOUBLE);
    }
    if (matches!(move_.name.as_str(), "Venoshock" | "Barb Barrage")
        && matches!(
            defender.status,
            StatusCondition::Poisoned | StatusCondition::BadlyPoisoned
        ))
        || (matches!(
            move_.name.as_str(),
            "Retaliate" | "Fusion Bolt" | "Fusion Flare" | "Lash Out"
        ) && move_.is_double_power)
    {
        push_mod(&mut mods, modifiers, "conditional double power", MOD_DOUBLE);
    }
    if move_.name == "Knock Off" && !cant_remove_item(defender.item, &defender.name) {
        push_mod(&mut mods, modifiers, "Knock Off", MOD_1_5);
    } else if field.terrain == crate::types::Terrain::Electric && move_.name == "Psyblade" {
        push_mod(&mut mods, modifiers, "Psyblade", MOD_1_5);
    } else if (move_.name == "Misty Explosion"
        && field.terrain == crate::types::Terrain::Misty
        && is_grounded(attacker, field))
        || (move_.name == "Grav Apple" && field.gravity)
        || (move_.name == "Expanding Force"
            && field.terrain == crate::types::Terrain::Psychic
            && is_grounded(attacker, field))
    {
        push_mod(&mut mods, modifiers, "field base power boost", MOD_1_5);
    }

    if attacker.supreme_overlord_allies > 0 && attacker.ability == Ability::SupremeOverlord {
        let table = [MOD_1_1_ALT, MOD_1_2, MOD_1_3, 0x1666, MOD_1_5];
        let idx = attacker.supreme_overlord_allies.min(5) as usize - 1;
        push_mod(&mut mods, modifiers, "Supreme Overlord", table[idx]);
    }
    if attacker.item == Item::PunchingGlove && move_.is_punch {
        push_mod(&mut mods, modifiers, "Punching Glove", MOD_1_1_ALT);
    }
    for &modifier in &attacker.custom_bp_mods {
        push_mod(&mut mods, modifiers, "custom BP modifier", modifier);
    }

    mods
}

fn calc_attack_mods(
    move_: &Move,
    attacker: &Pokemon,
    attacker_highest_stat: Stat,
    def_ability: Ability,
    field: &Field,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Vec<i32> {
    let mut mods = Vec::new();
    if field.tablets_of_ruin
        && move_.category == Category::Physical
        && attacker.ability != Ability::TabletsOfRuin
    {
        push_mod(&mut mods, modifiers, "Tablets of Ruin", MOD_THREE_QUARTERS);
    } else if field.vessel_of_ruin
        && move_.category == Category::Special
        && attacker.ability != Ability::VesselOfRuin
    {
        push_mod(&mut mods, modifiers, "Vessel of Ruin", MOD_THREE_QUARTERS);
    }
    if (attacker.ability == Ability::Defeatist && is_half_hp(attacker))
        || (attacker.ability == Ability::None && false)
    {
        push_mod(&mut mods, modifiers, "attack ability 0.5", MOD_HALF);
    }
    if ((attacker.ability == Ability::FlowerGift && field.weather.is_sun())
        || field.flower_gift_attack)
        && move_.category == Category::Physical
        && attacker.item != Item::UtilityUmbrella
    {
        push_mod(&mut mods, modifiers, "Flower Gift attack", MOD_1_5);
    }

    if (attacker.ability == Ability::Guts
        && attacker.status != StatusCondition::Healthy
        && move_.category == Category::Physical)
        || (attacker.ability == Ability::FlareBoost
            && attacker.status == StatusCondition::Burned
            && move_.category == Category::Special)
        || (attacker.ability == Ability::Overgrow
            && is_third_hp(attacker)
            && move_.type_ == PokemonType::Grass)
        || (attacker.ability == Ability::Blaze
            && is_third_hp(attacker)
            && move_.type_ == PokemonType::Fire)
        || (attacker.ability == Ability::Torrent
            && is_third_hp(attacker)
            && move_.type_ == PokemonType::Water)
        || (attacker.ability == Ability::Swarm
            && is_third_hp(attacker)
            && move_.type_ == PokemonType::Bug)
        || (attacker.ability == Ability::DragonMaw && move_.type_ == PokemonType::Dragon)
        || (attacker.ability == Ability::FlashFire
            && attacker.ability_on
            && move_.type_ == PokemonType::Fire)
        || (attacker.ability == Ability::Steelworker && move_.type_ == PokemonType::Steel)
        || (matches!(attacker.ability, Ability::Plus | Ability::Minus) && attacker.ability_on)
        || (attacker.ability == Ability::Sharpness && move_.is_slice)
        || (attacker.ability == Ability::RockyPayload && move_.type_ == PokemonType::Rock)
    {
        push_mod(&mut mods, modifiers, "attack ability 1.5", MOD_1_5);
    } else if attacker.ability == Ability::SolarPower
        && field.weather.is_sun()
        && move_.category == Category::Special
        && attacker.item != Item::UtilityUmbrella
    {
        push_mod(&mut mods, modifiers, "Solar Power", MOD_1_5);
    } else if paradox_offense_boosts(attacker, attacker_highest_stat, move_) {
        push_mod(&mut mods, modifiers, "Paradox ability attack", MOD_1_3);
    } else if attacker.ability == Ability::Transistor && move_.type_ == PokemonType::Electric {
        push_mod(&mut mods, modifiers, "Transistor", MOD_1_3);
    } else if (attacker.ability == Ability::OrichalcumPulse
        && field.weather == Weather::Sun
        && move_.category == Category::Physical
        && attacker.item != Item::UtilityUmbrella)
        || (attacker.ability == Ability::HadronEngine
            && field.terrain == crate::types::Terrain::Electric
            && move_.category == Category::Special)
    {
        push_mod(&mut mods, modifiers, "box legend attack", MOD_1_33);
    }

    if (attacker.item == Item::LightBall
        && matches!(attacker.name.as_str(), "Pikachu" | "Pikachu-Gmax"))
        || (attacker.ability == Ability::WaterBubble && move_.type_ == PokemonType::Water)
        || (matches!(attacker.ability, Ability::HugePower | Ability::PurePower)
            && move_.category == Category::Physical)
        || (attacker.ability == Ability::Stakeout && attacker.ability_on)
    {
        push_mod(&mut mods, modifiers, "attack/item 2.0", MOD_DOUBLE);
    }

    if (def_ability == Ability::ThickFat
        && matches!(move_.type_, PokemonType::Fire | PokemonType::Ice))
        || (def_ability == Ability::WaterBubble && move_.type_ == PokemonType::Fire)
        || (def_ability == Ability::PurifyingSalt && move_.type_ == PokemonType::Ghost)
        || (def_ability == Ability::Heatproof && move_.type_ == PokemonType::Fire)
    {
        push_mod(&mut mods, modifiers, "defensive attack reduction", MOD_HALF);
    }

    if (attacker.item == Item::ChoiceBand && move_.category == Category::Physical)
        || (attacker.item == Item::ChoiceSpecs && move_.category == Category::Special)
    {
        push_mod(&mut mods, modifiers, "Choice item", MOD_1_5);
    }
    for &modifier in &attacker.custom_attack_mods {
        push_mod(&mut mods, modifiers, "custom attack modifier", modifier);
    }

    mods
}

fn calc_defense_mods(
    _move_: &Move,
    defender: &Pokemon,
    defender_highest_stat: Stat,
    def_ability: Ability,
    field: &Field,
    hits_physical: bool,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Vec<i32> {
    let mut mods = Vec::new();
    if field.sword_of_ruin && hits_physical && def_ability != Ability::SwordOfRuin {
        push_mod(&mut mods, modifiers, "Sword of Ruin", MOD_THREE_QUARTERS);
    } else if field.beads_of_ruin && !hits_physical && def_ability != Ability::BeadsOfRuin {
        push_mod(&mut mods, modifiers, "Beads of Ruin", MOD_THREE_QUARTERS);
    }
    if ((def_ability == Ability::FlowerGift && field.weather.is_sun())
        || field.flower_gift_special_defense)
        && !hits_physical
        && defender.item != Item::UtilityUmbrella
    {
        push_mod(&mut mods, modifiers, "Flower Gift SpD", MOD_1_5);
    }
    if (def_ability == Ability::MarvelScale
        && defender.status != StatusCondition::Healthy
        && hits_physical)
        || (def_ability == Ability::GrassPelt
            && field.terrain == crate::types::Terrain::Grassy
            && hits_physical)
    {
        push_mod(&mut mods, modifiers, "defense ability 1.5", MOD_1_5);
    } else if defender.paradox_ability_boost
        && ((defender_highest_stat == Stat::Defense && hits_physical)
            || (defender_highest_stat == Stat::SpecialDefense && !hits_physical))
    {
        push_mod(&mut mods, modifiers, "Paradox ability defense", MOD_1_3);
    } else if def_ability == Ability::FurCoat && hits_physical {
        push_mod(&mut mods, modifiers, "defense ability 2.0", MOD_DOUBLE);
    }
    if defender.item == Item::AssaultVest && !hits_physical {
        push_mod(&mut mods, modifiers, "Assault Vest", MOD_1_5);
    } else if defender.item == Item::Eviolite && defender.can_evolve {
        push_mod(&mut mods, modifiers, "Eviolite", MOD_1_5);
    }
    for &modifier in &defender.custom_defense_mods {
        push_mod(&mut mods, modifiers, "custom defense modifier", modifier);
    }
    mods
}

fn paradox_offense_boosts(attacker: &Pokemon, attacker_highest_stat: Stat, move_: &Move) -> bool {
    attacker.paradox_ability_boost
        && ((attacker_highest_stat == Stat::Attack && move_.category == Category::Physical)
            || (attacker_highest_stat == Stat::SpecialAttack
                && move_.category == Category::Special))
}

fn stab_modifier(
    attacker: &Pokemon,
    original_types: [Option<PokemonType>; 2],
    move_: &Move,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> i32 {
    if move_.type_ == PokemonType::Typeless {
        return MODIFIER_DENOMINATOR;
    }
    let original_has_type = original_types
        .into_iter()
        .flatten()
        .any(|t| t == move_.type_);
    if attacker.is_terastalized && attacker.tera_type != Some(PokemonType::Stellar) {
        let tera_type = attacker.tera_type.unwrap_or(move_.type_);
        if move_.type_ == tera_type && original_has_type {
            if attacker.ability == Ability::Adaptability {
                modifiers.push(ModifierBreakdown::new("Adaptability Tera STAB", 0x2400));
                return 0x2400;
            }
            modifiers.push(ModifierBreakdown::new("Tera boosted STAB", MOD_DOUBLE));
            return MOD_DOUBLE;
        }
        if (move_.type_ != tera_type && original_has_type) || move_.type_ == tera_type {
            if attacker.ability == Ability::Adaptability && move_.type_ == tera_type {
                modifiers.push(ModifierBreakdown::new("Adaptability Tera STAB", MOD_DOUBLE));
                return MOD_DOUBLE;
            }
            modifiers.push(ModifierBreakdown::new("Tera STAB", MOD_1_5));
            return MOD_1_5;
        }
    } else if attacker.is_terastalized && attacker.tera_type == Some(PokemonType::Stellar) {
        if move_.gets_stellar_boost {
            if original_has_type {
                modifiers.push(ModifierBreakdown::new(
                    "Stellar original-type STAB",
                    MOD_DOUBLE,
                ));
                return MOD_DOUBLE;
            }
            modifiers.push(ModifierBreakdown::new("Stellar boost", MOD_1_2));
            return MOD_1_2;
        }
    }

    let current_has_type = attacker.has_type(move_.type_);
    if current_has_type {
        if attacker.ability == Ability::Adaptability {
            modifiers.push(ModifierBreakdown::new("Adaptability STAB", MOD_DOUBLE));
            MOD_DOUBLE
        } else {
            modifiers.push(ModifierBreakdown::new("STAB", MOD_1_5));
            MOD_1_5
        }
    } else if attacker.ability == Ability::Protean && attacker.ability_on {
        modifiers.push(ModifierBreakdown::new("Protean", MOD_1_5));
        MOD_1_5
    } else {
        MODIFIER_DENOMINATOR
    }
}

fn calc_final_mods(
    move_: &Move,
    attacker: &Pokemon,
    defender: &Pokemon,
    def_ability: Ability,
    field: &Field,
    is_critical: bool,
    type_effectiveness: f32,
    defender_current_hp: u16,
    defender_max_hp: u16,
    modifiers: &mut Vec<ModifierBreakdown>,
) -> Vec<i32> {
    let mut mods = Vec::new();
    let ignores_screens = move_.ignores_screens || attacker.ability == Ability::Infiltrator;
    if field.defender_side.aurora_veil && !is_critical && !ignores_screens {
        let modifier = if field.format == Format::Singles {
            MOD_HALF
        } else {
            MOD_0_67_DOUBLES_SCREEN
        };
        push_mod(&mut mods, modifiers, "Aurora Veil", modifier);
    } else if field.defender_side.reflect
        && move_.category == Category::Physical
        && !is_critical
        && !ignores_screens
    {
        let modifier = if field.format == Format::Singles {
            MOD_HALF
        } else {
            MOD_0_67_DOUBLES_SCREEN
        };
        push_mod(&mut mods, modifiers, "Reflect", modifier);
    } else if field.defender_side.light_screen
        && move_.category == Category::Special
        && !is_critical
        && !ignores_screens
    {
        let modifier = if field.format == Format::Singles {
            MOD_HALF
        } else {
            MOD_0_67_DOUBLES_SCREEN
        };
        push_mod(&mut mods, modifiers, "Light Screen", modifier);
    }

    if attacker.ability == Ability::Neuroforce && type_effectiveness > 1.0 {
        push_mod(&mut mods, modifiers, "Neuroforce", MOD_1_25);
    }
    if matches!(move_.name.as_str(), "Collision Course" | "Electro Drift")
        && type_effectiveness > 1.0
    {
        push_mod(
            &mut mods,
            modifiers,
            "super-effective signature move",
            MOD_1_33,
        );
    }
    if attacker.ability == Ability::Sniper && is_critical {
        push_mod(&mut mods, modifiers, "Sniper", MOD_1_5);
    }
    if attacker.ability == Ability::TintedLens && type_effectiveness < 1.0 {
        push_mod(&mut mods, modifiers, "Tinted Lens", MOD_DOUBLE);
    }

    if matches!(def_ability, Ability::Multiscale | Ability::ShadowShield)
        && defender_current_hp == defender_max_hp
    {
        push_mod(&mut mods, modifiers, "Multiscale", MOD_HALF);
    }
    if def_ability == Ability::Fluffy && makes_effective_contact(attacker, move_) {
        push_mod(&mut mods, modifiers, "Fluffy contact", MOD_HALF);
    }
    if def_ability == Ability::PunkRock && move_.is_sound {
        push_mod(&mut mods, modifiers, "Punk Rock defense", MOD_HALF);
    }
    if def_ability == Ability::IceScales && move_.category == Category::Special {
        push_mod(&mut mods, modifiers, "Ice Scales", MOD_HALF);
    }
    if field.defender_side.friend_guard {
        push_mod(&mut mods, modifiers, "Friend Guard", MOD_THREE_QUARTERS);
    }
    if matches!(
        def_ability,
        Ability::SolidRock | Ability::Filter | Ability::PrismArmor
    ) && type_effectiveness > 1.0
    {
        push_mod(
            &mut mods,
            modifiers,
            "super-effective reducer",
            MOD_THREE_QUARTERS,
        );
    }
    if def_ability == Ability::Fluffy && move_.type_ == PokemonType::Fire {
        push_mod(&mut mods, modifiers, "Fluffy fire", MOD_DOUBLE);
    }
    if attacker.item == Item::ExpertBelt && type_effectiveness > 1.0 {
        push_mod(&mut mods, modifiers, "Expert Belt", MOD_1_2);
    } else if attacker.item == Item::LifeOrb {
        push_mod(&mut mods, modifiers, "Life Orb", MOD_LIFE_ORB);
    }
    if berry_resist_type(defender.item) == Some(move_.type_)
        && (type_effectiveness > 1.0 || move_.type_ == PokemonType::Normal)
        && !matches!(attacker.ability, Ability::Unnerve | Ability::AsOne)
    {
        let modifier = if def_ability == Ability::Ripen {
            MODIFIER_DENOMINATOR / 4
        } else {
            MOD_HALF
        };
        push_mod(&mut mods, modifiers, "resist berry", modifier);
    }
    for &modifier in &attacker.custom_final_mods {
        push_mod(&mut mods, modifiers, "custom final modifier", modifier);
    }
    mods
}

fn weather_damage_boost(
    move_: &Move,
    attacker_ability: Ability,
    weather: Weather,
    attacker_item: Item,
    defender_item: Item,
) -> bool {
    (((weather.is_sun() || attacker_ability == Ability::MegaSol)
        && move_.type_ == PokemonType::Fire)
        || (weather.is_rain() && move_.type_ == PokemonType::Water))
        && defender_item != Item::UtilityUmbrella
        || ((weather.is_sun() || attacker_ability == Ability::MegaSol)
            && move_.name == "Hydro Steam"
            && attacker_item != Item::UtilityUmbrella)
}

fn weather_damage_drop(
    move_: &Move,
    attacker_ability: Ability,
    weather: Weather,
    defender_item: Item,
) -> bool {
    ((weather == Weather::Sun && move_.type_ == PokemonType::Water)
        || (weather == Weather::Rain
            && move_.type_ == PokemonType::Fire
            && attacker_ability != Ability::MegaSol))
        && defender_item != Item::UtilityUmbrella
}

fn push_mod(
    mods: &mut Vec<i32>,
    breakdown: &mut Vec<ModifierBreakdown>,
    label: &'static str,
    modifier: i32,
) {
    mods.push(modifier);
    breakdown.push(ModifierBreakdown::new(label, modifier));
}

fn is_half_hp(pokemon: &Pokemon) -> bool {
    match (pokemon.current_hp, pokemon.max_hp_override) {
        (Some(cur), Some(max)) => cur <= max / 2,
        _ => false,
    }
}

fn is_third_hp(pokemon: &Pokemon) -> bool {
    match (pokemon.current_hp, pokemon.max_hp_override) {
        (Some(cur), Some(max)) => cur <= max / 3,
        _ => false,
    }
}

fn count_positive_boosts(boosts: crate::types::Boosts) -> u16 {
    [
        boosts.attack,
        boosts.defense,
        boosts.special_attack,
        boosts.special_defense,
        boosts.speed,
    ]
    .into_iter()
    .filter(|boost| *boost > 0)
    .map(|boost| boost as u16)
    .sum()
}

fn effective_weight(pokemon: &Pokemon) -> f32 {
    let mut weight = pokemon.weight_kg;
    if pokemon.ability == Ability::HeavyMetal {
        weight *= 2.0;
    } else if pokemon.ability == Ability::LightMetal {
        weight /= 2.0;
    }
    if pokemon.item == Item::FloatStone {
        weight /= 2.0;
    }
    weight.max(0.1)
}

fn final_speed(
    pokemon: &Pokemon,
    staged_speed: u16,
    tailwind: bool,
    swamp: bool,
    highest_stat: Stat,
) -> u16 {
    let mut speed = staged_speed as i32;
    let mut speed_mods = Vec::new();
    if pokemon.item == Item::ChoiceScarf {
        speed_mods.push(MOD_1_5);
    } else if pokemon.item == Item::IronBall {
        speed_mods.push(MOD_HALF);
    }
    if tailwind {
        speed_mods.push(MOD_DOUBLE);
    }
    if swamp {
        speed_mods.push(0x0400);
    }
    if pokemon.paradox_ability_boost && highest_stat == Stat::Speed {
        speed_mods.push(MOD_1_5);
    }
    if !speed_mods.is_empty() {
        speed = apply_mod(speed, chain_mods(&speed_mods));
    }
    if pokemon.status == StatusCondition::Paralyzed {
        speed /= 2;
    }
    if speed > 65535 {
        speed %= 65536;
    }
    speed.min(10000).max(0) as u16
}

fn cant_remove_item(item: Item, species: &str) -> bool {
    if matches!(item, Item::None | Item::KlutzSuppressed) {
        return true;
    }
    if locked_item_for_species(species) == Some(item) {
        return true;
    }
    if species == "Arceus" && item_boost_type(item).is_some() {
        return true;
    }
    false
}

fn is_grounded(pokemon: &Pokemon, field: &Field) -> bool {
    field.gravity
        || pokemon.item == Item::IronBall
        || (pokemon.ability != Ability::Levitate
            && pokemon.item != Item::AirBalloon
            && !pokemon.has_type(PokemonType::Flying))
}
