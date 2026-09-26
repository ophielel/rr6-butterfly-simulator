# 训练与评测实现说明 (TRAINING.md)

本文件说明 `TRAINING_PLAN.md`（原 `TRAINING_PLAN.md`，现 `docs/TRAINING_PLAN.md`）
在 Python 侧是怎么落地的：接口、模块、运行方式、以及实现过程中**必须记录的偏差**。
实验结论与验收对照在 `docs/TRAINING_RESULTS.md`。

## 1. 任务边界（不可违反的流程）

每个环境 step 是**完整一回合**：

```text
reset(seed) -> 读取 observation -> 为全部角色选技能/目标/E.G.O
            -> 一次性提交完整 plan -> simulator 结算
            -> 返回下一回合 observation、reward、done、info/log
```

* Rust: `Simulator::submit_plan(state, Vec<Action>)`；
* Python: `LimbusEnv.step_turn(plan)`。

`submit_plan` 的校验**只**来自模拟器自己的 `legal_actions()`：

1. plan 中出现 `Commit` → 拒绝；
2. 同一 actor 出现两次 → 拒绝；
3. 任一 action 不在当前合法动作集合里 → 拒绝（`illegal action in plan: ...`）；
4. 提交完仍有可行动单位没行动 → 拒绝（`incomplete plan: ...`）。

任何一种失败都会**回滚**：`*state = snapshot`，状态哈希不变（Rust 测试
`submit_plan_is_validated_atomically`）。增量接口 `submit()`/`step()` 仍保留，
两者结算同一份 plan 得到完全一致的状态哈希
（`submit_plan_matches_incremental_submission`）。

`info` 里带齐回放材料：`state_hash_before/after`、`transition_hash`、
`search_key`、`seed`、`rng_before/after`（RNG 续跑标识）、本回合新增 `log`、
以及本回合的真实统计 `stats`（见 §3）。

## 2. 模块地图

| 文件 | 作用 |
|------|------|
| `python/lcb/env.py` | `LimbusEnv`：`reset`/`observe`/`legal_actions`/`submit`/`step_turn`/`search_key` |
| `python/lcb/features.py` | 稳定离散编码：状态 `state_dim=1794`、动作 `action_dim=44`；技能数值来自生成的 library JSON（不依赖技能名称文本） |
| `python/lcb/plans.py` | 固定 actor 顺序的**整回合计划生成器**（beam + action mask）、解析式先验 `estimate_action` |
| `python/lcb/teacher.py` | 阶段 A：回合级树搜索教师、§4.3 手写 value、轴事件检测 `detect_axis`、样本编码 `encode_samples` |
| `python/lcb/rewards.py` | §3 奖励 + 每局的统计载体 `EpisodeStats` |
| `python/lcb/baselines.py` | `Random` / `FirstLegal` / `Greedy`（真实模拟器单回合前瞻）/ `NeuralPolicy` |
| `python/lcb/nn.py` | 纯 NumPy 策略/价值网络（手写反向传播 + Adam），无 torch 依赖 |
| `python/lcb/bc.py`, `python/lcb/ppo.py` | 阶段 B/C |
| `python/lcb/dataset.py` | 教师样本的 npz 存取、**按 seed 切分**、`iter_decisions` |
| `training/run_dagger.py` | DAgger：在策略自己访问到的 clone 状态上请教师打标签 |
| `python/lcb/evaluate.py` | §7 评测协议：`run_episode` / `summarise` / `best_of_n` / `provenance` |
| `python/lcb/scenarios.py` | 评测场景（见 §4 的场景旋钮） |
| `search/run_teacher.py`, `training/run_dagger.py` | 阶段 A 数据收集 CLI |
| `training/train_bc.py`, `training/train_ppo.py` | 阶段 B/C CLI |
| `eval/run_eval.py` | 四策略评测 CLI，输出 `reports/*.jsonl` + `reports/evaluation.json` + `replays/` |

## 3. 观测、动作与统计

**观测** (`observe()` → `PySimulator::observation_json`)：Boss 与每个角色的
HP/max HP/Shield/SP/Speed/Stagger/存活/全部状态（Potency/Count/Stack）、技能槽
（current/next/preview/target）、E.G.O 槽位、物理与罪孽抗性、阶段、回合数、
E.G.O 资源、Sin Resonance、RNG 续跑标识（`draw_count` + 内部状态）。
**战斗日志不进入观测**：日志长度不能泄漏进特征，它只用于回放与轴分析。

**动作**：每个候选携带技能自身数值（Base/Coin Power、硬币数、攻击权重、
罪孽、伤害类型、是否防御/E.G.O/不可拼点）、目标块（HP 比例、抗性、Sinking、
时态 Stack、是否 1 HP 幻影）、以及**上下文块**（当前是第几个 actor、前面
actor 选了什么）。上下文块使"共享打分器 + 固定 actor 顺序"等价于自回归策略，
不需要给 7 个角色各一个 head。

**arrmask**：训练、搜索、评测共用 `legal_actions()`，没有任何一处自己造动作。

**每回合统计** (`TurnStats`，由引擎在真实结算时累计，不是估算)：
`damage_to_enemies`、`damage_to_allies`、`status_damage`、`sinking_damage`、
`sinking_sp_damage`、`sinking_triggers`、`skill_uses`、`ego_uses`、`deaths`。

