# 成绩改进实验：从奖励系数转回学习信号、数据与算力

> 状态：**完成**。本文记录本轮针对 full-HP holdout 胜率所做的诊断、改动、协议和
> 结果，包括一个决定性的负结果：在 46-47% 这个水平上，**critic warm-up、熵正则、
> 演示数据扩充和把训练预算放大 10 倍都没有产生超出噪声的胜率变化**。
>
> 基线：`models/ppo_plan_E_suffix_trigger001.npz`
> （历史记录 `reports/plan_E_full_eval/real_PPO.jsonl` = `241/500` on `43001-43500`）。
> 所有新评测使用本仓库此前从未使用过的 seed band。

## 0. 结论先行

1. **本轮唯一确定有效的改动是工程性的**：候选效果探测（`probe_action_effects`）
   对所有现存 checkpoint 都是纯浪费——它每个候选解析一个完整回合克隆，而这些
   checkpoint 的效果权重行**恒为零**，探测结果永远不可能改变任何一个决策。加上
   `PolicyValueNet.uses_action_effects()` 后，训练和评测都自动跳过探测，
   **同一 checkpoint 的同一批 seed 结果逐位不变，耗时从 266.7 s / 20 seeds 降到
   8.0 s / 20 seeds（33 倍）**，单个训练臂从 `> 60 分钟` 降到 `3.8 分钟`。
2. **四个配对臂在全新 holdout band 上全部落在噪声内**：`235/500`（基线）、
   `230`、`229`、`229`、`228`，配对净增 `-5 ~ -7`，精确 McNemar `p = 0.43 ~ 0.62`。
3. **把 PPO 训练预算放大 10 倍（500 → 5000 episodes）也只得到 `237/500`**，
   配对净增 `+2`，`p = 0.89`。这条最重要：它说明当前分数**不是采样不足**。
4. **两个策略在 63.4% 的回合上给出不同的完整 plan，胜率却相同**
   （`turn_agreement = 0.367`，`actor_agreement = 0.929`）。也就是说这个任务在
   当前模拟器、当前阵容、`infinite_ego_resources=true`、30 回合、`real` HP 条件下，
   存在一个**很宽的 46-47% 平台**：很多不同的打法得到同样的结果。
5. 因此本轮**没有**把成绩提高。下面把"为什么"和"要真正提高需要什么"写清楚，
   而不是再换一个奖励系数把数字抬上去。

## 1. 诊断：分数到底卡在哪

用 `reports/plan_E_full_eval/real_PPO.jsonl`（500 seeds, real HP）做失败归因：

| 指标 | 胜局 (241) | 负局 (259) |
|---|---:|---:|
| 平均对敌伤害 | 8131 | 7003 |
| 平均存活回合 | 8.50 | 9.32 |
| 平均队友存活 | 4.15 | 0.52 |
| 平均 E.G.O 次数 | 6.00 | 5.94 |
| 平均 Sinking 伤害 | 17104 | 15287 |

负局里 `Enemies` 团灭 155 局、超时 104 局，击杀回合分布 `7/8/9/10`。胜负几乎完全
由"能否在第 8-9 回合前把 25 616 HP 打完"决定，而 E.G.O 使用次数在胜负局之间没有
差别。**提高 E.G.O 频率、继续加奖励项都不是瓶颈**：胜负局的平均伤害只差 16%，
而胜局需要 8200 以上的总伤害。

### 1.1 seed band 的绝对难度差异极大

| checkpoint | band | wins |
|---|---|---:|
| plan E | `43001-43500`（历史） | 241/500 |
| plan E | `600001-600500`（本轮） | 235/500 |
| plan E | `43001-43200` | 104/200 |
| plan E | `200001-200200` | 90/200 |
| plan E | `150001-150200`（计划 E 的训练 band） | 200/200 |
| strong201 | `600001-600060` | 23/60 |

同一 checkpoint 在不同 band 上从 39% 到 100%。所以：

> **绝对胜率不能跨 band 比较**；所有结论必须写成"同一 band 上的配对净增"。
> 本轮的每一步都用同一 band、同一批 seed、同一奖励做配对比较。

## 2. 改动清单

### 2.1 有效：跳过无效的效果探测（`python/lcb/nn.py`, `lcb/ppo.py`, `lcb/baselines.py`）

`PolicyValueNet.uses_action_effects()` 检查 `W2` 最后 `EFFECT_DIM=22` 行是否全零。
本仓库所有 checkpoint 都是 `action_dim=44` 训练的，加载到 66 维时这 22 行由
`load()` 零填充且从未被训练，因此 `probe_action_effects` 的全部工作都不可能改变
决策。实测：

```text
evaluate_argmax, 20 seeds, real, action_effects=true
改动前 266.7 s     改动后 8.0 s     胜率同为 0.45，kill turn 同为 8.0
单臂训练 10x50 episodes：>60 min  ->  3.8 min
```

