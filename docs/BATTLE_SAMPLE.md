# Sample battle: 7 Sinners vs Butterfly of Entangled Lives::Imago

A sanity playthrough of the fixed content, produced by

```text
cargo run -p lcb-cli -- run --seed 3 --turns 12
```

which plays `setup::fixed::TEAM` (the seven identities the project models) against
`setup::fixed::BOSS_IMAGO` - the 25616 HP Imago, no Pupa and no illusory
butterflies - with a deliberately simple policy: every unit attacks the lowest-HP
enemy with whatever the Dashboard offers, no E.G.O and no defense.

## What to look for

* the Imago starts at 25616 HP (`9090 + 275.44 x 60` at level 60) and the Sinners
  at level 60;
* Clashes are resolved with summed Final Power, and the log names the winner;
* `[In the Past] / [In the Present] / [In the Future]` stacks drive the Imago's
  Turn Start passives (the Past passive burns whoever hits it, the Future passive
  hands out Bleed Count);
* the unique Sinking ([Butterfly]) appears as `[butterfly]` lines, Petals and the
  Kōzan follow-up as `[unopposed]`;
* no unhandled effect kinds and no crash; the only warning is the documented
  SP-on-Clash unknown.

## Damage accounting

Measured on `--seed 7` (the same policy), the team loses its whole 1717 HP pool
over five turns; the per-hit log accounts for 1574 of it and the Turn End Burn
ticks for the remaining ~143 (~29 HP per turn, which is exactly the Burn Potency
the team accumulates), i.e. every point of damage is attributable to a rule the
simulator names.  The Imago ends this seed at 25179/25616.

## Transcript

