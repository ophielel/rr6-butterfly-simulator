# 训练与评测结果 (TRAINING_RESULTS.md)

对 `docs/TRAINING_PLAN.md` 的实现结果。**结论先行**，然后是数字与对照。

> **Legacy notice:** Sections 0–8 below record the pre-audit run and must not be mixed with the post-audit artifacts. The corrected run is summarized in `reports/post_audit_summary.json` and uses `data/teacher_post_audit_sweep/`, `models/*_post_audit.*`, and `reports/post_audit_*`.

## 0. 结论

1. **短轴存在，而且被搜索找到了。** 阶段 A 的回合级树搜索（`search/run_teacher.py`）
   在测试带上 **64% 胜率、P25 击杀回合 3、短胜率（≤4 回合）59%**，全面优于
   `Greedy` 基线（61% / P25 4 / 51%），并在 **31% 的胜局**里满足
   "Sinking 建立 → Solemn Lament/Harmony → 触发伤害升高" 的轴事件判定；
   best-of-10 重开窗口胜率 100%。
2. **把这条轴蒸馏进网络没有成功。** BC（教师 400 局 + DAgger 一轮 96 局）在
   测试带上 **23% 胜率**，PPO 微调 400 局后没有提升（验证带 17%-30%）。它学到了轴
   （**57% 的胜局**带轴事件，比 Greedy 的 20% 高），但没有学到"活到能收割"。
   §7.1 的验收条款 2/3/4 对 **AI 策略**不成立；对 **搜索教师**成立 2 和 4，不成立 3
   （中位击杀回合与 Greedy 同为 4，未达 15% 改善）。
3. **真实 HP 场景这一波打不赢。** 不打开场景旋钮时（Imago 25616 HP、30 回合），
   四策略胜率全为 0，队伍约 630-800 伤害/局并在 9 回合左右团灭。因此
   `kill_turn`/短胜率/best-of-N 只在 `burst` 场景（`enemy_hp_scale=0.08`，
   Imago 2049 HP）上度量；旋钮只改 HP，`provenance()` 把它写进报告。

## 0.1 修复后实验（post-audit）

- Teacher HP sweep: scales 0.08/0.20/0.50/1.00, 8 seeds per scale, generic combat objective; route labels were post-hoc only.
- BC was trained from the fresh sweep data. The first PPO run was superseded after an audit found that its rollout/validation resets omitted `infinite_ego_resources`; PPO was then rerun with the corrected propagation and its scenario metadata records `infinite_ego_resources=true`.
- Burst (`enemy_hp_scale=0.08`, infinite E.G.O resources): Random 20%, FirstLegal 12%, Greedy 100%, Teacher 100%, BC 100%, PPO 100% wins over 50 seeds.
- Real HP (`enemy_hp_scale=1.0`, Imago 25616 HP): all six policies had 0/50 wins. Thus N=1/5/10/20/50 restart clear rates are also 0 for this run; the real boss result is a failure to reach terminal victory, not a scaled-burst result.
- Full summaries, configuration fingerprints, and restart windows: `reports/post_audit_summary.json`.
- Separate 100-rollout PPO comparison: `reports/ppo_hp_training_100.json`. Half HP achieved 1/100 in matched evaluation; full HP achieved 0/100. Both runs record `infinite_ego_resources=true`. These are superseded by the scenario-specific run below.
- Scenario-specific Teacher → BC → PPO run: `reports/ppo_hp_scenario_specific_1000.json`. Teacher used 200 episodes per scene, PPO used 1000 rollouts per scene, and one Teacher-demonstration constraint update per PPO iteration. Half HP reached 27/100 matched evaluation wins; full HP reached 0/100 because its Teacher produced 0/200 wins.

## 0.2 T1 + curriculum run (current)

The current, seed-separated result is summarized in `reports/ppo_hp_research_t1_curriculum.json`.
The T1 Teacher budget is `horizon=4`, `plan_width=32`, `candidate_cap=6`, `turn_width=4`.

| Scene / policy | Wins | Episodes | Rate | Wilson 95% CI |
|---|---:|---:|---:|---|
| Half T1 Teacher | 272 | 500 | 54.4% | — |
| Half T1 BC | 93 | 500 | 18.6% | 15.4–22.2% |
| Half plain PPO | 119 | 500 | 23.8% | 20.3–27.7% |
| **Half PPO + Teacher demo** | **495** | **500** | **99.0%** | **97.7–99.6%** |
| Full T1 Teacher | 2 | 500 | 0.4% | — |
| Full direct PPO + Teacher demo | 212 | 500 | 42.4% | 38.1–46.8% |
| **Full curriculum PPO (0.50→0.65→0.80→1.00)** | **406** | **500** | **81.2%** | **77.5–84.4%** |

All final evaluations use new seed bands (`42001–42500` half and `43001–43500` full),
`strict=true`, 30 turns, and `infinite_ego_resources=true`. The same full-HP seed band
was used for direct PPO and curriculum PPO. DAgger was implemented and audited, but its
sampled and argmax mixtures were neutral or negative on the fixed half-HP diagnostic band;
it is retained as a negative ablation rather than selected by outcome.