这不是"调参"：跳过探测后的 plan 与探测后**逐位相同**（因为那 22 行权重为零），
有 `test_effect_probe_is_skipped_for_checkpoints_that_cannot_use_it` 回归测试覆盖。

### 2.2 无效但保留：critic warm-up 与熵正则

* `PPOConfig.value_warmup_epochs`（默认 2）：在算 advantage 之前先用本轮 return
  训练 value head。BC 不训练 value head，所以历史代码里
  `advantage = return - V(s)` 中的 `V` 一直是随机初始化的，实测 `ctrl` 臂
  （`value_warmup_epochs=0`）的 `value_warmup_loss` 全为 `0.0`，而 `algo` 臂为
  `0.26-0.55`，说明改动确实生效。
* `PPOConfig.entropy_coef`：候选分布熵奖励，梯度 `dH/dlogits = -p(log p + H)`。
  实测不带熵奖励时熵从 `4.85` 掉到 `3.46`，加上 `0.01` 后稳定在 `3.5-4.2`，
  即它确实阻止了动作分布塌缩。
* 两者对 holdout 胜率的配对净增分别是 `-6`（`p=0.52`）和 `-7`（`p=0.43`）。

保留它们的理由是它们修的是**真实的算法缺陷**（未经训练的 critic 被用于
advantage；零探索项），而不是因为它们提分；默认值对历史复现命令是行为变更，
已在 `docs/TRAINING.md` 与本表中标注。

### 2.3 工程修复（不改变语义）

* `tools/eval_checkpoints.py`：配对评测工具（同一 band、Wilson CI、精确 McNemar）。
* `tools/action_agreement.py`：两个 checkpoint 的逐动作一致性。
* `tools/report_critic_ablation.py`：把训练诊断和配对结果汇总成一张表。
* `iter_decisions`：候选切片改为按行自己的 `offsets` 前缀和取值，不再依赖
  "文件行序恰好等于 decision 排序"这一未声明的假设（当前数据恰好满足，输出不变）。
* `test_training.py`：临时目录改为仓库内 `.tmp/` 并加入 `--value-*`/熵/probe 回归测试。
* `probe_action_effects`：不再为每次探测重建完整 beam。

## 3. 协议

四个配对臂，同一初始 checkpoint、同一训练 seed、同一奖励、同一评测 band：

| 因子 | 取值 |
|---|---|
| `algo` | `value_warmup_epochs=2, entropy_coef=0.01` vs 历史 `0, 0.0` |
| `data` | 加入 `data/suffix_teacher_strong_50` vs 只用通用 full 搜索数据 |

```text
初始 checkpoint  models/ppo_plan_E_suffix_trigger001.npz
训练 seeds       150001-150500
最终 holdout     600001-600500   (本仓库此前从未使用)
额外复核 band    610001-610200、620001-620200
scenario         real (enemy_hp_scale=1.0), strict=true,
                 infinite_ego_resources=true, max_turns=30
PPO              10 iterations x 50 episodes, 3 epochs/iteration,
                 lr=1e-4, demo 10 updates/iteration
评测             argmax, 同一批 seed, 配对 McNemar
```

### 3.1 训练期 validation 被关闭（协议说明）

`action_effects=true` 时（改动前）每个验证决策都要为每个候选做一次完整
`step_turn` 克隆：实测 `evaluate_argmax` 在 10 个 seed 上需要 186.8 秒
（18.7 秒/seed），100 seed 的验证带每轮就是 **31 分钟**，10 轮共 5 小时/臂。

同时历史 100-seed 验证曲线并不可靠：`fast6` 那次在 iteration 2 达到 `0.49` 后掉到
`0.16`，而 `plan_E` 是单调升到第 10 轮的 `0.46`；两轮 run 的"验证最优"分别落在
第 2 轮和第 10 轮。因此本轮把训练期 validation 关闭（`--val-seed-count 0`），
四个臂一律取最后一轮，再用同一个从未使用过的 band 做配对比较。

## 4. 结果

`reports/paired_critic_warmup_500.json`、`reports/paired_scale_500.json`、
`reports/paired_second_band_200.json`、`reports/paired_third_band_200.json`。

### 4.1 主表：holdout `600001-600500`（500 seeds，real HP）

| 臂 | warmup | entropy | suffix | rollout 训练胜率(末轮) | holdout wins | 胜率 | 95% CI | 配对净增 vs planE | McNemar p |
|---|---:|---:|---|---:|---:|---:|---|---:|---:|
| plan E（基线） | 历史 | 历史 | 有 | 0.28 | **235/500** | 0.470 | 0.426-0.514 | — | — |
| ctrl | 0 | 0.0 | 无 | 0.30 | 230/500 | 0.460 | 0.416-0.504 | -5 | 0.620 |
| algo | 2 | 0.01 | 无 | 0.12 | 229/500 | 0.458 | 0.414-0.502 | -6 | 0.519 |
| data | 0 | 0.0 | 有 | 0.24 | 229/500 | 0.458 | 0.414-0.502 | -6 | 0.532 |
| algo_data | 2 | 0.01 | 有 | — | 228/500 | 0.456 | 0.412-0.500 | -7 | 0.435 |
| scale5000 | 2 | 0.01 | 有 | 0.34 | **237/500** | 0.474 | 0.430-0.518 | **+2** | **0.892** |

