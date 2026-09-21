# 回合级战斗 AI 训练与验证规划

> 本文件是给 AI 编程代理执行的工程规格，不是概念介绍。实现时以本文的接口、边界和验收标准为准；若现有代码与本文冲突，先记录差异，再修改模拟器或适配层，不要悄悄改变实验定义。

## 0. 实验结论先写清楚

目标不是证明 AI 每个 seed 都能稳定打出同一套流程，而是验证：

1. 在真实的“回合开始统一选行动、一次提交、按速度结算”规则下，理论上存在的短速杀轴能被搜索和策略模型发现；
2. 允许真实凹轨：坏 RNG、错误 clash、非理想牌面可以导致重开，不能因为方差大就把这类轨道判为失败；
3. 在足够多的独立 seed 上，AI 能产生胜率不劣于 Greedy、且短胜局明显更短的结果；
4. replay/log 能证明短胜局使用了 `Sinking -> Harmony / Solemn Lament -> Sinking 触发` 的真实技能联动，而不是宏动作标签。

“轴存在”与“轴稳定”是两个不同问题。本阶段只要求前者，并把短胜局的出现概率、下分位击杀回合和 best-of-N 结果记录下来。

## 1. 不可违反的游戏流程

每个环境 step 是完整的一回合，不是单个角色动作：

```text
reset(seed)
  -> 读取当前 BattleState
  -> 为全部角色选择技能、目标和 E.G.O
  -> 一次性提交完整 Plan
  -> simulator 按速度、clash、命中、状态和攻击结束规则结算
  -> 返回下一回合 BattleState、reward、done、BattleLog
```

模型内部可以按固定顺序逐个生成 7 个动作，这是输出编码方式；生成完以前不得提交或读取中间战斗结果。模拟器只能在完整计划提交后推进一回合。

Rust/Python 最低接口：

```rust
fn step_turn(&mut self, plan: Vec<Action>) -> StepResult;
```

```python
state = env.reset(seed)
next_state, reward, done, info = env.step_turn(plan)
```

`info` 必须包含可重放的 `battle_log`、`state_hash_before`、`state_hash_after`、RNG continuation/seed 标识和本回合统计。

## 2. 数据结构与合法性

### 2.1 Action

动作必须对应真实可执行技能，不得用“叠沉沦”“爆发”之类宏动作替代：

```json
{
  "actor": "sinner-4-11004",
  "slot": 1,
  "kind": "UseSkill",
  "skill_id": "skill-...",
  "target": "enemy-7-9567",
  "is_ego": false,
  "ego": null,
  "ego_kind": null
}
```

E.G.O 动作必须显式带上 `ego`、`ego_kind`、目标和资源消耗。每个角色都必须有动作；若真实规则允许防御/空过，使用显式的 `Defend`/`Pass`，不要用缺省字段隐式表示。

### 2.2 BattleState observation

至少包括：

- Boss：HP/max HP、阶段、技能槽、Sinking Potency/Count、Past/Present/Future、Temporal Disjunction；
- 每个角色：HP、SP、Speed、Stagger、存活、技能槽、合法动作、E.G.O 资源、ammo/charge、技能使用次数、状态；
- 回合上下文：turn index、上一回合伤害、Sinking 触发伤害/次数、行动顺序、已使用 E.G.O、阶段阈值信息；
- 随机性：seed 或可复现 RNG continuation 的引用，不把不可复现的随机状态丢掉。

先用稳定的离散 ID/数值编码；不要依赖技能名称文本语义。所有 action mask 必须由模拟器的合法性检查产生，训练和搜索共用同一份 mask。

## 3. Reward 与目标函数

主目标按优先级排列：

```text
胜利 > 击杀速度 > 我方存活/资源 > 伤害与 Sinking 过程信号
```

建议初始 reward：

```text
Boss 击杀                         +1000
每经过一回合                       -10
我方角色死亡                       -30
团灭                              -1000
Boss HP 实际下降                    +0.01 * damage
真实 Sinking 触发造成的伤害          +0.02 * sinking_damage
```

不要给“增加 Potency/Count”过高的独立奖励，否则会学出无限叠层而不收割。过程奖励只能辅助 credit assignment，终局胜利和速度始终占主导。

搜索和训练分别保存以下指标，避免把单回合伤害误当最终目标：

