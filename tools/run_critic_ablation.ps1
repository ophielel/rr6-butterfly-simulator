#!/usr/bin/env pwsh
# Matched A/B(/C/D) PPO arms for the critic-warmup + rollout-suffix experiment.
#
# Every arm starts from the same checkpoint, uses the same training seeds, the
# same PPO random seed, the same iteration/rollout budget, the same reward and
# the same final evaluation band.  The only differences are the two factors:
#
#   algo   : --value-warmup-epochs 2 --entropy-coef 0.01   vs the historical 0/0
#   data   : the suffix Teacher set added to the generic full-search demos
#
# Protocol note: training-time validation is switched **off** (`--val-seed-count
# 0`).  The 100-seed validation curve was noisy enough that "best by validation"
# peaked at iteration 1-2 in two of the recorded runs, while `plan_E` improved
# monotonically to the last iteration.  Checkpoint selection is therefore
# replaced by the paired holdout comparison below, applied identically to every
# arm (final iteration).
#
# `--action-effects` is passed for every arm.  It costs nothing here: none of the
# checkpoints in this repository has a trained effect block, so the probes are
# skipped automatically (`PolicyValueNet.uses_action_effects`).
#
# The holdout band (600001+) has never been used by any earlier run in this
# repository, so the final comparison is not a reused band.
#
# Run directly as a background command (no `pwsh -File` wrapper and no
# `Tee-Object`): an extra pipe in the chain keeps the child's stdout block
# buffered and stalls the trainer.
$ErrorActionPreference = "Stop"
$py = "C:\Users\24790\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe"
Set-Location "C:\Users\24790\Desktop\butterfly"

$base = "models/ppo_plan_E_suffix_trigger001.npz"
$demoGeneric = "data/diagnostic_ablation_full_bare_quality"
$demoSuffix = "data/suffix_teacher_strong_50"

$arms = @(
    @{ name = "ctrl";      warmup = 0; entropy = 0.0;  suffix = $false },
    @{ name = "algo";      warmup = 2; entropy = 0.01; suffix = $false },
    @{ name = "data";      warmup = 0; entropy = 0.0;  suffix = $true  },
    @{ name = "algo_data"; warmup = 2; entropy = 0.01; suffix = $true  }
)

foreach ($arm in $arms) {
    $out = "models/exp_critic_$($arm.name).npz"
    if (Test-Path $out) {
        Write-Host "skip $out (exists)"
        continue
    }
    $demo = @($demoGeneric)
    if ($arm.suffix) { $demo += $demoSuffix }
    Write-Host "=== training $($arm.name): warmup=$($arm.warmup) entropy=$($arm.entropy) suffix=$($arm.suffix)"
    Write-Host "=== started $(Get-Date -Format u)"
    & $py training/train_ppo.py `
        --checkpoint $base `
        --out $out `
        --scenario real `
        --seed-start 150001 --seed-count 500 `
        --iterations 10 --episodes-per-iteration 50 --epochs-per-iteration 3 `
        --learning-rate 0.0001 `
        --seed 20261601 `
        --val-seed-start 151001 --val-seed-count 0 `
        --allow-non-bc-checkpoint `
        --action-effects `
        --value-warmup-epochs $arm.warmup `
        --entropy-coef $arm.entropy `
        --demo-data @demo `
        --demo-updates-per-iteration 10 --demo-batch-decisions 32
    if ($LASTEXITCODE -ne 0) { throw "arm $($arm.name) failed" }
    Write-Host "=== finished $(Get-Date -Format u)"
}

Write-Host "=== final paired evaluation on the fresh holdout 600001-600500"
& $py tools/eval_checkpoints.py --scenario real `
    --seed-start 600001 --seed-count 500 --reference planE `
    --action-effects `
    --checkpoint "planE=$base" `
    --checkpoint "ctrl=models/exp_critic_ctrl.npz" `
    --checkpoint "algo=models/exp_critic_algo.npz" `
    --checkpoint "data=models/exp_critic_data.npz" `
    --checkpoint "algo_data=models/exp_critic_algo_data.npz" `
    --out reports/paired_critic_warmup_500.json
Write-Host "=== done $(Get-Date -Format u)"