`scale5000` 用 `170001-175000`（5000 episodes, 20 iterations）训练，是其余臂
10 倍的 rollout 预算。所有臂的 median kill turn 都是 8.0。

### 4.2 复核 band（确认不是单个 band 的偶然）

| band | plan E | algo | algo_data | scale5000 | 配对净增 |
|---|---:|---:|---:|---:|---|
| `610001-610200` | 102/200 (0.510) | 103/200 (0.515) | — | — | +1, p=1.000 |
| `620001-620200` | 92/200 (0.460) | — | 95/200 (0.475) | 96/200 (0.480) | +3 (p=0.65) / +4 (p=0.48) |

三个 band 的结论一致：没有超出噪声的差异。

### 4.3 训练诊断（证明改动确实作用于学习过程）

| 臂 | warmup_loss | 熵轨迹 | 说明 |
|---|---|---|---|
| ctrl | `0.0 × 10` | `4.85 → 3.46` | critic 完全没在算 advantage 之前训练；策略熵塌缩 |
| algo | `0.26-0.55` | `3.5-4.2` | 两项改动都生效，但胜率无变化 |
| data | `0.0 × 10` | `3.05-5.32` | 只是数据变化 |
| scale5000 | 有 | 有 | 10 倍预算 |

### 4.4 行为差异（为什么"没有提高"是真的平台，不是没训练）

`tools/action_agreement.py`，80 seeds / 709 回合：

```text
plan E vs algo     完整 plan 一致率 0.367    逐 actor 一致率 0.929
```

63% 的回合给出了不同的完整 plan，胜率却相同。结合 `scale5000` 的 10 倍预算无效，
这说明当前不是"策略还没学好"，而是**这个场景在当前条件下存在一个很宽的 46-47%
结果平台**：换打法不改变结果。

## 5. 探索性结论（不进主表）

### 5.1 带策略先验的 lookahead 搜索

新增 `NeuralLookaheadPolicy`（`python/lcb/baselines.py`）：候选完整 plan 来自网络
自己在合法动作集上的 top-k beam，用真实 `step_turn` 克隆评估
`reward + horizon 内 rollout + V(s)`，plan 数上限是显式常量（不同于
`GreedyPolicy` 随 actor 数增长的代价）。它保留了网络原本会选的 plan 作为候选之一，
所以只可能在搜索主动偏好别的合法 plan 时改变决策。

在 `beam=4, branch=2, horizon=1` 上实测每局 55-370 秒，4 个样本里 3 胜；对照
纯 argmax 约 2-3 秒。这个代价无法支撑 500 seed 的统计结论，因此**只作为工具保留，
不作为本轮成绩改进**。修好 §2.1 之后它的成本也随之下降，但本轮没有重新采集样本量
足够的对照，所以不写进主表。

### 5.2 不采纳的方向

* 继续调 reward 系数（含 `sinking_trigger_reward`）：历史已经证明它只改善既有爆发
  策略的终局 credit，`axis_ok_rate` 始终为 0，属于"硬堆奖励"。
* 直接加大搜索预算的 Teacher：`reports/teacher_search_diagnostics.json` 显示
  full-HP Teacher 在 50 seed 上最多 3 胜，无法支撑新一轮 BC。

## 6. 要真正提高成绩，下一步应该做什么

本轮的负结果把可能性收窄了。既然算法、探索项、演示数据和 10 倍算力都不动，
剩下的可能只有两条，且都需要先改**输入**而不是改训练：

1. **给动作特征加上技能身份。** `lcb.features` 的动作编码目前只有数值属性
   （Base/Coin Power、罪孽、伤害类型、防御/E.G.O 标记、目标信息），**没有技能 ID**。
   固定阵容的合法动作里只有 30 个不同技能/E.G.O 字符串，两个数值相同但效果不同的
   技能在网络看来完全一样。加一个固定宽度的 skill-slot 指示（并把
   `action_dim` 加宽、旧 checkpoint 用零填充加载）可以让策略区分它们。这需要重训
   BC + PPO，是本轮之后最明确的方向。
2. **换一个不会在 9 回合内团灭的场景条件。** 104/259 的负局是超时、155/259 是团灭，
   队伍能造成的总伤害决定了上限；`infinite_ego_resources=true` 已经去掉了资源约束，
   真正的约束是**存活回合数**。在不改游戏规则的前提下，能改的只有显式场景旋钮
   （`enemy_hp_scale`）或阵容，而这两者都属于实验条件变更，必须像本轮一样单独标注。

同时，任何后续实验都应该沿用本轮的配对协议（同 band、配对 McNemar、至少 500 seeds
或在多个 band 上复核），因为 100-seed 级别的差异在本任务上完全可能是噪声。

