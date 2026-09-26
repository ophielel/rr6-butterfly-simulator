# RR6 Section 5 罗生蝶模拟器：从零编写到 Teacher → BC → PPO Curriculum 的完整工程记录

> 本文是本仓库从开始编写模拟器，到完成当前场景专属训练、DAgger 审计和完整血量评测的全过程记录。
>
> 截止版本：post-fix cap/reward-removal update，分支：`main`，远程：`origin/main`。
>
> 对应提交：`fd7447b`（Sinking 上限、奖励消融、matched T1 artifacts）。
>
> 本文记录实现路径、证据来源、代码结构、修复过程、实验协议、结果、协议偏差和仍未解决的问题。它不是一份新的游戏规则表。逐条规则的来源和实现状态仍以 [`docs/MECHANICS.md`](MECHANICS.md) 为准，生成数据覆盖以 [`docs/COVERAGE.md`](COVERAGE.md) 为准，训练接口以 [`docs/TRAINING.md`](TRAINING.md) 和 [`docs/TRAINING_PLAN.md`](TRAINING_PLAN.md) 为准。

---

## 目录

1. [项目要解决什么问题](#1-项目要解决什么问题)
2. [最先确定的工程原则](#2-最先确定的工程原则)
3. [固定内容和边界](#3-固定内容和边界)
4. [从资料到可运行数据的流程](#4-从资料到可运行数据的流程)
5. [总体架构](#5-总体架构)
6. [模拟器核心的构建顺序](#6-模拟器核心的构建顺序)
7. [技能、牌面和行动系统](#7-技能牌面和行动系统)
8. [拼点、攻击和伤害](#8-拼点攻击和伤害)
9. [状态、理智、被动和 E.G.O](#9-状态理智被动和-ego)
10. [罗生蝶和第五站场景](#10-罗生蝶和第五站场景)
11. [Replay、Hash、Clone 和严格模式](#11-replayhashclone-和严格模式)
12. [第一次完整模拟器审阅](#12-第一次完整模拟器审阅)
13. [第二轮、第三轮和完整复查](#13-第二轮第三轮和完整复查)
14. [固定内容修复和 post-audit](#14-固定内容修复和-post-audit)
15. [训练系统的设计](#15-训练系统的设计)
16. [阶段 A：Random、Greedy、Beam Teacher](#16-阶段-arandomgreedybeam-teacher)
17. [阶段 B：Behavior Cloning](#17-阶段-bbehavior-cloning)
18. [阶段 C：PPO 和 Teacher demonstration replay](#18-阶段-cppo-和-teacher-demonstration-replay)
19. [DAgger：为什么做，怎么做，结果如何](#19-dagger为什么做怎么做结果如何)
20. [HP 场景和 curriculum](#20-hp-场景和-curriculum)
21. [评测协议和指标](#21-评测协议和指标)
22. [所有主要实验的时间线](#22-所有主要实验的时间线)
23. [当前结果与历史 final baseline](#23-当前结果与历史-final-baseline)
24. [结果应该怎样解释](#24-结果应该怎样解释)
25. [复现当前流程](#25-复现当前流程)
26. [文件和产物索引](#26-文件和产物索引)
27. [测试和验证记录](#27-测试和验证记录)
28. [已知协议问题和文档注意事项](#28-已知协议问题和文档注意事项)
29. [仍然没有完成的工作](#29-仍然没有完成的工作)
30. [最终总结](#30-最终总结)

---

## 1. 项目要解决什么问题

项目目标不是制作一个覆盖所有人格、所有敌人、所有模式的完整《Limbus Company》客户端模拟器，而是制作一个**面向 RR6 Section 5 罗生蝶战斗研究的、来源驱动的小型前向模拟器**。

具体目标有两个层次：

### 1.1 第一层：让模拟器成为可信的规则执行器

模拟器必须能够从一个明确的 BattleState 和一组完整行动出发，按以下顺序推进：

```text
BattleState(t)
  -> 读取合法行动
  -> 每名可行动单位选择一个真实技能、目标或 E.G.O
  -> 一次性提交完整回合 Plan
  -> 按速度、拉拼、拼点、攻击、状态、攻击结束和回合结束顺序结算
  -> BattleState(t+1)
```

模拟器不能为了让某个策略更强而修改规则，也不能把“叠沉沦”“执行爆发”之类策略概念伪装成游戏行动。所有行为必须落到真实技能、真实目标、真实资源和真实状态上。

### 1.2 第二层：在模拟器之上研究行动策略

模拟器稳定后，Python 层才负责：

- 生成 Random、FirstLegal、Greedy 基线；
- 通过真实模拟进行回合级 Beam Search；
- 把搜索教师的数据蒸馏为 Behavior Cloning 策略；
- 用 PPO 进行回合级微调；
- 在半血和完整血量场景上测量胜率、击杀回合、短胜率、重开结果和回放。

这两个层次必须分离：

```text
Rust simulator = 规则、状态、结算、RNG、Replay、Hash
Python research = 环境、搜索、训练、评测、统计
```

训练结果不能反过来证明模拟规则正确。策略只能学到模拟器实际提供的世界，因此模拟器审阅必须先于大规模训练。

---

## 2. 最先确定的工程原则

最初计划见 [`docs/archive/rr6_butterfly_simulator_plan_v2.md`](archive/rr6_butterfly_simulator_plan_v2.md)。整个项目一直遵循以下原则。

### 2.1 资料优先级

1. 游戏内文本、技能说明、状态说明、Boss 描述。
2. 英文 wiki.gg，主要用于数值、行为、技能效果和战斗规则。
3. 日文 WikiWiki，主要用于拼点、牌面、攻击累积、守备技能和攻击时序。
4. 韩文游戏文本或其他可靠资料。
5. 中文资料只用于辅助显示，不作为核心规则唯一来源。

### 2.2 不确定性处理

资料没有确认的规则不能靠“看起来合理”补上。项目使用以下标记：

- `UNKNOWN`：知道规则存在，但当前资料不足以确定具体行为；
- `NOT_IMPLEMENTED`：规则已经知道，但代码还没有执行它；
- `synthetic`：为了让模拟器可重复而加入的工程替代，例如客户端 RNG 算法；
- `official`：来自游戏内文本；
- `single_source_verified`：有一个可追溯来源；
- `multi_source_verified`：至少有两个来源交叉确认；
- `video_verified`：有游戏视频、截图或逐帧记录验证。

`REAL` 或严格场景不能把 `UNKNOWN` 和 `synthetic` 偷换成官方事实。严格模式会在关键技能含有未建模效果时阻止执行，并通过运行时接口列出阻塞项。

### 2.3 模拟器和搜索分离

Rust 不知道“沉沦轴”“人类路线”或“最优策略”。Rust 只知道：

- 当前状态；
- 当前合法行动；
- 提交行动后的真实结果；
- RNG 续行状态；
- 前后状态 Hash 和 Transition Hash。

搜索、奖励、Teacher、BC、PPO、DAgger 和评测全部在 Python 层实现。

### 2.4 实验旋钮不能伪装成游戏规则

最终研究中使用两个明确记录的实验条件：

- `enemy_hp_scale`：只缩放场景敌方 HP，`1.0` 才是数据中的真实 Imago HP；
- `infinite_ego_resources`：绕过 E.G.O 资源可用性和资源扣除，但不绕过 SP、Overclock 费用、技能效果和状态效果。

两项都不是游戏规则。每份 Teacher、训练、验证和评测报告都必须记录完整场景配置。

---

## 3. 固定内容和边界

### 3.1 目标战斗

正式目标是 Line 6, Section 5 的 Wave 1：

```text
9567  Butterfly of Entangled Lives::Imago
9572  Illusory Butterfly::The Past
9573  Illusory Butterfly::The Present
9574  Illusory Butterfly::The Future
```

Imago 是主敌人。三个 Illusory Butterfly 是附属敌人，命中幻影会改变 Imago 的时间状态 Stack。主敌人被击败时，战斗结束，即使附属单位仍然存在。

### 3.2 固定队伍

项目固定建模七个身份及相关 E.G.O 数据。核心身份记录包括：

| ID | 固定内容 |
|---|---|
| `10110` | Lobotomy E.G.O::Solemn Lament 对应 Yi Sang |
| `10414` | Lobotomy E.G.O::Faint Aroma & Solitude 对应 Ryoshu |
| `10813` | Jeong's Office Rep 对应 Hong Lu |
| `10913` | Lobotomy E.G.O::The Sword Sharpened with Tears 对应 Rodion |
| `11004` | Los Mariachis Jefe 对应 Sinclair |
| `11114` | LCA Udjat Vanguard Team 3 Leader 对应 Outis |
| `11214` | Lobotomy E.G.O::Lamp 对应 Gregor |

相关 E.G.O 记录以正式 ID 保存在 `data/ego/`，包括 `20106`、`20109`、`20807`、`20810`、`20903`、`21009`、`21206`、`21207` 等记录。原始数据中存在不同人格、版本和技能变体，不能只按玩家简称判断唯一身份。

### 3.3 不是目标的内容

以下内容可能被加载来解释战斗数据，但不是当前研究的完整目标：

- Pupa 之前的所有站点选择事件；
- 需要独立 Parts 的完整 Focused Encounter；
- 全游戏其他 Section；
- 所有未进入固定队伍的身份、E.G.O 和敌人；
- 完整客户端 UI；
- 支持被动的组队槽位选择界面。

Pupa、站点 2-4 和 campaign state 后来被加入，用来承载第五站的上下文和 Section 5 机制，但“事件选择 UI 本身”仍不是当前正式训练接口的一部分。

---

## 4. 从资料到可运行数据的流程

模拟器运行时不访问 wiki。运行时只读取 `data/` 中生成的数据。这样做有三个目的：

1. 固定某次实验看到的规则输入；
2. 可以对生成 JSON 做 diff；
3. 可以在报告中记录数据指纹，而不是依赖某个网页当前内容。

### 4.1 原始资料缓存

目录结构：

```text
data/_raw/
  pages/       wiki.gg 页面和日文 Wiki 缓存
  gamedata/    游戏本地化和内部 ID 文本
  wiki_status_text.json
```

`data/_raw/` 是缓存，不应手工编辑。wiki.gg 访问受到 WAF 限制，项目通过 `tools/wikigg.py` 经 `r.jina.ai` 获取并缓存 MediaWiki 内容。游戏文本通过 `tools/fetch_gamedata.py` 拉取固定 commit 的本地化仓库，并将 commit SHA 写入 `data/sources/gamedata.json`。

### 4.2 生成数据目录

```text
data/
  identities/<id>.json
  ego/<id>.json
  enemies/<id>.json
  statuses/statuses.json
  mechanics/effects.json
  mechanics/status_effects.json
  mechanics/enemy_scripts.json
  passives/passives.json
  sources/_index.json
  sources/gamedata.json
```

### 4.3 数据构建命令

完整流水线可以拆成以下步骤：

```bash
python3 tools/fetch_gamedata.py
python3 tools/fetch_pages.py
python3 tools/build_library.py
python3 tools/extract_effects.py
python3 tools/build_enemy_scripts.py
python3 tools/extract_passives.py
python3 tools/extract_panic.py
python3 tools/extract_status_effects.py
python3 tools/report.py
```

通常使用缓存模式：

```bash
python3 tools/build_all.py --skip-fetch
```

所有脚本设计为幂等。重复执行时，只重建生成结果，不应手工修改生成 JSON 来“修复”规则。

### 4.4 技能文本到效果结构

`tools/extract_effects.py` 读取技能原文，识别固定模式，将结果写入 `data/mechanics/effects.json`。每个结构化效果保留原始文本，例如：

```json
{
  "kind": "coin_power",
  "value": 1,
  "condition": {
    "source": "target",
    "statuses": ["Sinking", "Butterfly", "Butterfly"],
    "gte": 6
  },
  "raw": "If the sum of the target's [Sinking] and both [Butterfly] is 6 or higher, Coin Power +1"
}
```

解析器必须区分：

- `[On Use]`、`[Before Attack]`、`[On Hit]`、`[Heads Hit]`、`[Attack End]`；
- `[Clash Win]` 和 `[Clash Lose]`；
- 固定门槛和按状态数量缩放；
- `[On Hit without Cracking]` 和普通 `[On Hit]`；
- 当前币、末币、复用币和每回合限制；
- Potency、Count、Stack 三种不同数值分量。

无法识别的文本不会被删除，而是写入对应技能的 `unmodeled` 列表。`docs/COVERAGE.md` 统计已建模和未建模条目，严格模式通过 `strict_blockers()` 阻止不完整技能静默运行。

### 4.5 敌人行动脚本

`tools/build_enemy_scripts.py` 将 Imago 的时间状态、血线和三回合行为模式生成到 `data/mechanics/enemy_scripts.json`，并对照日文资料检查：

- 六个 Skill Slot；
- Past、Present、Future 三种时间状态；
- 小、中、大技能；
- 66% 和 33% 血线后的不同模式；
- 第三回合的空槽和终结技能。

脚本构建阶段就进行交叉检查，资料页面发生变化时尽量让构建失败，而不是悄悄改变战斗。

### 4.6 新增机制的固定流程

新增一个机制时，流程是：

1. 找到游戏文本、wiki 页面或缓存资料；
2. 在 `docs/MECHANICS.md` 记录规则、来源和状态；
3. 如果能抽取，给 `tools/extract_effects.py` 增加模式；
4. 如果文本非常特殊，直接为生成数据增加明确结构，但保留 `raw`；
5. 在 `sim/crates/lcb-core/src/effects.rs` 增加数据类型；
6. 在 `battle.rs` 的对应阶段执行；
7. 添加一个以资料为依据的 Rust 回归测试；
8. 重新运行数据流水线和 `cargo test`；
9. 重新检查 coverage 和 strict blockers。

---

## 5. 总体架构

### 5.1 Rust workspace

```text
sim/
  crates/lcb-core/
    src/state.rs       BattleState、Unit、Dashboard、StatusSet、配置
    src/battle.rs      回合、行动、拼点、攻击、状态、敌人行为
    src/damage.rs      伤害公式、抗性和 Offense/Defense Level
    src/effects.rs     结构化效果和效果解释器
    src/scripts.rs     Imago、Pupa、幻影的行动脚本
    src/setup.rs       固定队伍、固定 Wave、campaign 初始化
    src/library.rs     读取生成 data library
    src/ids.rs         正式 ID 和类型化身份
    src/rng.rs         可复现 RNG
    src/replay.rs      replay 结构和转移记录
    src/hash.rs        state hash、search key、transition hash
    src/testsupport.rs 固定硬币、测试构造器
  crates/lcb-cli/
    src/main.rs        inspect、run、JSON stdio 服务
  crates/lcb-py/
    src/lib.rs         PyO3 binding
```

### 5.2 Python 层

```text
python/lcb/
  env.py          LimbusEnv，reset、observe、legal_actions、step_turn
  stdio_client.py JSON stdio 后端客户端
  features.py     state/action encoder
  plans.py        actor 顺序、PlanGenerator、action mask
  search.py       Random、Greedy 等搜索接口
  teacher.py      回合级 Beam Teacher、value、轴检测
  dataset.py      Teacher npz、seed split、decision iterator
  bc.py           Behavior Cloning
  nn.py           NumPy policy/value network 和反向传播
  ppo.py          PPO rollout、joint log-prob、demo update
  dagger.py       DAgger 辅助逻辑
  rewards.py      真实状态奖励和 EpisodeStats
  scenarios.py   real、half、burst、curriculum 场景
  evaluate.py     评测、统计、Wilson CI、replay index
  baselines.py   Random、FirstLegal、Greedy、NeuralPolicy
```

### 5.3 CLI 和后端通道

当 PyO3 扩展可以编译时，Python 通过 `lcb_sim` 直接调用 Rust。扩展不可用时，`lcb-cli serve` 提供 JSON line 协议，支持：

```text
reset
legal_actions
step
submit
clone
state_hash
search_key
unknown_rules
strict_blockers
```

项目后来增加了 PyO3 与 stdio 的 parity 测试，检查 reset 配置、合法动作、状态 Hash、Search Key 和 TurnStats 是否一致。

---

## 6. 模拟器核心的构建顺序

### 6.1 Phase 1：基础状态

最先实现的是能被复制、序列化、哈希和确定性推进的 BattleState：

- HP、max HP、Shield；
- Speed range 和每回合 Speed roll；
- `Sanity::None` 与 `Sanity::Sane { sp }`，避免异想体错误拥有 SP；
- `StatusSet` 的 Potency、Count、Stack；
- 技能牌组、Dashboard、E.G.O 槽位；
- 资源、弹药、Charge、Sin Resonance；
- 当前回合、战斗阶段、胜者；
- RNG 内部状态和 draw count。

这一阶段就加入了 clone isolation、hash 和 serialization 测试。Clone 必须深复制，搜索在 clone 上试验行动不能修改原状态。

### 6.2 Phase 2：技能和牌面

项目没有把每回合技能列表重新随机生成，而是实现了：

- Skill Amount；
- 多个 Dashboard Slot；
- 每槽两个可选技能和一个预览技能；
- 使用技能后轮换；
- 牌组所有复制品放置完后重置抽牌池；
- 额外 Skill Slot 从第二回合开始增加；
- Defense Skill 和 E.G.O 替换底部技能并消耗对应牌；
- Discard 删除可选技能而不删除 preview；
- 技能被取消、目标死亡或使用者混乱时的牌面保留规则。

### 6.3 Phase 3：战斗

随后实现基础拼点、攻击、伤害、抗性、Stagger 和防御技能。这个阶段的目标是使每一枚 Coin 的生命周期明确：

```text
技能选择
  -> Combat Start / On Use
  -> Engage / Clash 配对
  -> Clash 逐轮投币
  -> Before Attack
  -> 每枚 Coin 攻击
  -> On Hit / Heads Hit / On Kill
  -> Attack End
  -> Turn End
```

### 6.4 Phase 4：状态

先实现固定队伍和罗生蝶战斗真正依赖的通用状态，再接入每个状态自己的文本：

- Burn、Bleed、Sinking；
- Butterfly；
- Poise、Rupture、Fragile、Protection；
- Tremor 和 Tremor Burst；
- Damage Up/Down、Power Up/Down；
- Attack Power Up/Down；
- Plus Coin Boost、Minus Coin Drop；
- Offense/Defense Level Up/Down；
- Bind、Haste、Paralyze；
- Charge、Shield、Unbreakable Coin；
- The Living、The Departed、Bullet - Solitude、LCA Fracture Round；
- Temporal Disjunction 和 Imago 时间状态。

状态本身的 Turn Start、Turn End 和 continuous clauses 由 `data/mechanics/status_effects.json` 驱动，而不是在 `battle.rs` 中为每个状态写一份完全独立的猜测逻辑。

### 6.5 Phase 5：E.G.O

E.G.O 层逐步接入：

- 资源成本和资源键；
- Awakening、Corrosion、Overclock；
- SP 费用；
- 普通攻击产生对应罪孽资源；
- Reuse this Coin；
- Attack Weight；
- E.G.O 的目标、附加伤害、Shield 和恢复。

审阅中发现资源键最初存在大小写不一致问题：普通攻击使用小写 `gloom`、`sloth`，E.G.O 成本使用首字母大写。统一成本表和战斗资源键后，普通攻击才能真正让 E.G.O 进入合法动作集合。

### 6.6 Phase 6：Boss 和 Section 5

最后将 Imago 的具体脚本和时间状态放入通用战斗系统：

- 六槽、三回合周期；
- 66%/33% 血线；
- Past、Present、Future Stack；
- Temporal Disjunction；
- 三种时态的 Turn Start 和攻击被动；
- 影蝶的 Segmentation；
- 主敌人胜利判定；
- Pupa 的 Shield、HP floor 和 Encounter End；
- 站点 2-4 影蝶和 campaign carry。

---

## 7. 技能、牌面和行动系统

### 7.1 Action 的身份

最初行动接口只依赖某个槽位，之后发现多个敌人可能拥有相同 slot 编号，因此 `Action::Engage` 改为同时携带：

```text
enemy UnitId + enemy slot
```

这个身份一路保留到：

- Rust submit；
- enemy slot resolution；
- PyO3 JSON；
- stdio JSON；
- Python features；
- replay canonical form；
- search key。

否则“对 enemy slot 0 行动”可能错误地读取另一个敌人的 slot 0。

### 7.2 完整回合 Plan

训练阶段规定每个 step 是完整回合，不是逐角色推进：

```python
obs = env.observe()
legal = env.legal_actions()
plan = [one_action_for_each_actor]
info = env.step_turn(plan)
```

Rust 入口 `Simulator::submit_plan` 做以下检查：

1. 不允许 `Commit` 混入完整 Plan；
2. 同一 actor 不能出现两次；
3. 每个 Action 必须属于当前 `legal_actions()`；
4. 仍有可行动单位未提交时拒绝；
5. 任意失败都恢复 snapshot，状态 Hash 不变。

增量 `submit()` 和完整 `step_turn()` 仍保留，但两条路径必须在相同输入下得到相同状态 Hash。

### 7.3 合法动作是唯一 Mask 来源

搜索、BC、PPO 和评测都不能自己猜动作。所有候选动作都来自 Rust simulator 的 `legal_actions()`。这避免网络输出一个在当前牌面、资源或目标条件下根本不能执行的动作。

### 7.4 牌面错误的发现和修复

早期审阅复现过一个具体错误：选择上方 `next` 技能时，submit 阶段直接覆盖 `current`，结算阶段又错误地下移原来的 `next`，导致未选择的底部卡消失、已使用的上方卡仍然存在。

修复后，提交阶段只记录选中了哪张卡，结算阶段才：

- 消耗被选卡；
- 保留未选卡；
- 从牌组抽新 preview；
- 按真实牌面顺序轮换。

防御技能曾有相同但独立的轮换遗漏，后来通过“防御确实发动后下一回合牌面”的回归测试补上。

---

## 8. 拼点、攻击和伤害

### 8.1 伤害公式

当前规则表记录的主要形式是：

```text
Damage = CoinRoll × (1 + Static) × (1 + Dynamic)
```

引擎执行 floor 和最低伤害限制，并单独处理：

- Sin resistance；
- type resistance；
- Offense/Defense Level；
- Critical；
- Clash count；
- Stagger multiplier；
- Damage Up/Down；
- Fragile、Protection；
- 每个 Coin 自己的伤害 clause；
- Attack Weight 的主目标和副目标。

### 8.2 拼点 power

项目最初错误地把拼点理解为单枚硬币比较，后来依据 EN wiki 和日文战斗说明修为：

```text
Final Power = Base Power + 所有剩余硬币中正面的 Coin Power 之和
Match Power = Final Power + skill level bonus
```

修复同时包括：

- 双方都投掷所有剩余硬币；
- 平局不摧毁任何硬币；
- 有差值时摧毁失败方第一枚剩余硬币；
- 普通攻击拼点使用双方本次技能的 Attack Level，不使用对方 Defense Level；
- Unbreakable Coin 败拼后变为 Cracked Coin，仍然继续攻击；
- Cracked Coin 保留正确的基础威力和投币流程。

### 8.3 攻击累积

单方面攻击和拼点胜利后的攻击都要共用逐币累积：

```text
每枚正面 Coin 的威力 = Base Power + 当前 Coin Power
第 1 枚造成当前累积威力
第 2 枚造成前两枚累积威力
第 3 枚造成前三枚累积威力
```

日文资料中的示例 `4 + 4 x 3` 得到 `8、12、16`，`1 + 6 x 5` 得到 `7、13、19、25、31`。这些示例被用作一致性测试。

### 8.4 防御和拉拼

已接入：

- Guard：第一次被攻击时按 Final Power 获得 Shield；
- Evade：对每个 incoming Coin 比较并可能消耗；
- Counter：对攻击者进行反击；
- Clashable Guard 和 Clashable Counter；
- 速度足够时 Engage 抢敌人 Skill Slot；
- Gregor 的特殊“无视速度拉拼”标记；
- 同一个敌方 slot 多次 Engage 时最后提交者获胜。

完整复查阶段专门检查过“最后拉拼者比原目标慢”的情况，避免测试只覆盖最后提交者恰好也是最快者的简单样例。

### 8.5 Attack Weight

Attack Weight 的设计是：

- 主目标走 Clash 或单方面攻击完整路径；
- 额外目标共享 Coin 的投币结果；
- 每个目标仍独立执行自己的命中、Sinking、Rupture、Guard、Evade 和被动；
- 使用者自己的弹药、自伤和 Reuse 事件只执行一次。

这个规则修复前，副目标只扣裸伤害，漏掉副目标状态和命中效果。对于 Imago 的群攻，这是会改变正常战斗结果的公共错误。

---

## 9. 状态、理智、被动和 E.G.O

### 9.1 Potency、Count、Stack 的统一

审阅中大量错误来自“写入一个字段，读取另一个字段”。因此生成数据为每个状态记录结构：

- Potency 型：Burn、Bleed、Sinking、Rupture、Tremor、Poise 等；
- Count 型：Protection、Fragile、Haste、Bind、Charge 等；
- Stack 型：In the Past、In the Present、In the Future、Temporal Disjunction、Petals、Dazzle 等；
- 双值或特殊型：Butterfly、Unique Ammo。

数值达到 0 时是否删除状态也按状态自己的 expiry policy 执行，不能对所有状态统一使用 `add_count(-1)`。

### 9.2 Sinking

Sinking 的主要路径是：

- 命中时按 Potency 对 SP 或非 SP 目标造成对应效果；
- 每次触发消耗 Count；
- 多枚 Coin 和多目标攻击按实际命中次数触发；
- 新命中施加的 Sinking 不能被同一次命中错误地立即消耗；
- Butterfly 的 The Living 和 The Departed 分别对应 Potency 和 Count；
- 统计真实 Sinking damage 和 Sinking SP damage；
- Echoes of the Manor 对每个合格 Potency/Count gain component 单独掷概率；
- 使用 `StatusSet` 的统一写入边界和 JSON 载入归一化，把 Sinking Potency/Count 限制在默认 Max Value 99；
- 达到上限时，`StatusGainEvent` 记录实际增加量，而不是效果请求量。

奖励不直接奖励增加 Potency 或 Count，避免策略只叠层、不执行收割。训练层也不再直接奖励 Sinking 触发伤害，触发伤害只作为评测统计保留。

### 9.3 理智和 Panic

Sanity 存储范围是 `[-45, 45]`。项目逐步实现了：

- 起始 SP 可配置，默认 45；
- Low Morale；
- Panic；
- -45 SP 的 E.G.O Corrosion；
- Panic Type 的阶段效果；
- E.G.O SP 费用和 Overclock SP 费用仍然执行。

来源中没有明确的部分仍在 `docs/MECHANICS.md` 标记，而不是使用不明概率。

### 9.4 被动层

`data/passives/passives.json` 中的被动按 Combat Start、Turn Start、Turn End、Attack End 和 continuous modifier 执行。审阅发现被动经常“数据存在但没有从真实回合流程调用”，因此后续测试强调：

```text
真实技能选择
  -> 真实提交
  -> 真实阶段
  -> 被动触发
  -> 下一回合状态
```

只手动向 Unit 塞一个状态，不算被动已经接通。

### 9.5 E.G.O 资源和无限资源条件

`BattleConfig::infinite_ego_resources` 的语义是：

| 项目 | 无限资源时是否绕过 |
|---|---:|
| E.G.O 资源是否足够 | 是 |
| E.G.O 资源扣除 | 是 |
| SP 扣除 | 否 |
| Overclock 1.5 倍费用 | 否 |
| 技能效果 | 否 |
| 命中、状态、伤害和目标 | 否 |

这个开关默认是 `false`，只有研究场景显式设置为 `true`。它贯通 Rust 配置、PyO3 reset、stdio reset、Python `Scenario`、Teacher、PPO rollout、PPO validation 和评测 replay。

### 9.6 特别修复的 Rime Shank、Bygone Days 和 Echoes

post-audit 前重点修复了三个训练关键机制：

#### Rime Shank Threadspin IV

- Awakening：5 Sinking Potency、5 Count；
- Corrosion：10 Potency、8 Count；
- Attack Weight 3；
- 目标 HP 高于 50% 时保留技能文本中的增伤条款。

#### Bygone Days Yi Sang

- `N = floor(6 + 1.5 x Gloom Resonance)`；
- 生成 N 次独立的 1 Potency Sinking；
- 每次独立选择一个存活敌人；
- 使用可重放 RNG；
- 每次 gain 生成独立 `status_gain_event`；
- SkillUse 事件记录来源 actor、target、skill id、coin index 和原始文本。

#### Echoes of the Manor

- 每个合格 Potency 或 Count gain component 单独进行 50% 掷骰；
- 成功时记录 `echoes_of_the_manor` 事件；
- 直接和 queued grant 按规则替换当前值；
- Turn End 按规则减少 Potency；
- 对 Bygone Days 的多次独立 gain 做确定性 replay 测试。

---

## 10. 罗生蝶和第五站场景

### 10.1 Imago 行动模式

Imago 的实现包括：

- 六个 Skill Slot；
- 三回合行动循环；
- 每种时间状态的小、中、大技能；
- 66% 和 33% HP 之后的新行动块；
- 第三回合的 idle slots；
- 即使 Stagger 也可以按照文档行动。

### 10.2 时间状态

战斗开始时 Past、Present、Future 各有 Stack。Turn Start 按最大 Stack 激活时间状态，同值时保持当前状态。切换状态增加 Temporal Disjunction。跨过血线时一次性增加各状态 Stack。

不同 Stack 区间提供不同强度的 Potency、Count、Clash Power 和 Final Power 增益。过去、现在、未来分别连接 Burn、Poise、Bleed、Kalpāgni、Smite the Wicked、Bloodflower 和 Imago 的时间被动。

### 10.3 影蝶 Segmentation

影蝶被作为正式 Section 5 Wave 的一部分加入，而不是只把 Imago 单独模拟：

- 每个影蝶 HP 为 1，Speed 固定为 1；
- 主目标命中影蝶时，按 Coin 次数移除对应 Imago 时间 Stack；
- 每名 Sinner 每回合最多因相应机制恢复一次 SP；
- 本回合未被主目标命中时，回合结束可能向 Imago 增加对应 Stack；
- Imago 作为主敌人决定 Wave 是否结束。

### 10.4 Pupa 和 campaign

Pupa 的实现包含：

- Encounter Start Shield；
- HP 不低于 90% 的 floor；
- Shield 破坏后的模式切换；
- The Quickening 的 Encounter End。

站点 2-4 的影蝶和 campaign state 后来加入，Section 5 可以从 campaign 携带 Pupa HP、时间 Stack 和禁用的被动组件。选择事件 UI 没有完整建模，状态由配置和 campaign state 表示。

---

## 11. Replay、Hash、Clone 和严格模式

### 11.1 状态 Hash

状态 Hash 必须覆盖会影响未来战斗的内容，包括：

- 所有 Unit HP、SP、Shield、Stagger、状态；
- Dashboard、牌组、当前技能和 preview；
- E.G.O 资源、弹药、Charge；
- 时态 Stack、Boss 行动脚本位置；
- 共享资源和效果使用次数；
- RNG 内部状态和 continuation；
- 回合和阶段。

战斗 log、警告和训练统计不应污染用于搜索转置的 `search_key`。

### 11.2 Transition Hash

每次完整回合记录：

```text
state_hash_before
state_hash_after
transition_hash
search_key
rng_before / rng_after
turn stats
battle log
```

评测 replay 中还记录：

- seed；
- scenario config；
- 完整 Plan；
- boss Sinking 状态；
- 本回合真实统计；
- 每回合的前后 Hash。

### 11.3 Clone

Teacher 和 DAgger 都在 clone 上试行动。Clone 必须是深复制：

```python
probe = env.clone_state()
probe.step_turn(plan)
```

不能改变原始 env 的牌面、RNG、回合、状态和资源。

### 11.4 Strict mode

严格模式的意义不是“让胜率更高”，而是防止未实现规则静默改变结果。当前严格检查包括：

- 技能 `unmodeled` 阻塞；
- critical Bygone Days 和 Rime Shank preflight；
- unknown rules inventory；
- stdio/PyO3 配置一致性。

仍有一个全局 unknown-rule inventory 尚未彻底完成，必须看 `docs/MECHANICS.md` 的明确列表，不能只看某一份旧的 coverage 摘要。

---

## 12. 第一次完整模拟器审阅

2026-09-20 的第一份专项审阅只看普通战斗中最容易改变结果的五组错误。审阅前已有 90 项 Rust 测试，但这些测试没有覆盖完整的真实技能流程。

### 12.1 E.G.O 资源键错误

普通技能生成小写资源键，E.G.O 成本表读取首字母大写键。即使资源余额设置为 100，E.G.O 也不进入合法动作。统一键格式后补上：

```text
普通攻击产生资源 -> E.G.O 变为可用 -> 正确扣费
```

### 12.2 第一回合被动缺失

原先先 `begin_turn()` 再给 Unit 挂载被动，导致第一回合看不到花札、先锋被动等效果。初始化顺序改为：

```text
创建 Unit
  -> 挂载状态和被动
  -> 初始化战斗规则
  -> begin_turn
```

### 12.3 拼点获胜后的 Attack End 缺失

单方面攻击会调用 `apply_attack_end`，拼点获胜后的攻击没有调用。修复后 Attack End 统一从完成攻击的路径触发，覆盖弹药、Protection、Reload 和其他攻击结束效果。

### 12.4 上方技能牌消耗错误

见第 7 节，提交阶段和结算阶段分离后修复。

### 12.5 状态分量和过期错误

Protection 写入 Potency 但减伤读取 Count，一回合 Haste/Protection 也没有清除。后来建立状态结构和 expiry policy，所有状态消耗走统一路径。

---

## 13. 第二轮、第三轮和完整复查

### 13.1 第二轮复查：四组通用战斗问题

第二轮有 99 项测试通过，但外部探针发现：

1. Sinking Count 到 0 后 Potency 仍然继续触发；
2. 拼点等级错误地比较对手 Defense Level；
3. queued Bind/Haste 在速度计算之后才应用；
4. 防御实际发动后没有正确轮换牌面。

这些问题后来都增加了真实技能回归测试。

### 13.2 第三轮复查：三组更深的结算问题

第三轮有 106 项测试通过，但探针发现：

1. Unbreakable Coin 败拼后的 Cracked Coin 威力和正反面处理错误；
2. 最后提交的 Engage 仍可能被旧目标关系抢走；
3. 同一回合两个 E.G.O 可能重复花掉同一份共享资源。

后续提交分别修复 Cracked Coin、enemy UnitId + slot 以及共享资源 reservation。最终研究场景使用无限 E.G.O 资源，但 SP 和技能结算仍然保留，因此这些修复不能省略。

### 13.3 完整复查：20 组根因问题

在 `a8b1ec7` 基线上进行的完整复查不再只看单个函数，而是沿着完整链路检查：

```text
原始文本
  -> 结构化效果
  -> 选牌
  -> 提交
  -> 拼点
  -> Before Attack
  -> Coin
  -> On Hit
  -> Attack End
  -> Turn End
  -> 下一回合状态
```

主要发现包括：

- `[Clash Lose]` 独立阶段丢失；
- `[On Hit without Cracking]` 被当作普通命中；
- 复合句只抽取前半句；
- 固定门槛被错误当作按状态数量缩放；
- Final Power 没有贯通实际攻击累积；
- Coin 局部增伤在伤害计算之后才生效；
- Stagger 阈值没有消费，阈值单位把百分比和 HP 混用；
- 行动队列没有在混乱、死亡后重新验证；
- Clashable Guard/Counter 被当成普通防御；
- 拼点投币漏触发 Bleed；
- queued effect 和未选择的防御技能错误执行；
- 条件未满足时提前消耗 per-turn/per-encounter 次数；
- 群攻副目标绕过完整命中流程；
- 七个固定身份的关键被动未完全接通；
- E.G.O SP 时序、目标数量和恢复量有误；
- Imago 时态切换晚于时态被动；
- Boss 击杀复用时序错误。

这次复查的重要影响是：项目不再把“coverage extractor 匹配到了文本”当成“技能一定正确”。`unmodeled=0` 只是说明文本没有被遗留在未建模列表中，仍然需要真实状态和真实技能流程测试。

---

## 14. 固定内容修复和 post-audit

### 14.1 数据层修复

post-audit 阶段重新生成当前 Threadspin IV 变体，修复了 Rime Shank、Bygone Days、Echoes 的数据和事件语义。训练前必须重新执行 `tools/build_all.py --skip-fetch`，不能继续使用旧 JSON。

### 14.2 Rust、PyO3 和 stdio 修复

这一阶段完成：

- `BattleConfig.infinite_ego_resources` 全链路；
- PyO3 reset、stdio reset 和 Python scenario parity；
- `Action::Engage` 传递 enemy UnitId 和 slot；
- 原子 `submit_plan` 和 `step_turn`；
- TurnStats；
- `state_hash`、`search_key`、`transition_hash`；
- strict critical preflight；
- clone 和 reset/mask/one-turn parity 测试；
- replay payload 记录完整场景配置。

### 14.3 第一个 PPO 训练错误

第一次 post-audit PPO 的 rollout reset 和 validation reset 没有传播 `infinite_ego_resources`。这使得训练条件和报告中声明的条件不一致。

处理方式不是修改结果，而是：

1. 在 `collect_episode` 中传播 `Scenario.infinite_ego_resources`；
2. 在 `evaluate_argmax` 中传播同一配置；
3. 增加 Python 回归测试；
4. 将旧 PPO run 标记为 superseded；
5. 在相同实验定义下重新训练和评测。

### 14.4 奖励审计错误

奖励最初从所有敌方总伤害推断 Boss HP 下降，因此攻击 1 HP 的幻影也可能被误当作 Imago damage credit。修复后 `_boss_hp()` 只选择最大 `max_hp` 的敌人，`compute_reward()` 只奖励实际主 Boss HP delta。

这一步很重要，因为奖励本身不能硬编码某个路线，但必须能从真实模拟状态得到正确归因。

### 14.5 人类设置 Oracle

新增 `search/run_oracle.py` 作为规则和关键设置的前向模型检查器：

```text
Turn 1: Ishmael Bygone Days Overclock + Rodion Rime Shank Overclock
Turn 2: Yi Sang Bygone Days
之后: Greedy fallback
```

Oracle 不是训练策略，也不加入 Teacher 数据。它只验证固定技能、E.G.O、资源、Sinking 事件和 replay 是否连通。burst 场景可以在三回合左右清除，real HP 也能执行完整三回合而不报接口错误。

### 14.6 本次修订：Sinking 上限和训练奖励再审计

本次修订追踪了一个在旧 Teacher replay 中可直接观察的规则违反：Sinking Potency
曾经超过默认 Max Value 99。旧的 half T1 数据最高到 146，full T1 数据最高到
203。根因不是某一张 E.G.O 的特例，而是状态容器的通用 `add_potency()`、直接
`set()` 和载入路径都没有把状态的默认上限作为最后边界。

修复集中在 `sim/crates/lcb-core/src/state.rs`：`Sinking` 的增量、直接写入、
`clamp_*` 和反序列化都经过同一个 99 上限；`record_sinking_gain()` 同时把
事件中的 Potency/Count delta 改为实际成功写入的数量，并且不让“没有真正获得
Potency”的 gain 继续触发 Echoes 的额外 Count。来源是缓存的 wiki.gg
`Status Effects` Overview，其中说明未另列上限的状态默认 Max Value 为 99。

同一轮审计还删除了 `python/lcb/rewards.py` 中的
`+0.02 * sinking_damage`。这是训练 shaping reward，不是战斗规则；保留
`sinking_damage` 统计不会改变模拟器。因奖励函数同时被 Teacher、PPO、DAgger
和评测调用，本次对照重新生成了 matched T1 Teacher、BC 和 3000-rollout
Teacher-demo PPO 数据，而不是把旧数据悄悄混入新结论。

---

## 15. 训练系统的设计

### 15.1 环境 step 的定义

训练环境的一个 step 是完整一回合：

```text
reset(seed)
  -> observe
  -> 固定 actor 顺序生成所有行动
  -> 一次性提交 Plan
  -> Rust 完整结算
  -> 下一状态、reward、done、info
```

网络可以逐个 actor 生成动作，但生成过程中不能推进模拟器，也不能读取“前面角色已经造成的中间伤害”。

### 15.2 Observation

`observe()` 读取的内容包括：

- Imago 和所有敌人的 HP、max HP、Shield、阶段、技能槽；
- Boss Sinking、时间状态和 Temporal Disjunction；
- 每个 Sinner 的 HP、SP、Speed、Stagger、存活、状态和牌面；
- E.G.O 槽位和资源；
- 罪孽和伤害类型抗性；
- 当前回合、攻击顺序、RNG continuation；
- 当前合法动作的可编码信息。

战斗 log 不进入 observation，避免 log 长度和文本内容泄漏未来信息。log 只用于 replay、审计和 post-hoc axis label。

### 15.3 Features

当前稳定离散编码为：

```text
state_dim  = 1794
action_dim = 44
```

Action feature 包括：

- actor 位置；
- 技能 ID 对应的结构化数值；
- Base Power、Coin Power、Coin 数量；
- Attack Weight；
- 罪孽、伤害类型、是否防御、是否 E.G.O；
- 目标 HP、抗性、Sinking、时间 Stack；
- 前面 actor 已选的动作上下文。

编码不依赖“技能名称中出现某个词”来判断策略。特定技能 ID 只在模拟器数据和动作身份中存在，不能变成奖励硬编码。

### 15.4 Reward

当前正式奖励定义：

```text
击杀 Boss                       +1000
每回合                         -10
每名我方角色死亡                -30
我方团灭                       -1000
主 Boss 实际 HP 下降            +0.01 x delta
```

早期版本曾加入 `+0.02 x sinking_damage`。奖励审计后删除了这一项：
`sinking_damage` 仍从 Rust `TurnStats` 进入 `EpisodeStats` 和评测报告，但
`compute_reward()` 不再读取它。这样 PPO、Teacher 的回合排序和所有训练阶段都
必须依靠真实 Imago HP 下降、存活、击杀和回合成本获得 credit，而不是把触发
伤害本身当成独立目标。不直接奖励“增加 Potency/Count”。

`TurnStats` 由 Rust 在实际效果发生的位置累计，不由 Python 估算：

- `damage_to_enemies`；
- `damage_to_allies`；
- `status_damage`；
- `sinking_damage`；
- `sinking_sp_damage`；
- `sinking_triggers`；
- `skill_uses`；
- `ego_uses`；
- `deaths`。

---

## 16. 阶段 A：Random、Greedy、Beam Teacher

### 16.1 基线顺序

实现顺序是：

```text
Random
  -> FirstLegal
  -> Greedy
  -> Beam Teacher
  -> BC
  -> PPO
```

MCTS 没有实现，因为计划明确要求先完成模拟器和回放，再考虑更大搜索。`lcb.search.mcts_turn` 有意抛出未实现错误，而不是提供一个未经验证的伪实现。

### 16.2 Greedy

Greedy 不使用手写“沉沦路线”，而是对每个候选通过真实模拟器做当前回合前瞻，再按即时收益和安全性排序。完整 Plan 评分不能只用公式估算。

### 16.3 Beam Teacher 的搜索节点

Teacher 节点是完整回合后的状态：

```text
state(t)
  -> 生成完整 Plan
  -> clone 上 step_turn
  -> state(t+1)
  -> 继续搜索
```

固定 actor 顺序生成 Plan，内部有一层 actor-level candidate beam，外层保留多个完整回合节点。每个节点保存：

- 当前 clone env；
- 完整 Plan；
- 父节点路径；
- 状态 Hash；
- RNG continuation；
- 累计 reward；
- leaf value。

### 16.4 Teacher value

正式 emergence search 使用 generic combat objective，不把 Rime Shank、Sinking、Harmony 或 Solemn Lament 当成硬编码动作奖励。默认权重为：

```text
win             1000
remaining boss HP 100
ally damage       30
ally death        60
turn               0
sinking readiness  0
ego readiness      0
```

最终排序是词典序：

1. 已胜利优先；
2. 胜局击杀回合更少优先；
3. 未胜局 Boss 剩余 HP 更少优先；
4. 再比较累计过程价值。

### 16.5 注册 Teacher 预算

```text
T0: horizon=3, plan_width=16, candidate_cap=4, turn_width=3, rollout_width=4
T1: horizon=4, plan_width=32, candidate_cap=6, turn_width=4, rollout_width=4
T2: horizon=5, plan_width=64, candidate_cap=8, turn_width=5, rollout_width=4
```

最终主要实验使用 T1：

```text
horizon=4
plan_width=32
candidate_cap=6
turn_width=4
rollout_width=4
score_mode=heuristic
max_turns=30
```

### 16.6 Teacher 数据格式

每个 seed 单独保存 `.npz`，并生成 `teacher_index.json`。每个 Teacher decision 包含：

- turn；
- observation；
- 每个 actor 的候选集；
- 教师选中的真实动作；
- label 在候选集中的位置；
- Teacher value；
- seed；
- episode 是否胜利；
- kill turn；
- post-hoc axis label。

胜局还保存 replay JSON，包含逐回合 state hash、transition hash、真实 TurnStats 和 Boss Sinking。

---

## 17. 阶段 B：Behavior Cloning

### 17.1 模型

BC 使用纯 NumPy 的共享 state encoder 和候选动作 scorer，不依赖 PyTorch：

```text
h        = tanh(state W1 + b1)
z_i      = tanh([h, action_i] W2 + b2)
logit_i  = z_i W3 + b3
```

固定 actor 顺序和前面 actor 的选择被编码到动作特征中，因此一个共享 scorer 可以形成自回归 Plan，而不必为七个 Sinner 写七套独立网络。

当前网络参数量：

```text
124033
```

### 17.2 Loss 和合法动作

每个 actor 的 loss 只在自己的合法候选动作集上计算。教师标签不是“某个宏动作”，而是候选集中的真实 `UseSkill`、`Defend`、`E.G.O` 或目标动作。

样本权重：

- 普通和失败样本保留；
- 胜局增加权重；
- 击杀更快的胜局增加权重；
- 轴事件满足时可以增加有限权重；
- 但正式最终 T1 搜索使用 generic objective，不能用特定路线硬编码标签。

### 17.3 seed split

BC 按 episode seed 切分，而不是按逐 actor 行随机切分：

```text
75% episode seeds -> train
25% episode seeds -> validation
```

同一 episode 的多个 actor decision 不会被拆到训练和验证两边。

### 17.4 半血 BC

输入：`data/teacher_t1_half_500/`，500 个 Teacher episode。

- 训练决策：3050；
- 验证决策：1030；
- epochs：20；
- validation accuracy 约 `0.579`；
- checkpoint：`models/bc_hp_half_t1_500.npz`；
- 最终 holdout：93/500，18.6%。

### 17.5 完整血量 BC

输入：`data/teacher_t1_full_500/`，500 个 Teacher episode。

- 训练决策：4417；
- 验证决策：1457；
- epochs：20；
- validation accuracy 约 `0.574`；
- checkpoint：`models/bc_hp_full_t1_500.npz`；
- 最终 holdout：1/500，0.2%。

### 17.6 Action agreement 诊断

只看 validation episode seed：

| 场景 | Top-1 | Top-3 | 最弱 actor 位置 |
|---|---:|---:|---:|
| Half T1 | 78.13% | 99.46% | actor 2，60.60% Top-1 |
| Full T1 | 76.46% | 99.27% | actor 2，58.26% Top-1 |

这个结果说明一个重要问题：逐 actor 看起来有较高准确率，不等于完整七人 Plan 的成功率高。一个 actor 的小偏差可能改变后续 actor 的合法候选、拼点结果、死亡和 E.G.O 时机。

---

## 18. 阶段 C：PPO 和 Teacher demonstration replay

### 18.1 PPO 时间单位

PPO 的一个 environment step 仍然是完整回合。一个回合的 joint log-prob 是七个 actor log-prob 的和：

```text
log P(full plan) = sum_i log P(action_i | state, previous actions)
```

advantage、return 和 value target 也按回合计算，不能把同一回合七个 actor 当作七个独立环境时间步。

### 18.2 PPO 更新

最终训练使用的主要配置：

```text
learning_rate       0.001
clip                0.2
value_coef          0.5
epochs_per_iteration 3
demo_batch_decisions 32
demo_updates         1 per iteration
gamma               1.0
```

每次 iteration：

1. 用训练 seed 收集完整回合 rollout；
2. 用真实 `step_turn` 得到 reward；
3. 计算 normalized advantage；
4. 进行 PPO clipped policy update；
5. 进行 value update；
6. 如果配置了 demonstrations，随机采一小批 Teacher decisions 做 BC constraint update；
7. 在独立 validation seeds 上运行 argmax policy；
8. 记录 validation win rate 和 median kill turn；
9. 保存 validation 最好的参数，而不是无条件保存最后一轮。

加载 BC checkpoint 后，训练 CLI 的 learning rate 和 value learning rate 会重新使用当前配置，避免从 checkpoint 加载旧学习率。

### 18.3 Teacher demonstration replay

PPO rollout 之外，每轮附加一次 Teacher BC 更新：

```text
real PPO environment update
  + small batch Teacher demonstration constraint
```

参数：

```bash
--demo-data data/teacher_t1_half_500
--demo-updates-per-iteration 1
--demo-batch-decisions 32
```

该机制不是把 Teacher 变成宏动作，也没有跳过 simulator。它只让网络在真实合法候选集上保留一部分 Teacher 行为分布。

### 18.4 训练 seed 和 validation seed

模型 JSON 会记录：

- checkpoint；
- out checkpoint；
- scenario；
- training seed range；
- validation seed range；
- iteration 数；
- episodes per iteration；
- epochs per iteration；
- Teacher data 路径；
- demonstration decision 数；
- best validation iteration。

早期错误 run 被标记为 superseded，不与修复后的 run 合并统计。

---

## 19. DAgger：为什么做，怎么做，结果如何

### 19.1 动机

BC 只看到 Teacher 访问的状态。网络一旦选错一个动作，可能进入 Teacher 数据中没有出现的状态，之后连续出错。这是典型的 covariate shift，因此实现了 DAgger：

```text
策略自己 rollout
  -> 在策略实际访问到的 clone state 上查询 Teacher
  -> 保存 Teacher label
  -> 合并原始 Teacher 数据重新训练 BC
```

Teacher 查询在 clone 上进行，不改变策略 rollout 的原始环境。

### 19.2 实现要求

DAgger 依赖以下 parity：

- BattleState clone/load-state；
- observation 一致；
- legal action mask 一致；
- state hash/search key 一致；
- invalid plan 的 error path 一致；
- 同一 clone 上 Teacher plan 可以 replay。

当前已经覆盖 reset、mask、one-turn parity；更广泛的 clone/load-state/error-path parity 仍是后续工作。

### 19.3 实验

Half 场景的固定诊断 seed band 为 `39001-39100`：

| 方案 | 同一诊断带胜场 |
|---|---:|
| 原始 T1 BC | 23/100 |
| Sampled DAgger round 1 | 6/100 |
| Argmax DAgger round 1 | 21/100 |
| Argmax DAgger round 2 | 19/100 |

对应数据：

```text
data/dagger_t1_half_r1_200/
data/dagger_t1_half_argmax_r1_200/
data/dagger_t1_half_argmax_r2_200/
```

完整血量 DAgger round 1 没有正向胜局。DAgger 路径本身完成并通过审计，但这几种混合没有提升固定诊断带，因此保留为负向 ablation，没有用于最终 checkpoint。

---

## 20. HP 场景和 curriculum

### 20.1 场景定义

```text
burst          enemy_hp_scale=0.08, max_turns=12, Imago 约 2049 HP
short          enemy_hp_scale=0.20, max_turns=20, Imago 约 5123 HP
half           enemy_hp_scale=0.50, max_turns=30, Imago 约 12808 HP
curriculum_065 enemy_hp_scale=0.65, max_turns=30
curriculum_080 enemy_hp_scale=0.80, max_turns=30
real           enemy_hp_scale=1.00, max_turns=30, Imago 25616 HP
```

所有正式研究场景使用：

```text
strict=true
infinite_ego_resources=true
```

### 20.2 为什么需要 HP 场景

早期 real HP 测试中，七人队伍通常每回合造成约 200-400 伤害，在约第 9 回合附近团灭，而 Imago 数据中的 HP 是 25616。若没有终局胜利，就无法有意义地统计 kill turn、short-win rate 或 best-of-N clear rate。

因此：

- `enemy_hp_scale` 只用于建立可达的研究场景；
- 只改变 HP，不改变技能、状态、抗性或行动规则；
- 每个报告写入 scale；
- real HP 必须单独评测，不能用 half/burst 结果代替。

### 20.3 Curriculum 训练链

最终 full HP curriculum 使用：

```text
0.50 -> 0.65 -> 0.80 -> 1.00
```

具体链路：

```text
models/ppo_hp_half_t1_demo_3000.npz
  -> models/ppo_curriculum_065_1000.npz
  -> models/ppo_curriculum_080_1000.npz
  -> models/ppo_curriculum_1000.npz
```

阶段配置：

| 阶段 | 训练 seeds | validation seeds | rollout | Teacher data | 训练结果 |
|---|---|---|---:|---|---:|
| 0.65 | 46001-47000 | 48001-48100 | 1000 | `teacher_curriculum_065_200` | 845/1000 |
| 0.80 | 47001-48000 | 48101-48200 | 1000 | `teacher_curriculum_080_200` | 723/1000 |
| 1.00 | 48001-49000 | 48201-48300 | 1000 | `teacher_t1_full_500` | 573/1000 |

最终 checkpoint 不在这些训练/validation seeds 上做最后报告，而是在新的 `43001-43500` holdout 上评测。

---

## 21. 评测协议和指标

### 21.1 基线策略

评测实现：

1. Random：从合法动作随机选择；
2. FirstLegal：每个 actor 取第一个合法动作；
3. Greedy：真实模拟当前候选，使用 `cap=6`；
4. Teacher：T0/T1 回合级 Beam；
5. BC：从 BC checkpoint 使用 argmax；
6. PPO：从 PPO checkpoint 使用 argmax。

### 21.2 评测字段

每个 episode 记录：

- seed；
- winner；
- won；
- kill turn；
- turns；
- boss HP start/left；
- damage to enemies/allies；
- Sinking damage 和 triggers；
- deaths；
- survivors；
- E.G.O uses；
- axis labels；
- replay 和 transition hashes。

汇总包括：

- 胜率；
- Wilson 95% CI；
- 成功局平均、median、P10、P25 kill turn；
- fastest kill；
- short-win rate；
- Boss HP median；
- average survivors；
- average Sinking damage；
- long-tail failure rate；
- best-of-N；
- restart-aware clear rate。

### 21.3 Axis detector

轴检测只读取真实 replay 状态和 `TurnStats`，不读取技能名称来制造奖励。初版判定：

1. 第一次 Harmony/Solemn Lament 前，Boss 已达到 Sinking Potency >= 5 或 Count >= 3；
2. 两个 E.G.O 在三回合窗口内配合，或至少存在可识别爆发；
3. E.G.O 后两回合的 Sinking trigger damage 大于之前窗口。

后续报告还增加：

- `known_setup_like`：是否接近手写的 Ishmael/Rodion/Yi Sang setup；
- `direct_burst`：是否出现真实 burst；
- peak Sinking Potency/Count。

最终 PPO 的 `axis_ok_rate` 为 0，但 `direct_burst_rate` 为 1。也就是说最终高胜率策略并没有被证明复现初始定义的那条 `Sinking -> Harmony/Solemn Lament -> trigger` 标签轴。它在当前模拟器和无限资源条件下学出了另一种直接爆发行为。

---

## 22. 所有主要实验的时间线

### 22.1 初始 2026-09-19：先把模拟器做完

第一天的提交集中在 Rust core、数据 pipeline 和固定内容：

| Commit | 内容 |
|---|---|
| `ce39365` | 建立 Rust core、data pipeline、PyO3 bindings |
| `a97a8f4` | 修正拼点、Dashboard，加入防御技能 |
| `0f87f22` | 实现 Imago action pattern、时间状态和状态被动 |
| `c91a362` | 加入固定内容依赖的通用状态机制 |
| `f5321d0` | 实现 Pupa 和 Encounter End 技能 |
| `b51f63c` | 抽取和评估复合技能条件 |
| `a69bf99` | 修复状态写入 0、效果限制和反击 |
| `3d36932` | 加入 Sin Resonance 和 resonance payoff |
| `d98d800` | 实现三层牌面和 Discard |
| `59eeb43` | 扩展 effect extractor，加入弹药缩放和 crit modifier |
| `76a7cff` | 修复条件分组、Boss scaling、SkillHint |
| `036ded8` | 加入站点 2-4 幻影和 campaign layer |
| `8d74aad` | 补完 Section 5 Imago 剩余条款 |
| `7676ed6` | Imago 110 条款完成初版建模 |
| `f9e9bbf` | Attack Weight 使 Imago AoE 命中多个槽位 |
| `9a6397f` | 补固定七身份和 E.G.O 的剩余效果条款 |
| `3febb06` | 执行之前被引擎跳过的 Skill phases |
| `4cb39ce` | 加入 Passive layer、抽取和 coverage |
| `8dbcfae` | 加入 Sanity、Low Morale、Panic 和 Panic Types |
| `6632339` | 让状态行为来自状态自己的文本 |
| `4850797` | 加入 Clash-end 和 On-hit status rider |
| `964a45a` | 加入 -45 SP 的 forced E.G.O Corrosion |
| `bc7cd11` | 在 README 中记录规则层 |

这一阶段的原则是先让固定内容可执行，再开始搜索。项目计划明确要求模拟器完成前不要做大规模搜索。

### 22.2 2026-09-20：修正常规战斗流程

| Commit | 内容 |
|---|---|
| `fe956fa` | 修复 Reuse reroll 和它暴露的 E.G.O 机制 |
| `90e6734` | 防止技能机制静默丢失 |
| `4b268b0` | 修正按缺失 HP 的 Reuse 缩放 |
| `6f63cce` | 对未处理 effect kind 加 runtime guard |
| `c388bcf` | 修复 Support Passives、Butterfly 和 Corrosion 时序 |
| `4aa4595` | 修正 Butterfly damage cap 的对象 |
| `9ad486f` | 修复第一次专项审阅的五组问题 |
| `152c127` | 在 MECHANICS 和 STATUS 记录审阅修复 |
| `4a1bb54` | 修复败拼后的 Unbreakable Coin 仍然行动并决定 Attack End |
| `99fb464` | 只将条款指定且满足血线的 Coin 转为 Unbreakable |
| `b05e130` | 修复第二轮复查的四组问题 |
| `4ee489d` | 统一清理其他路径消耗到 0 的状态 |
| `6fbbff9` | 允许快速 Sinner 拉敌方 Skill 进入 Clash |
| `5834062` | 最后提交的 Engage 获得敌方槽位归属 |
| `a8b1ec7` | 修复 Cracked Coin power 和 stolen chain |
| `7eb91cf` | 完整复查第 1 轮，覆盖文本、威力、Stagger、状态和顺序 |
| `c4e5424` | 完整复查第 2 轮，覆盖防御、splash、upkeep 和限制 |
| `adb6bba` | 完整复查第 3 轮，覆盖共鸣、E.G.O timing、Boss phase 和 coverage |

### 22.3 2026-09-21：固定内容闭环和训练基础

| Commit | 内容 |
|---|---|
| `4da946a` | Unique Ammo 和身份核心被动 |
| `4c51610` | Alternate Skills、Ryoshu 无拼点攻击和 Hanafuda Hand |
| `ad19628` | 把 Greedy sanity check 改为真正 multi-seed comparison |
| `230934f` | Kōzan、Bright 和 cancel gate |
| `c130dd7` | 关闭固定内容最后一批 unmodelled clauses |
| `263541a` | Rodion Plus/Minus Coin Skill |
| `b178630` | 在 STATUS 顶部记录 coverage 状态 |
| `5affd00` | Cracked Coins、Before Attack、critical modifier |
| `3e0c559` | 生成七 Sinner 对 Imago 的 sample battle |
| `b4f620d` | 默认从 45 SP 开始并归档完成的规划文档 |
| `990b4f9` | 刷新 45 SP sample battle 和 damage accounting |
| `102c693` | 修 Koi-Koi 必须先于 Kōzan 的回合时序 |
| `960338d` | Section 5 Wave 加入 Imago 和三只幻影 |
| `cf1964e` | 完成训练栈：Teacher、BC、PPO、evaluation |

### 22.4 2026-09-25：post-audit repair

| Commit | 内容 |
|---|---|
| `51e7fb9` | 完成 post-audit simulator repair 和 evaluation |
| `697dc42` | 加入 post-audit Teacher replay JSON artifacts |
| `4936ea6` | 将 infinite resources 传播到 PPO rollout |
| `68fcff7` | 运行独立 100-episode PPO HP experiments |

这一阶段的关键不是“多跑几次”，而是先修实验条件传播和奖励归因，再重新生成数据。

### 22.5 2026-09-26：场景专属 PPO、DAgger、curriculum

| Commit | 内容 |
|---|---|
| `41b804b` | Teacher demonstration constraint，完成 scenario-specific PPO |
| `5cadfbf` | 加入 T1 action diagnostics、Wilson CI、DAgger audit 和 HP curriculum |

最后一个 commit 包含代码、训练元数据、Teacher 索引、DAgger 索引、报告、replay 和文档更新，共 392 个文件，NPZ checkpoint 仍按 `.gitignore` 不提交。

### 22.6 本次修订的 matched reward ablation

本次对照沿用 T1、相同的 Teacher seed、PPO seed 和 500-seed holdout，以隔离
“移除 Sinking 触发奖励”的影响。Teacher 也必须重新生成，因为 BeamTeacher 的
每个候选回合通过 `compute_reward()` 排序。结果和完整路径见
`reports/ppo_hp_sinking_reward_ablation.json`。这轮没有重新跑旧的
0.50 -> 0.65 -> 0.80 -> 1.00 curriculum，因此不把新 direct PPO 数字写成新的
curriculum 结论。

---

## 23. 当前结果与历史 final baseline

本节的 23.2-23.5 保留了上一轮 T1/curriculum 的完整 baseline，方便复核
`reports/ppo_hp_research_t1_curriculum.json`。它们属于 pre-cap / pre-reward-removal
实验；本次 post-fix matched direct ablation 在 23.6 单独列出，不能把两轮数字
拼接成一个结论。

### 23.1 实验公共条件

```text
strict=true
max_turns=30
infinite_ego_resources=true
final half seeds = 42001-42500
final full seeds = 43001-43500
final episodes = 500 per policy
```

Half：`enemy_hp_scale=0.5`，Imago 约 12808 HP。

Full：`enemy_hp_scale=1.0`，Imago 25616 HP。

### 23.2 Teacher

| 场景 | Teacher budget | episodes | wins | win rate |
|---|---|---:|---:|---:|
| Half | T1 | 500 | 272 | 54.4% |
| Full | T1 | 500 | 2 | 0.4% |
| Curriculum 0.65 | T1 | 200 | 17 | 8.5% |
| Curriculum 0.80 | T1 | 200 | 4 | 2.0% |

Teacher 数据路径：

```text
data/teacher_t1_half_500/
data/teacher_t1_full_500/
data/teacher_curriculum_065_200/
data/teacher_curriculum_080_200/
```

### 23.3 Half final holdout

| 策略 | wins | episodes | win rate | Wilson 95% CI | median kill turn |
|---|---:|---:|---:|---|---:|
| T1 BC | 93 | 500 | 18.6% | 15.4%-22.2% | 8 |
| Plain PPO | 119 | 500 | 23.8% | 20.3%-27.7% | 8 |
| PPO + Teacher demo | 495 | 500 | 99.0% | 97.7%-99.6% | 5 |

Plain PPO 使用 1000 rollout。Teacher-demo PPO 使用 3000 rollout，每轮 3 个 PPO epoch，每轮一次 Teacher BC constraint update。

### 23.4 Full final holdout

| 策略 | wins | episodes | win rate | Wilson 95% CI | median kill turn |
|---|---:|---:|---:|---|---:|
| T1 BC | 1 | 500 | 0.2% | 0.04%-1.12% | 12 |
| Direct PPO + Teacher demo | 212 | 500 | 42.4% | 38.1%-46.8% | 7 |
| Curriculum PPO | 406 | 500 | 81.2% | 77.5%-84.4% | 8 |

Curriculum PPO 使用：

```text
0.50 -> 0.65 -> 0.80 -> 1.00
```

这组数字属于 **pre-cap / pre-reward-removal 的历史结果**。本次修订只完成
matched direct T1 ablation，没有重新生成 curriculum checkpoint；因此旧 curriculum
不能和 `reports/ppo_hp_sinking_reward_ablation.json` 中的 post-fix direct PPO
混成一张“最终”表。

对应 checkpoint：

```text
models/ppo_curriculum_065_1000.npz
models/ppo_curriculum_080_1000.npz
models/ppo_curriculum_1000.npz
```

Full curriculum 的 `restart_aware` 指标中，8-turn threshold 的单次 clear rate 为 0.66，N=5 窗口 clear rate 约 0.9899，N=10、20、50 窗口均为 1.0。单次胜率和重开窗口胜率分开报告，不能把后者写成单次稳定能力。

### 23.5 交叉比较

两份 full holdout 使用同一 seed band `43001-43500`，可以做 paired outcome comparison：

- direct PPO 与 curriculum PPO 同时胜利：179/500；
- direct PPO 胜、curriculum PPO 负：33/500；
- curriculum PPO 胜、direct PPO 负：227/500；
- 两者都负：61/500。

因此 curriculum 在同一 holdout 上不是由于换了一批更容易的 seed 才提升。

Half plain PPO 和 Teacher-demo PPO 也使用同一 `42001-42500` band：

- 两者同时胜利：119/500；
- plain PPO 胜、demo PPO 负：0/500；
- demo PPO 胜、plain PPO 负：376/500；
- 两者都负：5/500。

这说明 demonstration replay 对最终 half 策略产生了明显的行为改变。

### 23.6 本次修订后的 matched direct T1 ablation

以下表格仍使用原来的 half `42001-42500` 和 full `43001-43500` holdout，
但新模型、Teacher 数据和 PPO 训练都使用了修订后的代码和奖励。旧值只作为
matched baseline，并不与新值合并。

| 场景 / 策略 | 旧版本 | 修订后 | 变化 |
|---|---:|---:|---:|
| Half T1 Teacher | 272/500 (54.4%) | 310/500 (62.0%) | +38 |
| Full T1 Teacher | 2/500 (0.4%) | 1/500 (0.2%) | -1 |
| Half BC holdout | 93/500 (18.6%) | 89/500 (17.8%) | -4 |
| Full BC holdout | 1/500 (0.2%) | 1/500 (0.2%) | 0 |
| Half direct PPO + Teacher demo | 495/500 (99.0%) | 168/500 (33.6%) | -327 |
| Full direct PPO + Teacher demo | 212/500 (42.4%) | 1/500 (0.2%) | -211 |

旧数据的最高 Sinking Potency 分别为 half 146、full 203；修订后 Teacher 和
holdout 的逐局峰值均不超过 99。新 BC validation accuracy 为 half 57.05%、
full 57.17%，与旧版 57.56%、57.38% 同一量级；主要变化出现在 direct PPO 的
训练目标和结果，而不是一个隐藏的 validation accuracy 变化。

这轮结果说明删除 shaping reward 不是一个只改报告的清理动作：在当前 3000
rollout、每轮一次 Teacher demonstration update 的 direct PPO 配置下，策略
性能明显下降。它也没有证明“奖励项应该恢复”；相反，它说明如果要在没有
Sinking 直接奖励的约束下恢复 full curriculum 能力，需要另行设计并预注册
curriculum 复训，不能引用旧 checkpoint 冒充 post-fix 结果。

---

## 24. 结果应该怎样解释

### 24.1 可以确认的结论

1. 模拟器已经提供了可执行的完整回合接口、严格合法性检查、clone、hash、replay 和真实状态统计。
2. T1 Teacher 在 half HP 上可以找到大量胜局，在 full HP 上只能偶尔找到胜局。
3. 单独 BC 的逐 actor 准确率不等于完整战斗胜率。
4. Half 场景中，PPO + Teacher demonstration replay 显著超过 plain PPO。
5. Full 场景中，直接从 full BC/PPO 训练可以达到 42.4%，预注册 curriculum 可以达到 81.2%。
6. 最终 holdout 的 full curriculum 胜局在同一 500 seed band 上稳定出现，而不是单条 replay 的偶然结果。
7. PPO 结果没有通过硬编码 Rime Shank、Sinking 或特定 E.G.O 路线得到。奖励和动作选择都来自真实模拟状态及合法动作集合。

### 24.2 不能确认的结论

1. 不能说完整血量 PPO 已经证明游戏中的真实阵容必然可稳定通关。实验同时使用了无限 E.G.O 资源和当前模拟器的已知近似。
2. 不能说最终网络发现了初始计划定义的标准 Sinking -> Harmony/Solemn Lament -> trigger 轴。当前 `axis_ok_rate=0`，虽然 `direct_burst_rate=1`。
3. 不能把 BC 约 57%-58% 的 validation accuracy 解释成 57%-58% 的战斗胜率。
4. 不能把 best-of-N clear rate 写成单次 policy win rate。
5. 不能把旧版 burst、post-audit、scenario-specific 和 T1/curriculum 数据混合成一个总结果。
6. 不能忽略 seed protocol 的局部重叠问题，见下一节。

### 24.3 早期“full HP 不可达”结论如何更新

早期真实 HP 0/50、0/100 和 full Teacher 0/200 说明当时的搜索预算、策略初始化和奖励条件没有找到可行路线。它们不能继续被写成“在任何后续方法下都不可达”。

T1 Teacher 找到 2/500，direct PPO 达到 212/500，curriculum 达到 406/500。因此更准确的表述是：

> 在早期配置和搜索预算下没有发现稳定 full HP 路线。加入 T1 数据、demo constraint 和 HP curriculum 后，在当前显式实验条件下发现了可重复的 full HP 策略。

### 24.4 本次修订的解释边界

本次 cap/reward ablation 可以确认两件事：模拟器状态不再产生超过 99 的 Sinking
Potency，且移除直接触发奖励会显著改变 Teacher-demo PPO 的学习结果。它不能
确认修订后的 direct PPO 是最优训练配置，也不能把旧 curriculum 的 81.2% 迁移
到新奖励。下一步若要保留 curriculum 作为正式 post-fix 结论，必须重新生成
0.65/0.80 Teacher 数据、重新训练每一级 checkpoint，并使用新的独立 holdout。

---

## 25. 复现当前流程

以下命令假定已经在仓库根目录，且 Python、Rust、PyO3 和 NumPy 环境可用。

### 25.1 构建数据

```bash
python3 tools/build_all.py --skip-fetch
```

需要刷新原始资料时，再执行 fetch 脚本。不要直接编辑 `data/_raw/` 或生成 JSON。

### 25.2 构建 Rust 和 Python binding

Linux/macOS 示例：

```bash
cd sim
cargo test
PYO3_PYTHON=$(python -c "import sys; print(sys.executable)") cargo build -p lcb-py --release
```

Windows 环境需要将生成的 DLL 复制为 Python 扩展名，例如：

```text
sim/target/release/lcb_sim.dll
  -> python/lcb/lcb_sim.pyd
```

### 25.3 Teacher 数据

Half T1：

```bash
python search/run_teacher.py \
  --scenario half \
  --budget t1 \
  --seed-start 30001 \
  --seed-count 500 \
  --jobs 12 \
  --out data/teacher_t1_half_500
```

Full T1：

```bash
python search/run_teacher.py \
  --scenario real \
  --budget t1 \
  --seed-start 31001 \
  --seed-count 500 \
  --jobs 12 \
  --out data/teacher_t1_full_500
```

Curriculum data：

```bash
python search/run_teacher.py \
  --scenario curriculum_065 \
  --budget t1 \
  --seed-start 44001 \
  --seed-count 200 \
  --jobs 12 \
  --out data/teacher_curriculum_065_200

python search/run_teacher.py \
  --scenario curriculum_080 \
  --budget t1 \
  --seed-start 45001 \
  --seed-count 200 \
  --jobs 12 \
  --out data/teacher_curriculum_080_200
```

### 25.4 BC

Half：

```bash
python training/train_bc.py \
  --data data/teacher_t1_half_500 \
  --out models/bc_hp_half_t1_500.npz \
  --epochs 20 \
  --batch-decisions 32
```

Full：

```bash
python training/train_bc.py \
  --data data/teacher_t1_full_500 \
  --out models/bc_hp_full_t1_500.npz \
  --epochs 20 \
  --batch-decisions 32
```

### 25.5 Action agreement

```bash
python tools/report_action_agreement.py \
  --data data/teacher_t1_half_500 \
  --checkpoint models/bc_hp_half_t1_500.npz \
  --out reports/agreement_hp_half_t1_500.json \
  --split-seed 20261101
```

Full 场景使用相同命令，将 data、checkpoint 和 out 换为 full 路径。

### 25.6 Half PPO plain

```bash
python training/train_ppo.py \
  --checkpoint models/bc_hp_half_t1_500.npz \
  --out models/ppo_hp_half_t1_plain_1000.npz \
  --scenario half \
  --seed-start 35001 \
  --seed-count 1000 \
  --iterations 10 \
  --episodes-per-iteration 100 \
  --epochs-per-iteration 3 \
  --learning-rate 0.001 \
  --seed 20261203 \
  --val-seed-start 38201 \
  --val-seed-count 100
```

### 25.7 Half PPO + Teacher demo

```bash
python training/train_ppo.py \
  --checkpoint models/bc_hp_half_t1_500.npz \
  --out models/ppo_hp_half_t1_demo_3000.npz \
  --scenario half \
  --seed-start 35001 \
  --seed-count 3000 \
  --iterations 30 \
  --episodes-per-iteration 100 \
  --epochs-per-iteration 3 \
  --learning-rate 0.001 \
  --seed 20261201 \
  --val-seed-start 38001 \
  --val-seed-count 100 \
  --demo-data data/teacher_t1_half_500 \
  --demo-updates-per-iteration 1 \
  --demo-batch-decisions 32
```

### 25.8 Full direct PPO

第一次 full run 使用了有 seed overlap 的配置，已经排除。使用修正后的 v2：

```bash
python training/train_ppo.py \
  --checkpoint models/bc_hp_full_t1_500.npz \
  --out models/ppo_hp_full_t1_demo_3000_v2.npz \
  --scenario real \
  --seed-start 37001 \
  --seed-count 3000 \
  --iterations 30 \
  --episodes-per-iteration 100 \
  --epochs-per-iteration 3 \
  --learning-rate 0.001 \
  --seed 20261202 \
  --val-seed-start 40001 \
  --val-seed-count 100 \
  --demo-data data/teacher_t1_full_500 \
  --demo-updates-per-iteration 1 \
  --demo-batch-decisions 32
```

### 25.9 Full HP curriculum

```bash
python training/train_ppo.py \
  --checkpoint models/ppo_hp_half_t1_demo_3000.npz \
  --out models/ppo_curriculum_065_1000.npz \
  --scenario curriculum_065 \
  --seed-start 46001 \
  --seed-count 1000 \
  --iterations 10 \
  --episodes-per-iteration 100 \
  --epochs-per-iteration 3 \
  --learning-rate 0.001 \
  --seed 20261311 \
  --val-seed-start 48001 \
  --val-seed-count 100 \
  --allow-non-bc-checkpoint \
  --demo-data data/teacher_curriculum_065_200 \
  --demo-updates-per-iteration 1 \
  --demo-batch-decisions 32
```

随后将 checkpoint 改为 `ppo_curriculum_065_1000.npz`、scenario 改为 `curriculum_080`，最后将 checkpoint 改为 `ppo_curriculum_080_1000.npz`、scenario 改为 `real`。

### 25.10 最终评测

Half demo PPO：

```bash
python eval/run_eval.py \
  --scenario half \
  --seed-start 42001 \
  --seed-count 500 \
  --policies PPO \
  --checkpoint models/ppo_hp_half_t1_demo_3000.npz \
  --greedy-cap 6 \
  --out reports/ppo_hp_half_t1_demo_3000_eval \
  --replays replays/ppo_hp_half_t1_demo_3000_eval
```

Full curriculum PPO：

```bash
python eval/run_eval.py \
  --scenario real \
  --seed-start 43001 \
  --seed-count 500 \
  --policies PPO \
  --checkpoint models/ppo_curriculum_1000.npz \
  --greedy-cap 6 \
  --out reports/ppo_hp_full_curriculum_eval \
  --replays replays/ppo_hp_full_curriculum_eval
```

### 25.11 测试

```bash
cargo test --manifest-path sim/Cargo.toml
python python/tests/test_env.py
python python/tests/test_training.py
python -m py_compile python/lcb/*.py training/*.py search/*.py eval/*.py tools/*.py
```

---

## 26. 文件和产物索引

### 26.1 规则和数据

| 路径 | 用途 |
|---|---|
| `docs/MECHANICS.md` | 规则、来源、验证状态、UNKNOWN 和 NOT_IMPLEMENTED |
| `docs/COVERAGE.md` | 生成的技能、状态、被动覆盖表 |
| `docs/DATA.md` | 数据来源和 pipeline |
| `docs/FIXED_CONTENT_SCOPE.md` | 固定内容、高保真 gate 和简化机制 |
| `docs/STATUS.md` | 当前实现阶段、测试和验证缺口 |
| `data/mechanics/effects.json` | 结构化技能效果 |
| `data/mechanics/status_effects.json` | 状态自己的 phase 和 continuous effects |
| `data/mechanics/enemy_scripts.json` | Pupa、Imago 和影蝶脚本 |
| `data/sources/` | provenance 和源版本 |

### 26.2 训练代码

| 路径 | 用途 |
|---|---|
| `python/lcb/env.py` | Python environment 和完整回合接口 |
| `python/lcb/features.py` | 状态和动作编码 |
| `python/lcb/plans.py` | PlanGenerator、actor order、mask |
| `python/lcb/teacher.py` | Beam Teacher、value、axis detection |
| `python/lcb/bc.py` | BC 训练 |
| `python/lcb/ppo.py` | PPO rollout 和 update |
| `python/lcb/dagger.py` | DAgger 辅助逻辑 |
| `training/run_dagger.py` | DAgger CLI |
| `training/train_bc.py` | BC CLI |
| `training/train_ppo.py` | PPO CLI |
| `search/run_teacher.py` | Teacher CLI |
| `search/run_oracle.py` | 固定 Sinking setup 前向模型 oracle |
| `eval/run_eval.py` | 统一评测 CLI |
| `tools/report_action_agreement.py` | Teacher label agreement |

### 26.3 当前主要数据和报告

```text
data/teacher_t1_half_500/
data/teacher_t1_full_500/
data/teacher_curriculum_065_200/
data/teacher_curriculum_080_200/
data/dagger_t1_half_r1_200/
data/dagger_t1_half_argmax_r1_200/
data/dagger_t1_half_argmax_r2_200/
data/dagger_t1_full_r1_200/

models/*.json                 训练配置和指标，可提交
models/*.npz                  二进制 checkpoint，按规则忽略
reports/ppo_hp_research_t1_curriculum.json
reports/agreement_hp_half_t1_500.json
reports/agreement_hp_full_t1_500.json
reports/hp_half_t1_bc_500/
reports/hp_full_t1_bc_500/
reports/ppo_hp_half_t1_plain_1000_eval/
reports/ppo_hp_half_t1_demo_3000_eval/
reports/ppo_hp_full_t1_demo_3000_v2_eval/
reports/ppo_hp_full_curriculum_eval/
reports/ppo_hp_sinking_reward_ablation.json
reports/ppo_hp_half_t1_demo_3000_nosinking_reward_eval/
reports/ppo_hp_full_t1_demo_3000_nosinking_reward_eval/
replays/
```

本次 reward ablation 的 Teacher/BC/PPO metadata 使用带
`nosinking_reward` 后缀的路径；对应的 `.npz` 仍被 `.gitignore` 忽略，JSON
index、训练报告、holdout JSON 和选定 replay 可以提交。

每个 `.jsonl` 是逐 episode 的机器可读结果。每个 replay JSON 记录最短胜局或选定样例，`replays/index.json` 汇总文件名、seed、kill turn 和 transition hashes。

---

## 27. 测试和验证记录

### 27.1 Rust 测试数量的阶段变化

测试数量随修复增加：

| 阶段 | 记录 |
|---|---|
| 第一次专项审阅前 | 90 项 Rust 测试 |
| 第二轮复查后 | 99 项 |
| 第三轮复查后 | 106 项 |
| post-audit 修复完成记录 | 118 项 |
| 本次修订后 workspace 测试记录 | 147 项（14 unit + 13 engineering + 120 mechanics） |

数量变化不是同一份固定测试报告重复运行，而是每轮新增机制和回归测试后的阶段记录。

### 27.2 重要测试类别

#### 工程测试

- clone 是深复制；
- state hash 稳定；
- serialization；
- RNG deterministic；
- replay state transition；
- 原子完整 Plan 非法时回滚；
- 增量提交与完整 Plan 结果一致；
- PyO3 与 stdio parity。

#### 机制测试

- Clash power 汇总所有 Coin；
- Clash tie 不摧毁 Coin；
- loser 每轮摧毁第一枚剩余 Coin；
- Unbreakable/Cracked Coin；
- damage formula；
- Stagger threshold；
- Burn、Bleed、Sinking、Poise、Rupture；
- Protection、Fragile、Power status；
- Tremor Burst、Shield、Bind；
- Dashboard rotation、Discard、Defense rotation；
- Sin Resonance 和 Absolute Resonance；
- Imago rotation、time state、stack bonus；
- Pupa Shield、HP floor 和 Encounter End；
- Illusory Butterfly segmentation；
- Rime Shank exact P/C 和 AW；
- Bygone individual gain events；
- Echoes per-event deterministic rolls；
- infinite resource affordability 和 SP 保留；
- main enemy victory condition；
- Sinking Potency/Count 的 99 上限、JSON 载入归一化和 cap 后事件 delta。

#### Python training tests

- Plan 和 mask 与 simulator 一致；
- Teacher plan 可 replay；
- Teacher sample label 在 candidate 中；
- seed split 不泄漏 episode；
- reward summary；
- `sinking_damage` 不再影响 `compute_reward()` 的回归测试；
- best-of-N；
- axis detection 读取真实状态；
- PPO finite-difference direction；
- PPO rollout 和 validation 都传播 `infinite_ego_resources`。

### 27.3 尚未完成的 golden test

当前没有游戏录像或截图驱动的 `video_verified` golden test。已有：

- wiki.gg；
- 日文战斗说明；
- 游戏本地化文本；
- 固定 Coin worked examples；
- deterministic replay。

但下列数值仍应与实际视频或截图核对：

- Imago 真实 max HP 及缩放后的绝对 HP；
- Pupa Shield；
- speed roll；
- 关键伤害数值；
- 客户端 RNG 序列。

---

## 28. 已知协议问题和文档注意事项

这一节必须保留，不能为了让文档看起来更整齐而删除。

### 28.1 seed 没有做到全局完全不重叠

项目的目标是训练、验证、DAgger 和最终 holdout 分离。最终 holdout `42001-42500` 和 `43001-43500` 没有与记录的训练和验证段重叠，因此最终单次测试没有直接使用训练 seed。

但全流程并没有做到“所有阶段、所有场景之间的 seed 数字全局不重复”：

- Half PPO demo 训练使用 `35001-38000`；
- Full direct PPO 修正版训练使用 `37001-40000`；
- 两者在 `37001-38000` 有数字重叠，但场景不同；
- DAgger half 和 full 的若干实际 band 也有数字重叠，但场景不同；
- Curriculum 0.80 的 validation `48101-48200` 不在 0.80 自己的训练段 `47001-48000` 中，但 Curriculum 1.00 的训练段 `48001-49000` 包含了它；
- Curriculum 1.00 的 validation `48201-48300` 也落在它自己的训练段 `48001-49000` 内。

因此正确表述是：

> 最终 holdout 与训练段分离，direct full run 的 validation 与其训练段分离。Curriculum 内部的 validation seed 没有做到全局完全隔离，存在用于 checkpoint 选择的协议泄漏风险。Curriculum 的 81.2% 是未见最终 holdout 的结果，但不是一份完全无重叠的全局训练/验证设计证明。

后续若要做正式统计，应为每个阶段预留完全不相交的训练、validation、DAgger 和 test 区间，并重新训练 curriculum。

### 28.2 报告 provenance 的 dirty 状态

Teacher 索引和部分评测报告是在最后代码提交前生成的，报告中可能记录：

```json
"git": {
  "commit": "41b804b...",
  "dirty": true
}
```

这代表报告生成时工作树包含后来提交的代码或生成文件，不代表 replay 本身不可用。最终代码和报告已由 `5cadfbf` 提交，但历史报告中的 provenance 不会自动重写。

### 28.3 旧文档中的中间阶段表述

`docs/TRAINING_RESULTS.md` 保留了 legacy、post-audit 和当前 T1/curriculum 多个阶段。`docs/STATUS.md` 也保留了部分中间阶段的“partial”叙述。

阅读优先级：

1. 当前生成报告和 JSON 的 scenario config；
2. `docs/MECHANICS.md` 的逐条规则；
3. `docs/COVERAGE.md` 的生成 coverage；
4. `docs/TRAINING_RESULTS.md` 中明确标记 current 的部分；
5. 旧的 legacy 章节只用于历史记录，不用于当前结论。

### 28.4 infinite E.G.O 不是游戏结论

最终 99.0% 和 81.2% 都是在 `infinite_ego_resources=true` 的显式场景条件下取得。该条件只让实验集中研究行动、SP、Overclock 和技能效果，不能写成“实际游戏资源无限时也一样”，更不能写成普通游戏胜率。

---

## 29. 仍然没有完成的工作

### 29.1 机制范围

- 全局 unknown-rule inventory；
- 所有 E.G.O passive、gift 和 Observation Level；
- 完整 Focused Encounter Parts；
- Section 5 选择事件 UI；
- 支持被动的队伍槽位选择；
- 某些 HP Healing Down、Wrath/Gloom Fragility 等次级状态效果；
- 某些 E.G.O Corrosion target selection 的完整一般规则；
- 客户端 RNG 算法；
- 所有技能和状态的 video-verified golden tests。

### 29.2 后端和回放

- 更广泛的 clone/load-state parity；
- error-path parity；
- 多敌人相同 slot 的边界行为；
- replay 自动重新执行并逐回合验证 Transition Hash 的独立工具。

### 29.3 训练和实验

- 重新规划全局不重叠 seed 后重跑 curriculum；
- 增大 Teacher T2 预算并与 T1 做完全独立比较；
- Teacher top-k 或软标签；
- 更多 DAgger 轮次和更大数据；
- 更大 PPO rollout 和更有统计功效的评测；
- paired bootstrap 或更完整的 paired confidence interval；
- 按角色、回合、E.G.O/普通技能/防守/目标选择继续拆 action agreement；
- 记录完整 HP trajectory、死亡回合分布和状态切换分布；
- 重新检查最终策略是否在有限 E.G.O 资源条件下仍然有效。

### 29.4 结果表达

当前结果适合表达为：

```text
在当前模拟器、固定阵容、strict=true、30 回合、无限 E.G.O 资源和明确 HP 场景条件下，
Teacher-demo PPO 与 HP curriculum PPO 在独立最终 holdout 上取得了所报告的胜率。
```

不适合表达为：

```text
AI 已经完全学会真实游戏中的罗生蝶最优打法。
```

---

## 30. 最终总结

整个项目经历了以下顺序：

```text
资料和边界定义
  -> 生成式数据 pipeline
  -> Rust BattleState、RNG、Clone、Hash、Replay
  -> 技能、牌面、拼点、伤害、状态、防御、E.G.O
  -> Pupa、Imago、时间状态、影蝶和 campaign
  -> 多轮完整战斗复查
  -> 修正数据解析和公共结算顺序
  -> PyO3/stdio parity 和严格 preflight
  -> 完整回合 step_turn
  -> Random、FirstLegal、Greedy
  -> 回合级 Beam Teacher
  -> 场景专属 BC
  -> PPO
  -> Teacher demonstration replay
  -> clone-state DAgger
  -> action agreement 和 Wilson CI
  -> HP curriculum
  -> 500-seed 最终 holdout
```

最重要的工程结论不是某一个胜率数字，而是：

1. 规则来源和策略实验分开；
2. 未知规则不被猜成已实现；
3. 完整回合 Plan 由模拟器原子校验；
4. 搜索、训练和评测共用真实合法动作 mask；
5. 奖励来自实际 Boss HP 和实际 TurnStats；
6. Teacher、BC、PPO、DAgger 和 curriculum 都保存了机器可读 provenance；
7. 失败实验和错误 run 被保留并标记为 superseded 或 negative ablation；
8. 99.0% half 和 81.2% full 是当前明确实验条件下的工程 benchmark，不是对完整游戏的无条件声明。

当前仓库的可复核入口是：

```text
规则：docs/MECHANICS.md
data：docs/DATA.md
范围：docs/FIXED_CONTENT_SCOPE.md
状态：docs/STATUS.md
训练接口：docs/TRAINING.md
训练计划：docs/TRAINING_PLAN.md
训练结果：docs/TRAINING_RESULTS.md
当前汇总：reports/ppo_hp_research_t1_curriculum.json
最终代码：commit 5cadfbf
```