## 1. 协议与样本量（§7 的"必须写明样本数"）

| 项目 | 值 |
|------|-----|
| 场景 | `burst` = Line 6 Section 5 波（9567 + 9572/9573/9574），**场景旋钮** `enemy_hp_scale=0.08`（Imago 2049 HP），12 回合上限，`strict=true` |
| 测试 seed | 9001-9100（100 个独立 seed），四/五策略各自独立跑同一批 seed |
| 重开次数 N | best-of-N 的窗口统计（N=1/10/50），同场景独立 seed = 计划里的"真实凹轨" |
| 训练 seed（教师） | 1-400 |
| 训练 seed（PPO） | 500-3500 随机采样，共 400 局 rollout |
| 验证 seed | 8001-8100（BC 选择、PPO checkpoint 选择），**与测试带完全不重叠** |
| 教师搜索预算 | horizon 3，plan width 32，candidate cap 6，turn width 4（计划的名义值）|
| 计算 | 单机 WSL，无 GPU；网络 124k 参数（NumPy 手写反向传播） |

## 2. 主结果：`burst` 场景，测试带 9001-9100

| 策略 | n | 胜率 | 击杀中位 | P25 | 短胜率 ≤4 回合 | 平均对敌伤害 | 轴事件率（全部局 / 胜局）| best-of-10 窗口胜率 |
|------|---|------|----------|------|----------------|--------------|--------------------------|---------------------|
| Random | 100 | 0.04 | 4.5 | 3.75 | 0.02 | 666 | 0.00 / 0.00 | 0.34 |
| FirstLegal | 100 | 0.11 | 5.0 | 5.0 | 0.02 | 1028 | 0.00 / 0.00 | 0.78 |
| **Greedy**（基线）| 100 | **0.61** | 4.0 | 3.0 | 0.51 | 1755 | 0.12 / 0.20 | 1.00 |
| **Teacher**（阶段 A 搜索）| 100 | **0.64** | 4.0 | 3.0 | **0.59** | 1851 | 0.20 / 0.31 | 1.00 |
| **AI**（BC `models/ai.npz`）| 100 | 0.23 | 4.0 | 4.0 | 0.21 | 1487 | 0.13 / **0.57** | 0.98 |

* 短胜率阈值 T = 4（取自 Greedy 在验证带的中位击杀回合，脚本自动取）。
* "轴事件率" = `detect_axis` 为真的比例，判定条件完全取真实状态与真实统计：
  第一次 Harmony/Solemn Lament 之前 Imago 的 Sinking 达到 Potency ≥5 或 Count ≥3，
  且 E.G.O 之后 2 回合的 Sinking 触发伤害高于之前，且两个 E.G.O 在 3 回合窗口内
  或其中一个构成主要爆发。
* 最快的 AI 胜局在 3 回合（`replays/burst_AI_seed9006.json`），教师也在 3 回合
  （`replays/burst_Teacher_seed9009.json`）；四策略 replay 都在
  `replays/` 并带逐回合 `transition_hash`。

## 3. §7.1 验收对照

脚本 `eval/summarise_reports.py` 直接按计划的条款算出下表（`reports/evaluation.json`
的 `acceptance` 字段）。

| 条款 | Greedy 基线 | **Teacher** | **AI (BC)** |
|------|-------------|-------------|-------------|
| ① 完整回合锁定 + replay 可重放 | ✔（所有 replay 都能用 `transition_hash` 复算） | ✔ | ✔ |
| ② 胜率 ≥ Greedy − 2pp | — | **✔ +3.0pp** | ✘ −38.0pp |
| ③ 中位击杀回合改善 ≥15% 且 ≥1 回合 | 4.0 | ✘ 0%（同为 4.0） | ✘ 0% |
| ④ 短胜率/best-of-N 明显优于 Greedy | 0.51 | **✔ 0.59**（best-of-10 均 100%） | ✘ 0.21 |
| ⑤ ≥70% 的胜局满足轴事件 | 0.20 | ✘ 0.31 | ✘ 0.57（方向对，量不够） |
| ⑥ replay/hash/log 能重放并解释 | ✔ | ✔ | ✔ |

**没有把"多次重开后的最好结果"写成单次稳定能力**：AI 的 best-of-10 窗口胜率
0.98、最快 3 回合都来自重开统计，与它 0.23 的单次胜率并列写出。

## 4. `real` 场景（不打开旋钮，Imago 25616 HP，30 回合）

| 策略 | n | 胜率 | 平均对敌伤害 | 平均存活 | 平均回合数 | 轴事件率 |
|------|---|------|--------------|----------|------------|-----------|
| Random | 20 | 0 | 635 | 0.70 | 6.0 | 0.00 |
| FirstLegal | 20 | 0 | 795 | 0.35 | 6.0 | 0.00 |
| Greedy | 20 | 0 | 3172 | 0.70 | 11.9 | **0.45** |
| AI | 20 | 0 | 2077 | 0.60 | 9.8 | 0.25 |

