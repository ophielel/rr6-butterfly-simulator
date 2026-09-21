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
  at level 60 with **45 SP** (`BattleConfig::starting_sp`), which is what makes
  their Coin flips land Heads more often;
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
    [clash] The Knight's Sanctuary (4) vs Temper and Cast (5) over 2 round(s); winner: sinner-3-10913
    [clash] Solemn Lament for the Living (4) vs Temper and Cast (5) over 2 round(s); winner: sinner-0-10110
    [clash] Khopesh Swordplay (3) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-5-11114
    [clash] Bang. Bang. (4) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-1-10414
    [clash] Destroy the Visible with the Invisible (3) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-6-11214
    [discard] Discarded 1081303
    [unopposed] Jeong's Office Rep (Ishmael) attacked with Kōzan [光斬]
    [clash] Tsuru-giri [鶴斬り] (4) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-2-10813
    [attack] Danza de Pasión -> 3 hit(s), 21 damage, coin rolls [6, 7, 8]
turn 2 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 200/202 SP 45 speed 8 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 203/205 SP 45 speed 5 
  Jeong's Office Rep (Ishmael)       HP 222/224 SP 45 speed 4 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 276/278 SP 45 speed 4 
  Los Mariachis Jefe (Sinclair)      HP 287/290 SP 45 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 242/244 SP 45 speed 5 
  Lobotomy E.G.O::Lamp (Gregor)      HP 270/274 SP 45 speed 3 
  Refracted Butterfly of Entangled Lives::Imago HP 25397/25616 SP 0 speed 3 [Past 10] 
    [clash] Celebration for the Departed (4) vs Immolation (6) over 3 round(s); winner: sinner-0-10110
    [butterfly] Butterfly dealt 1 Gloom damage (1 x 2 The Departed)
    [butterfly] Butterfly dealt 1 Gloom damage (1 x 2 The Departed)
    [butterfly] Butterfly dealt 1 Gloom damage (1 x 2 The Departed)
    [clash] Blossoming Fragrance in the Void (4) vs Pulverization (4) over 2 round(s); winner: sinner-1-10414
    [clash] Khopesh Swordplay (3) vs Immolation (6) over 2 round(s); winner: enemy-7-9567
    [discard] Discarded 1081301
    [butterfly] Butterfly dealt 1 Gloom damage (1 x 2 The Departed)
    [butterfly] Butterfly dealt 1 Gloom damage (1 x 2 The Departed)
    [clash] Tsuru-giri [鶴斬り] (4) vs Pulverization (4) over 2 round(s); winner: sinner-2-10813
    [butterfly] Butterfly dealt 1 Gloom damage (1 x 2 The Departed)
    [clash] With the Power of Justice (4) vs Pulverization (4) over 2 round(s); winner: sinner-3-10913
    [clash] Intense Stare: Zero-Range (3) vs Pulverization (4) over 2 round(s); winner: sinner-6-11214
    [attack] Baile y Rola -> 2 hit(s), 12 damage, coin rolls [6, 8]
turn 3 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 143/202 SP 45 speed 8 
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 140/205 SP 45 speed 3 
  Jeong's Office Rep (Ishmael)       HP 218/224 SP 45 speed 3 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 271/278 SP 45 speed 4 
  Los Mariachis Jefe (Sinclair)      HP 282/290 SP 45 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 183/244 SP 45 speed 7 
  Lobotomy E.G.O::Lamp (Gregor)      HP 264/274 SP 45 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 25105/25616 SP 0 speed 1 [Past 10] 
    [clash] Solemn Lament for the Living (4) vs Chaotic Turmoil (3) over 1 round(s); winner: sinner-0-10110
    [clash] Khopesh Swordplay (3) vs Kalpāgni (10) over 2 round(s); winner: enemy-7-9567
    [clash] The Knight's Faith (3) vs Chaotic Turmoil (3) over 1 round(s); winner: sinner-3-10913
    [attack] Danza de Pasión -> 3 hit(s), 21 damage, coin rolls [6, 7, 8]
    [attack] Intense Stare: Zero-Range -> 2 hit(s), 26 damage, coin rolls [7, 11]
turn 4 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 38 speed 8 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 38 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 114/224 SP 38 speed 3 STAGGERED
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 233/278 SP 24 speed 6 
  Los Mariachis Jefe (Sinclair)      HP 175/290 SP 41 speed 2 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 71/244 SP 38 speed 4 STAGGERED
  Lobotomy E.G.O::Lamp (Gregor)      HP 170/274 SP 40 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 25000/25616 SP 0 speed 1 [Past 15] 
    [clash] Arcana Pierce (5) vs Temper and Cast (5) over 2 round(s); winner: sinner-3-10913
    [clash] Baile y Rola (4) vs Fluttering Havoc (4) over 2 round(s); winner: enemy-7-9567
    [clash] Intense Stare: Zero-Range (3) vs Fluttering Havoc (4) over 2 round(s); winner: sinner-6-11214
    [attack] Temper and Cast -> 2 hit(s), 38 damage, coin rolls [8, 11]
    [attack] Fluttering Havoc -> 2 hit(s), 41 damage, coin rolls [7, 10]
    [attack] Fluttering Havoc -> 2 hit(s), 17 damage, coin rolls [4, 4]
turn 5 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 38 speed 8 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 38 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 58/224 SP 38 speed 8 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 218/278 SP 24 speed 6 
  Los Mariachis Jefe (Sinclair)      HP 148/290 SP 41 speed 3 
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 0/244 SP 38 speed 4 STAGGERED
  Lobotomy E.G.O::Lamp (Gregor)      HP 154/274 SP 40 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 24774/25616 SP 0 speed 1 [Past 15] 
    [discard] Discarded 1081302
    [clash] Susuki Misdirection (4) vs Immolation (6) over 3 round(s); winner: sinner-2-10813
    [clash] The Knight's Sanctuary (4) vs Immolation (6) over 2 round(s); winner: enemy-7-9567
    [clash] Destroy the Visible with the Invisible (3) vs Pulverization (4) over 2 round(s); winner: sinner-6-11214
    [attack] Pulverization -> 2 hit(s), 23 damage, coin rolls [7, 7]
    [attack] Pulverization -> 2 hit(s), 4 damage, coin rolls [4, 4]
turn 6 (phase AwaitingActions)
  Lobotomy E.G.O:: Solemn Lament (Yi Sang) HP 0/202 SP 38 speed 8 STAGGERED
  Lobotomy E.G.O:: Faint Aroma & Solitude (Ryōshū) HP 0/205 SP 38 speed 3 STAGGERED
  Jeong's Office Rep (Ishmael)       HP 0/224 SP 43 speed 8 
  Lobotomy E.G.O:: The Sword Sharpened with Tears (Rodion) HP 118/278 SP 24 speed 6 
  Los Mariachis Jefe (Sinclair)      HP 0/290 SP 41 speed 3 STAGGERED
  LCA Udjat Vanguard Team 3 Leader (Outis) HP 0/244 SP 38 speed 4 STAGGERED
  Lobotomy E.G.O::Lamp (Gregor)      HP 134/274 SP 40 speed 2 
  Refracted Butterfly of Entangled Lives::Imago HP 24652/25616 SP 0 speed 2 [Past 15] 
    [clash] The Knight's Sanctuary (4) vs Kalpāgni (10) over 2 round(s); winner: enemy-7-9567
--- warnings (1 unique) ---
  * SP gain/loss per clash is not documented for the current game version; configure BattleConfig::sp_on_clash_win / sp_on_clash_lose
state hash: 9738557972df2b6d
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