- `terminal_win`、`kill_turn`；
- `best_so_far_kill_turn`；
- Sinking 建立窗口、E.G.O 爆发窗口和触发伤害；
- 死亡、资源消耗、阶段转换。

## 4. 阶段 A：多回合搜索教师（先做这个）

### 4.1 搜索节点

节点是“完整回合结束后的 BattleState”，不是角色动作，也不是单回合事件：

```text
state(t)
  -> candidate full 7-person plan
  -> commit + step_turn()
  -> state(t+1)
  -> ...
  -> leaf value / terminal result
```

初始 horizon：3；通过后扩展到 5，再到 8。不要一开始搜索整局结束。

### 4.2 候选计划生成

实现一个统一的 `PlanGenerator`：

1. 按固定 actor 顺序逐个生成合法动作；
2. 每一步应用 action mask；
3. 只在生成完 7 个动作后调用 simulator；
4. 用 beam width 32 做第一版 baseline；
5. beam 节点保存完整计划、父节点、RNG continuation、state hash 和累计统计。

搜索必须支持多次重开。对同一个初始场景允许运行多个独立 seed/rollout；不要用“某一 seed 没找到短轴”否定轴的存在。

### 4.3 叶子价值

第一版使用显式手写 value function，之后再训练 value head：

```text
value(state) =
    terminal_win * WIN_BONUS
  - normalized_boss_hp
  - turn_penalty
  - ally_damage_penalty
  - death_penalty
  + sinking_readiness
  + available_ego_value
```

终局排序必须使用词典序或等价的强权重：

1. 已胜利优先；
2. 胜局 `kill_turn` 越小越好；
3. 未胜局 Boss 剩余 HP 越低越好；
4. 再比较我方存活、资源和 Sinking 准备度。

单回合伤害不得作为唯一评分。

### 4.4 随机性与 transposition

- `state_hash` 必须覆盖所有影响未来战斗的状态；
- transposition table 的 key 至少包括 `state_hash + rng_continuation_id`；
- 同一计划可在多个 RNG continuation 上评估，保存均值、最好结果和风险/方差；
- 训练数据标注必须保留“这是平均好计划”还是“这是凹到的短轨”。

搜索输出格式：`state -> full_plan -> next_state -> leaf/terminal metrics -> replay`。每条 teacher 样本必须能由 simulator 重放。

## 5. 阶段 B：Behavior Cloning

用阶段 A 的完整计划样本训练策略模型：

- 输入：当前完整 BattleState；
- 输出：7 个 actor 的真实动作分布或自回归动作分布；
- loss：只在合法 action mask 内计算；
- 样本权重：对短胜局、低 `kill_turn` 和高质量爆发窗口提高权重，但保留一定比例普通/失败样本，避免分布崩塌；
- 训练/验证/测试按 seed 分离，不按相邻 replay 随机切分。

模型可以是“共享 state encoder + 7 个 actor head”，也可以是按 actor 顺序的自回归 decoder；两者都必须在最后一个动作生成完后才调用 `step_turn`。

## 6. 阶段 C：PPO 微调

PPO 只在 BC 能稳定输出合法计划后启用：

1. rollout 时每个环境 step 采样完整 7 人 plan；
2. simulator 只结算完整回合；
3. advantage/return 以回合为时间单位；
4. action log-prob 是 7 个动作 log-prob 的联合和；
5. 对过早 Harmony、过早 Solemn Lament、无意义叠层由终局/过程结果自然惩罚，不添加会锁死策略的手工规则。

PPO 的目标是让短胜轨道更容易被发现，不要求消除所有坏轨道。固定训练 seed 不能进入测试 seed。

## 7. Baseline 与评测协议

必须实现四个策略：

1. `Random`：从合法动作均匀采样；
2. `FirstLegal`：每名角色取第一个合法动作；
3. `Greedy`：按当前可见即时收益/安全性排序，不看多回合搜索；
4. `AI`：BC/PPO 策略。

建议 seed 分组：训练 1–8000，验证 8001–9000，测试 9001–10000。若计算资源不足，至少保证测试集 500–1000 个独立 seed，并在报告中写明样本数。

由于允许真实凹轨，报告不能只给平均值，必须同时给：