```text
turn 1 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 202/202 SP 0 speed 6 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 205/205 SP 0 speed 5 
  Jeong's Office Rep (Ishmael)       HP 224/224 SP 0 speed 3 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 278/278 SP 0 speed 7 
  Los Mariachis Jefe (Sinclair)      HP 290/290 SP 0 speed 3 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 244/244 SP 0 speed 6 
  Lobotomy E.G.O::Lamp (Gregor)      HP 274/274 SP 0 speed 4 
  Refracted Butterfly of Entangled Lives::Imago HP 25616/25616 SP 0 speed 1 [Past 10] 
    [clash] The Knight's Sanctuary (4) vs Temper and Cast (5) over 3 round(s); winner: enemy-7-9567
    [clash] Solemn Lament for the Living (4) vs Temper and Cast (5) over 3 round(s); winner: enemy-7-9567
    [clash] Khopesh Swordplay (3) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-5-11114
    [clash] Bang. Bang. (4) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-1-10414
    [clash] Destroy the Visible with the Invisible (3) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-6-11214
    [discard] Discarded 1081303
    [unopposed] Jeong's Office Rep (Ishmael) attacked with Kōzan [光斬]
    [clash] Tsuru-giri [鶴斬り] (4) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-2-10813
    [attack] Danza de Pasión -> 3 hit(s), 19 damage, coin rolls [6, 6, 7]
turn 2 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 195/202 SP 0 speed 4 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 203/205 SP 0 speed 7 
  Jeong's Office Rep (Ishmael)       HP 222/224 SP 5 speed 8 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 272/278 SP 0 speed 6 
  Los Mariachis Jefe (Sinclair)      HP 287/290 SP 0 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 242/244 SP 0 speed 4 
  Lobotomy E.G.O::Lamp (Gregor)      HP 270/274 SP 0 speed 1 
  Refracted Butterfly of Entangled Lives::Imago HP 25503/25616 SP 0 speed 2 [Past 10] 
    [discard] Discarded 1081302
    [clash] Susuki Misdirection (4) vs Immolation (6) over 4 round(s); winner: enemy-7-9567
    [clash] Blossoming Fragrance in the Void (4) vs Pulverization (4) over 2 round(s); winner: sinner-1-10414
    [clash] With the Power of Justice (4) vs Pulverization (4) over 3 round(s); winner: enemy-7-9567
    [clash] Celebration for the Departed (4) vs Pulverization (4) over 3 round(s); winner: enemy-7-9567
    [clash] Khopesh Swordplay (3) vs Immolation (6) over 2 round(s); winner: enemy-7-9567
    [clash] Baile y Rola (4) vs Pulverization (4) over 2 round(s); winner: sinner-4-11004
    [attack] Intense Stare: Zero-Range -> 2 hit(s), 35 damage, coin rolls [9, 15]
turn 3 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 115/202 SP 0 speed 4 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 120/205 SP 0 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 180/224 SP 5 speed 4 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 259/278 SP 0 speed 5 
  Los Mariachis Jefe (Sinclair)      HP 282/290 SP 0 speed 3 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 195/244 SP 0 speed 6 
  Lobotomy E.G.O::Lamp (Gregor)      HP 264/274 SP 0 speed 1 
  Refracted Butterfly of Entangled Lives::Imago HP 25376/25616 SP 0 speed 3 [Past 10] 
    [clash] Khopesh Swordplay (3) vs Kalpāgni (10) over 2 round(s); winner: enemy-7-9567
    [clash] The Knight's Faith (3) vs Chaotic Turmoil (3) over 1 round(s); winner: sinner-3-10913
    [attack] Baile y Rola -> 2 hit(s), 6 damage, coin rolls [4, 4]
    [attack] Intense Stare: Zero-Range -> 2 hit(s), 8 damage, coin rolls [3, 3]
turn 4 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 0 speed 4 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 0 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP -2 speed 4 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 147/278 SP -22 speed 4 
  Los Mariachis Jefe (Sinclair)      HP 133/290 SP -7 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 11/244 SP -7 speed 4 STAGGERED
  Lobotomy E.G.O::Lamp (Gregor)      HP 140/274 SP -7 speed 1 
  Refracted Butterfly of Entangled Lives::Imago HP 25335/25616 SP 0 speed 2 [Past 15] 
    [clash] With the Tear-sharpened Sword (16) vs Temper and Cast (5) over 2 round(s); winner: sinner-3-10913
    [clash] Danza de Pasión (5) vs Fluttering Havoc (4) over 3 round(s); winner: enemy-7-9567
    [attack] Temper and Cast -> 2 hit(s), 11 damage, coin rolls [5, 5]
    [clash] Fluttering Havoc (4) vs Destroy the Visible with the Invisible (3) over 3 round(s); winner: sinner-6-11214
    [attack] Fluttering Havoc -> 2 hit(s), 6 damage, coin rolls [4, 7]
turn 5 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 0 speed 4 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 0 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP -2 speed 4 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 123/278 SP -32 speed 4 
  Los Mariachis Jefe (Sinclair)      HP 88/290 SP -7 speed 3 STAGGERED
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 0/244 SP -7 speed 4 STAGGERED
  Lobotomy E.G.O::Lamp (Gregor)      HP 123/274 SP -7 speed 1 
  Refracted Butterfly of Entangled Lives::Imago HP 25215/25616 SP 0 speed 1 [Past 15] 
    [clash] Weathered Pride (12) vs Immolation (6) over 2 round(s); winner: enemy-7-9567
    [attack] Pulverization -> 2 hit(s), 14 damage, coin rolls [4, 4]
    [attack] Pulverization -> 2 hit(s), 17 damage, coin rolls [7, 7]
    [attack] Pulverization -> 2 hit(s), 19 damage, coin rolls [7, 7]
--- warnings (1 unique) ---
  * SP gain/loss per clash is not documented for the current game version; configure BattleConfig::sp_on_clash_win / sp_on_clash_lose
state hash: bf3b6a16f4b697eb
```

## Notes

* With this policy the team loses on turn 5: it spends its Coins on Skill 1s
  while the Imago fields six Skill Slots, several of them multi-target (Atk Weight
  up to 7), for roughly 300 damage per turn.
* The team's own output is ~90-190 damage per turn, because the Imago's
  resistances are neutral-to-positive only for Wrath / Lust / Pride (x1.25) and it
  wins many of the Clashes.  A real run leans on E.G.O (which need accumulated
  resources), on chaining into the Imago's Slots (`Action::Engage`) and on
  managing Burn / Bleed, none of which this sample policy does.
* Balance is out of scope for the simulator: it reproduces the sourced rules, and
  the sample shows they are being applied consistently.