**奖励** (§3)：`+1000 击杀`、`-10/回合`、`-30 角色死亡`、`-1000 团灭`、
`+0.01*对敌伤害`、`+0.02*Sinking 触发伤害`。叠层本身不给独立奖励，避免"只叠不收"。

## 4. 必须记录在案的偏差

1. **场景旋钮 `enemy_hp_scale`**（`BattleConfig` 字段，Python `reset(...)` 可传）。
   它不是游戏规则，只是评测场景参数。原因见 `docs/TRAINING_RESULTS.md`：
   真实 25616 HP 的 Imago 在 30 回合上限内无法被清空（队伍约 200-400 伤害/回合，
   且在 9 回合左右被团灭），要度量 `kill_turn`/短胜率/best-of-N 就必须让终局胜利
   可达。`provenance()` 会把该值写进每一份报告。
2. **Support Passive 未启用**（与模拟器一致：没有队伍槽位选择的数据）。
3. **事件选择不在范围内**（用户裁定），因此第 5 节开局状态固定为默认时态。
4. **计算预算**：教师数据 400 局、BC 若干轮、PPO 400 局、测试集 100 个独立 seed。
   `TRAINING_PLAN.md` §7 允许在资源不足时缩小样本量，但**必须写明样本数**——
   报告里每一项都标了。
5. **PPO 的 checkpoint 选择**用验证带（8001+）的贪心胜率，测试带（9001+）
   只用于最终报告，不参与任何选择。

## 5. 复现步骤

```bash
# 0) 数据与扩展模块（见 README 的构建步骤）
python3 tools/build_all.py --skip-fetch
PYO3_PYTHON='C:/Python314/python.exe' cargo build -p lcb-py --manifest-path sim/Cargo.toml
cp sim/target/debug/lcb_sim.dll python/lcb/lcb_sim.pyd

# 1) 阶段 A：教师数据（400 局，12 进程约 3 分钟）
python search/run_teacher.py --scenario burst --seed-start 1 --seed-count 400 \
    --horizon 3 --plan-width 32 --candidate-cap 6 --turn-width 4 --jobs 12 \
    --out data/teacher_tree

# 2) 阶段 B：行为克隆
python training/train_bc.py --data data/teacher_tree --out models/bc_tree.npz --epochs 14

# 2b) 可选：DAgger 一轮，再合并训练；Teacher 查询的是策略实际访问状态的 clone
python training/run_dagger.py --checkpoint models/bc_tree.npz --out data/dagger1 \
    --scenario burst --seed-start 2001 --seed-count 96 --teacher-budget t1 --jobs 4
python training/train_bc.py --data data/teacher_tree data/dagger1 --out models/bc2.npz

# 3) 阶段 C：PPO（从 BC 继续，验证带选 checkpoint）
python training/train_ppo.py --checkpoint models/bc_tree.npz --out models/ppo_tree.npz \
    --scenario burst --seed-start 500 --seed-count 3000 --iterations 20 \
    --episodes-per-iteration 20 --val-seed-start 8001 --val-seed-count 12

# 4) 阶段 D：评测（测试带 9001+，四策略）
python eval/run_eval.py --scenario burst --seed-start 9001 --seed-count 100 \
    --policies Random FirstLegal Greedy AI --checkpoint models/ai.npz --greedy-cap 6
python eval/run_eval.py --scenario real --seed-start 9001 --seed-count 40 \
    --policies Greedy AI --checkpoint models/ai.npz --jobs 1

# 5) 测试
python python/tests/test_training.py     # 训练层
python python/tests/test_env.py          # wrapper 冒烟
cargo test --manifest-path sim/Cargo.toml
```

## 6. 与计划的接口对照（P0 差异报告）

| 计划要求 | 现状 |
|----------|------|
| `step_turn(plan) -> StepResult` | ✔ 已实现（Rust `submit_plan` + PyO3 `step_turn`），非法/不完整 plan 原子拒绝 |
| `info` 含 `battle_log`/`state_hash_before`/`state_hash_after`/RNG 标识/回合统计 | ✔ |
| 节点 = 完整回合后的 BattleState | ✔ `BeamTeacher.plan_turn` 用回合级树（`horizon` 层，每层 `turn_width`） |
| beam width 32 / horizon 3 | ✔ 默认即 32/3（`TeacherConfig`） |
| transposition key = `state_hash + rng continuation` | ✔ `search_key`（去掉日志/警告/回合统计，RNG 状态在哈希内）+ plan 组成 key，缓存的是"结算后的状态 + 回合奖励" |
| 多次重开 | ✔ 同场景不同 seed（best-of-N 的窗口统计） |
| BC 只在合法 mask 内算 loss | ✔ 候选集直接来自 `legal_actions()` |
| 按 seed 切分训练/验证/测试 | ✔ `dataset.seed_split`，PPO 的验证带单独指定 |
| 四策略对照 + P10/P25/短胜率/best-of-N | ✔ `evaluate.summarise` / `best_of_n` |
| 轴事件校验（Sinking→Harmony/Solemn Lament→触发） | ✔ `detect_axis`（读真实状态与 `stats`，不读技能名文本） |
| 结果带代码版本/模拟器数据指纹/配置/seed 范围/重开次数 | ✔ `provenance()` 写入 `reports/evaluation.json` |
| 手动估算伤害 | ✘ 只在一处：`estimate_action`（actor 级 beam 的先验）。**任何完整计划的打分都来自真实模拟**，见 `teacher.py` |