- 胜率；
- 成功局平均/中位 `kill_turn`；
- P10/P25 成功局击杀回合（短轨指标）；
- `kill_turn <= T` 的短胜率，其中 `T` 由 Greedy 验证集的目标分位数或预设速杀阈值确定；
- best-of-N 重开结果：N = 1、10、50（同一初始场景的独立 RNG 尝试）；
- 最快击杀回合、长尾失败率、平均存活人数；
- Harmony/Solemn Lament 使用回合、Sinking 触发伤害和完整 replay。

### 7.1 第一版验收标准

满足以下条件才称为“发现了可行短轴”：

1. AI 遵守完整回合锁定流程，所有 replay 可重放；
2. 测试集胜率不低于 `Greedy 胜率 - 2 个百分点`（例如 Greedy 为 72%，AI 至少为 70%）；
3. 在相同胜率约束下，AI 成功局平均或中位击杀回合至少改善 15%，且绝对至少少 1 回合；
4. AI 的短胜率或 best-of-N 短胜结果显著优于 Greedy，不能只靠单一 seed；
5. 至少 70% 的 AI 成功短胜局满足以下轴事件：
   - 第一次 Harmony/Solemn Lament 前已建立明显 Sinking（第一版阈值：Potency +5 或 Count +3）；
   - 两个 E.G.O 在 3 回合窗口内形成配合，或其中一个构成主要爆发；
   - E.G.O 后 2 回合的 Sinking 触发伤害高于前置窗口；
6. replay、state hash、transition hash 和战斗 log 能重放并解释这些短胜局。

这里的“显著改善”允许通过多次重开获得；不要求每个 seed 都触发轴，也不要求 Harmony/Solemn Lament 每局固定在同一回合使用。Greedy 的轴出现率仍要单独报告：若 Greedy 已经自然达到同等轴出现率，则只能称为“复现 baseline”，不能称为 AI 独立发现。

## 8. Replay 与正确性门槛

在训练前完成：

- 技能、clash、速度排序、状态触发、E.G.O 资源的单元测试；
- golden replay：固定输入 plan + seed 得到固定输出 hash；
- transition hash：逐回合校验前后状态；
- 非法 action 必须在 simulator 入口拒绝；
- 同一 replay 在 Rust 原生接口和 Python wrapper 中结果一致。

如果模型表现异常，先检查 simulator 和 replay，不要立即扩大网络或调整 reward。模型只能学到模拟器提供的世界。

## 9. 面向编程代理的实现顺序

### P0：代码勘察与差异报告

- 找到现有 `BattleState`、单角色 action 提交、回合推进、RNG、log 和 Python binding；
- 输出当前接口与本文接口的差异；
- 不在差异未记录前改训练代码。

### P1：完整回合接口

- Rust 实现 `Vec<Action> -> validate -> commit_all -> step_turn -> StepResult`；
- Python 暴露 `env.step_turn(plan)`；
- 添加完整 plan 合法性检查和 golden replay。

### P2：搜索 baseline

- 实现固定 actor 顺序的完整 plan generator；
- beam width 32、horizon 3；
- transposition table、RNG continuation 和 replay 导出；
- 在至少 100 个 seed 上验证能找到短轴候选。

### P3：评测框架

- 实现 Random/FirstLegal/Greedy/AI runner；
- 输出统一 JSONL/CSV 指标和 replay 路径；
- 支持 N 次重开与 P10/P25/best-of-N 统计。

### P4：BC 与 PPO

- 用搜索样本训练 BC；
- 通过合法性、loss、replay 回归后再启用 PPO；
- 保存 checkpoint、配置、git commit、simulator hash 和 seed 清单。

### P5：实验报告

- 分别报告“稳定表现”和“凹轨短胜表现”；
- 报告 Greedy 对照和轴出现率；
- 附至少若干条可重放的成功 replay；
- 明确注明是否为理论存在性验证，不能把多次重开后的最好结果写成单次稳定能力。

## 10. 交付物

最终代码至少应产出：

```text
docs/TRAINING_PLAN.md
src/ / rust simulator changes
python/ environment wrapper
search/ multi-turn beam teacher
training/ behavior cloning + PPO
eval/ baselines + metrics
replays/ successful and failed golden cases
reports/ evaluation.json + replay index
```

任何结果都要带：代码版本、模拟器版本/hash、配置、seed 范围、重开次数 N、评测时间和完整指标。这样 AI 编程代理可以按 P0→P5 实施，人类也能区分“确实发现短轴”和“偶然凹中一局”。
