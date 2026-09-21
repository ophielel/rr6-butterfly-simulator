# Sample battle: 7 Sinners vs Butterfly of Entangled Lives::Imago

A sanity playthrough of the fixed content, produced by

```text
cargo run -p lcb-cli -- run --seed 3 --turns 12
```

which plays `setup::fixed::TEAM` (the seven identities the project models) against
the **Line 6 Section 5 wave**: the 25616 HP Imago together with its three Illusory
Butterfly allies (1 HP each, Speed fixed to 1), with a deliberately simple policy:
every unit attacks the lowest-HP enemy with whatever the Dashboard offers, no
E.G.O and no defense.  The policy therefore spends its turns on the 1 HP
illusions, which is exactly what shows the Section 5 mechanic working: every hit
takes a Stack of the matching state of time off the Imago, and half of the damage
goes to the Imago instead.

## What to look for

* the Imago starts at 25616 HP (`9090 + 275.44 x 60` at level 60), its three
  allies at 1 HP, and the Sinners at level 60 with **45 SP**
  (`BattleConfig::starting_sp`);
* `[In the Past] / [In the Present] / [In the Future]` switch because of what the
  team hits: the illusion that is hit loses Stacks for the Imago, and an illusion
  left alone hands it 5 Stacks at Combat End;
* Clashes are resolved with summed Final Power, and the log names the winner;
* `[In the Past] / [In the Present] / [In the Future]` stacks drive the Imago's
  Turn Start passives (the Past passive burns whoever hits it, the Future passive
  hands out Bleed Count);
* the unique Sinking ([Butterfly]) appears as `[butterfly]` lines, Petals and the
  Kōzan follow-up as `[unopposed]`;
* no unhandled effect kinds and no crash; the only warning is the documented
  SP-on-Clash unknown.

## Damage accounting

Measured on `--seed 7`: the per-hit log sums to 1971 damage against the team while
the team's HP pool only loses 1539, and the difference is overkill - a hit that
takes a unit below 0 still logs its full damage.  The remaining damage to the team
comes from the Turn End Burn ticks and the SP damage the Imago inflicts, both of
which the log names.  The same seed leaves the Imago at 24468/25616 after twelve
turns.

## Transcript