（原始数据：`reports/real_*.jsonl`，同一批 seed 9001-9020。这里的 `axis` 是
真实状态判定成立的局数——**轴确实会发生，但 Boss 还剩两万多 HP**。）

这一栏不是"策略不好"，而是内容本身：7 名罪人每回合合计约 200-400 伤害，且
第 9 回合左右团灭，25616 HP 在 30 回合内不可清空。计划允许"真实凹轨"，但凹轨
也需要可达的终局；因此 §4 的击杀类指标全部改在 `burst` 场景度量，并在报告与
`provenance` 中标注。

## 5. 阶段内部数据

| 阶段 | 输入 | 结果 |
|------|------|------|
| A 教师（首批，单回合价值）| 400 seed | 胜率 0.435，中位击杀 4 |
| A 教师（回合级树，正式）| 400 seed | **胜率 0.708**，中位击杀 4，P25 3，平均存活 4.95 |
| B BC（教师 400 局）| 90k actor 决策 | 逐 actor top-1 准确率 **0.896-0.918**（验证带 100 seed 上胜率 0.27）|
| B DAgger-lite 一轮 | 96 局混合轨迹 | 合并训练后验证带胜率 0.24（未改善）|
| C PPO | 400 局 rollout，20 迭代 | rollout 采样胜率 0.20-0.45 波动，验证带贪心胜率 0.17；**未超过 BC** |

BC 的准确率与它 0.23-0.27 的胜率之间的落差是本次实验的主要发现：**逐 actor 的
10% 误差会在 7 个 actor 上复合**（0.9^7 ≈ 0.48），棋面上的小偏差把"能不能活到
收割"整个改变了。计划 §5 允许"共享 encoder + 7 个 actor head"，本次用的是
"共享 encoder + 候选打分器 + 自回归上下文"，从结果看需要更强的结构（或更多
DAgger 轮次与更大算力）才能把教师的能力搬进网络。

## 6. 交付物位置

| 计划里的交付物 | 本仓库 |
|----------------|--------|
| `docs/TRAINING_PLAN.md` | `docs/TRAINING_PLAN.md`（原仓库根目录的规划文件）|
| `search/` 多回合 beam 教师 | `search/run_teacher.py`、`search/run_dagger.py` |
| `training/` BC + PPO | `training/train_bc.py`、`training/train_ppo.py` |
| `eval/` 基线与指标 | `eval/run_eval.py`、`eval/summarise_reports.py` |
| `replays/` 成功与失败样例 | `replays/*.json`（每回合带 `transition_hash`）+ `replays/index.json` |
| `reports/evaluation.json` + 回放索引 | `reports/evaluation.json`（burst）、`reports/evaluation_real.json`（real）、`reports/<scenario>_<policy>.jsonl` |
| checkpoint + 配置 + seed 清单 | Legacy: `models/ai.npz`; post-audit: `models/bc_post_audit.npz`, `models/ppo_post_audit.npz` and their JSON reports |

## 7. 复现

```bash
# 见 docs/TRAINING.md §5 的完整步骤；最关键的几条：
python search/run_teacher.py --scenario burst --seed-start 1 --seed-count 400 \
    --horizon 3 --plan-width 32 --candidate-cap 6 --turn-width 4 --jobs 12 --out data/teacher_tree
python training/train_bc.py --data data/teacher_tree --out models/ai.npz --epochs 14
python eval/run_eval.py --scenario burst --seed-start 9001 --seed-count 100 \
    --policies Random FirstLegal Greedy AI Teacher --checkpoint models/ai.npz
python eval/summarise_reports.py --scenario burst
```

`reports/evaluation.json` 的 `provenance` 里带着：git commit、数据指纹
（`data/**/*.json` 的 SHA-256 前缀）、场景参数、seed 范围、策略清单、重开次数、
生成时间、Python/NumPy 版本；`replays/index.json` 是回放索引（每回合的
`transition_hash`）。

## 8. 已知局限（写清楚，不藏）

* **单次稳定性未达标**：AI 策略只有在重开（best-of-N）时才表现出竞争力；
  这正是计划 §0 区分"轴存在"与"轴稳定"的原因，本阶段只证明了前者。
* **PPO 预算极小**：400 局 rollout，远低于通常的 PPO 用量；结论"PPO 未改善"
  只在这个预算下成立。
* **场景旋钮**：击杀类指标在缩放 HP 的场景上取得，真实 HP 场景不可达终局。
* **教师的价值函数是手写第一版**（§4.3 允许），没有训练 value head；它的
  凹轨能力部分来自树搜索而不是价值估计。
* **轴判定的第二条**（"两个 E.G.O 在 3 回合窗口内配合"）在实战里很少同时满足
  （Sinclair 的 Harmony 资源需求较高），因此 `burst_ok`（触发伤害升高）是
  更常起作用的那一条。