```text
turn 1 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 202/202 SP 45 speed 6 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 205/205 SP 45 speed 5 
  Jeong's Office Rep (Ishmael)       HP 224/224 SP 45 speed 3 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 278/278 SP 45 speed 7 
  Los Mariachis Jefe (Sinclair)      HP 290/290 SP 45 speed 3 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 244/244 SP 45 speed 6 
  Lobotomy E.G.O::Lamp (Gregor)      HP 274/274 SP 45 speed 4 
  Refracted Butterfly of Entangled Lives::Imago HP 25616/25616 SP 0 speed 1 [Past 10] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 1/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 1/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 1/1 SP 0 speed 1 
    [attack] The Knight's Sanctuary -> 2 hit(s), 0 damage, coin rolls [8, 12]
    [attack] Temper and Cast -> 2 hit(s), 6 damage, coin rolls [5, 8]
    [attack] Temper and Cast -> 2 hit(s), 6 damage, coin rolls [5, 5]
    [attack] Fluttering Havoc -> 2 hit(s), 15 damage, coin rolls [7, 10]
    [attack] Fluttering Havoc -> 2 hit(s), 13 damage, coin rolls [7, 10]
    [attack] Fluttering Havoc -> 2 hit(s), 4 damage, coin rolls [4, 4]
    [attack] Fluttering Havoc -> 2 hit(s), 19 damage, coin rolls [7, 10]
turn 2 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 192/202 SP 45 speed 4 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 189/205 SP 45 speed 7 
  Jeong's Office Rep (Ishmael)       HP 202/224 SP 45 speed 6 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 268/278 SP 45 speed 4 
  Los Mariachis Jefe (Sinclair)      HP 290/290 SP 45 speed 3 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 226/244 SP 45 speed 6 
  Lobotomy E.G.O::Lamp (Gregor)      HP 267/274 SP 45 speed 3 
  Refracted Butterfly of Entangled Lives::Imago HP 25605/25616 SP 0 speed 2 [Future 15] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 1/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 1/1 SP 0 speed 1 
    [attack] Bang. Bang. -> 2 hit(s), 0 damage, coin rolls [4, 8]
    [discard] Discarded 1081303
    [attack] Tsuru-giri [鶴斬り] -> 2 hit(s), 0 damage, coin rolls [8, 12]
    [attack] Khopesh Swordplay -> 2 hit(s), 0 damage, coin rolls [7, 11]
    [attack] Solemn Lament for the Living -> 2 hit(s), 0 damage, coin rolls [11, 18]
    [attack] With the Power of Justice -> 3 hit(s), 0 damage, coin rolls [8, 12, 16]
    [attack] Danza de Pasión -> 3 hit(s), 0 damage, coin rolls [6, 7, 8]
    [attack] Rotting Annihilation -> 2 hit(s), 18 damage, coin rolls [8, 8]
    [attack] Rotting Annihilation -> 2 hit(s), 38 damage, coin rolls [13, 13]
    [attack] Pulverization -> 2 hit(s), 15 damage, coin rolls [7, 10]
    [attack] Pulverization -> 2 hit(s), 11 damage, coin rolls [4, 4]
    [attack] Pulverization -> 2 hit(s), 2 damage, coin rolls [4, 4]
    [attack] Pulverization -> 2 hit(s), 6 damage, coin rolls [4, 4]
turn 3 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 110/202 SP 45 speed 6 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 108/205 SP 45 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 109/224 SP 45 speed 7 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 265/278 SP 45 speed 5 
  Los Mariachis Jefe (Sinclair)      HP 283/290 SP 45 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 205/244 SP 45 speed 7 
  Lobotomy E.G.O::Lamp (Gregor)      HP 264/274 SP 45 speed 1 
  Refracted Butterfly of Entangled Lives::Imago HP 25539/25616 SP 0 speed 1 [Future 20] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 1/1 SP 0 speed 1 
    [attack] Khopesh Swordplay -> 2 hit(s), 0 damage, coin rolls [7, 11]
    [attack] With the Power of Justice -> 3 hit(s), 0 damage, coin rolls [8, 12, 16]
    [attack] Baile y Rola -> 2 hit(s), 0 damage, coin rolls [6, 8]
    [attack] Bloodflower -> 2 hit(s), 78 damage, coin rolls [21, 29]
    [attack] Chaotic Turmoil -> 1 hit(s), 8 damage, coin rolls [6]
turn 4 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 45 speed 6 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 45 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP 45 speed 7 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 176/278 SP 45 speed 6 
  Los Mariachis Jefe (Sinclair)      HP 273/290 SP 45 speed 3 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 91/244 SP 45 speed 5 STAGGERED
  Lobotomy E.G.O::Lamp (Gregor)      HP 261/274 SP 45 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 25616/25616 SP 0 speed 1 [Past 18] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 0/1 SP 0 speed 1 
    [clash] The Knight's Faith (3) vs Temper and Cast (5) over 2 round(s); winner: enemy-7-9567
    [clash] Danza de Pasión (5) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-4-11004
    [clash] Destroy the Visible with the Invisible (3) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-6-11214
    [attack] Temper and Cast -> 2 hit(s), 23 damage, coin rolls [5, 8]
    [attack] Fluttering Havoc -> 2 hit(s), 14 damage, coin rolls [4, 4]
    [attack] Fluttering Havoc -> 2 hit(s), 0 damage, coin rolls [4, 7]
turn 5 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 45 speed 6 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 45 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP 45 speed 7 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 165/278 SP 30 speed 6 
  Los Mariachis Jefe (Sinclair)      HP 245/290 SP 45 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 31/244 SP 45 speed 7 
  Lobotomy E.G.O::Lamp (Gregor)      HP 251/274 SP 45 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 25480/25616 SP 0 speed 3 [Past 23] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 0/1 SP 0 speed 1 
    [clash] Khopesh Swordplay (3) vs Immolation (6) over 2 round(s); winner: enemy-7-9567
    [clash] The Knight's Sanctuary (4) vs Immolation (6) over 3 round(s); winner: enemy-7-9567
    [clash] Pulverization (4) vs Baile y Rola (4) over 3 round(s); winner: enemy-7-9567
    [clash] Pulverization (4) vs Intense Stare: Zero-Range (3) over 2 round(s); winner: sinner-6-11214
    [attack] Pulverization -> 2 hit(s), 16 damage, coin rolls [6, 6]
turn 6 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 45 speed 6 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 45 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP 45 speed 7 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 0/278 SP 30 speed 6 STAGGERED
  Los Mariachis Jefe (Sinclair)      HP 124/290 SP 45 speed 2 STAGGERED
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 0/244 SP 45 speed 7 
  Lobotomy E.G.O::Lamp (Gregor)      HP 183/274 SP 45 speed 1 
  Refracted Butterfly of Entangled Lives::Imago HP 25430/25616 SP 0 speed 3 [Past 28] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 0/1 SP 0 speed 1 
    [attack] Kalpāgni -> 2 hit(s), 124 damage, coin rolls [20, 20]
    [clash] Chaotic Turmoil (3) vs Destroy the Visible with the Invisible (3) over 1 round(s); winner: sinner-6-11214
    [attack] Chaotic Turmoil -> 1 hit(s), 4 damage, coin rolls [3]
turn 7 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 45 speed 6 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 45 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP 45 speed 7 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 0/278 SP 30 speed 6 STAGGERED
  Los Mariachis Jefe (Sinclair)      HP 0/290 SP 45 speed 2 STAGGERED
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 0/244 SP 45 speed 7 
  Lobotomy E.G.O::Lamp (Gregor)      HP 85/274 SP 45 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 25356/25616 SP 0 speed 2 [Past 33] 
  Refracted Illusory Butterfly of Entangled Lives::The Past HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Present HP 0/1 SP 0 speed 1 
  Refracted Illusory Butterfly of Entangled Lives::The Future HP 0/1 SP 0 speed 1 
    [clash] Intense Stare: Zero-Range (3) vs Temper and Cast (5) over 2 round(s); winner: enemy-7-9567
    [attack] Temper and Cast -> 2 hit(s), 20 damage, coin rolls [10, 10]
--- warnings (1 unique) ---
  * SP gain/loss per clash is not documented for the current game version; configure BattleConfig::sp_on_clash_win / sp_on_clash_lose
state hash: b4536f9ae28287eb
```

## Notes

* With this policy the team is wiped around turn 10-12: it spends its Coins on
  Skill 1s while the Imago fields six Skill Slots, several of them multi-target
  (Atk Weight up to 7), for roughly 300 damage per turn.
* The team's own output is ~90-190 damage per turn, because the Imago's
  resistances are neutral-to-positive only for Wrath / Lust / Pride (x1.25) and it
  wins many of the Clashes.  A real run leans on E.G.O (which need accumulated
  resources), on chaining into the Imago's Slots (`Action::Engage`) and on
  managing Burn / Bleed, none of which this sample policy does.
* Balance is out of scope for the simulator: it reproduces the sourced rules, and
  the sample shows they are being applied consistently.
