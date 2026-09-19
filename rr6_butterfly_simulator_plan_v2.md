# RR6 罗生蝶模拟器制作计划

## 项目原则

从零实现一个可靠的小型《Limbus Company》战斗模拟器。

核心要求：

-   优先保证机制正确性；
-   不实现未经确认的规则；
-   未知机制标记为 UNKNOWN 或 NOT_IMPLEMENTED；
-   不使用"合理推测"替代资料；
-   模拟器规则与实验逻辑分离。

不要为了 benchmark、测试结果或策略表现修改游戏规则。

## 资料来源

灰机 Wiki 可能存在反爬或访问限制，因此不要依赖单一中文来源。

优先级：

1.  游戏内文本、技能说明、Boss 描述、状态 tooltip。
2.  英文 wiki.gg： https://limbuscompany.wiki.gg/
3.  日文 WikiWiki： https://wikiwiki.jp/lcbwiki/
4.  韩文游戏文本、官方资料、可靠社区资料。
5.  中文资料作为辅助。

每条机制记录：

-   来源；
-   版本；
-   语言；
-   描述；
-   验证状态。

验证状态：

-   official
-   multi_source_verified
-   single_source_verified
-   video_verified
-   unknown
-   synthetic

REAL 模式禁止加载 unknown 和 synthetic。

## 技术架构

Rust：

负责核心模拟：

-   BattleState
-   Unit
-   Skill
-   Coin
-   Clash
-   Damage
-   Status
-   E.G.O
-   Boss
-   RNG
-   Replay
-   Hash

Python：

负责：

-   环境封装；
-   搜索；
-   训练；
-   数据分析。

使用 PyO3 连接 Rust 和 Python。

## 固定内容

人格：

-   李箱脑叶公司E.G.O::庄严哀悼
-   格里高尔脑叶公司E.G.O:目灯
-   良秀脑叶公司E.G.O::余香·孤独
-   以实玛利定事务所代表
-   辛克莱流浪乐队老大
-   奥提斯LCA瓦吉特先锋三队队长
-   罗佳脑叶公司E.G.O::泪锋之剑

E.G.O：

-   李箱庄严哀悼 E.G.O
-   李箱往昔
-   格里高尔庄严哀悼 E.G.O
-   以实玛利往昔
-   以实玛利涌潮悲歌
-   辛克莱和声
-   罗佳冰爪

所有数据使用正式 ID，不使用玩家简称作为唯一标识。

## 类型设计

避免使用大量 bool 表示状态。

例如：

不要：

    has_sp = false

建议：

    enum SanityState {
        None,
        HasSanity(sp)
    }

确保异想体不会错误拥有 SP，也不会进入不适用的状态。

## 实现顺序

### Phase 1

基础状态：

-   HP
-   SP
-   Speed
-   RNG
-   Status
-   Clone
-   Hash
-   Serialization

### Phase 2

技能：

-   Skill
-   Coin
-   Skill Deck
-   Dashboard
-   Action Slot

不要简化成每回合重新抽技能。

### Phase 3

战斗：

-   Coin Roll
-   Skill Power
-   Clash
-   Damage
-   Resistance
-   Stagger

Clash 必须按照真实规则建模。

### Phase 4

状态：

实现当前需要的：

-   Sinking
-   Butterfly
-   Fragile
-   其他必要状态

### Phase 5

E.G.O：

实现：

-   资源消耗
-   SP 消耗
-   Awakening
-   Corrosion
-   Overclock
-   Coin Reuse

### Phase 6

Boss：

实现：

-   RR6 Section 5 罗生蝶
-   Imago
-   幻影蝶
-   相关 Boss 机制

不要复用其他 Section 的数据。

## Replay 与 Hash

必须支持：

-   固定 seed
-   完整 replay
-   状态恢复

transition hash 必须包含：

-   RNG
-   Deck
-   Dashboard
-   所有影响未来状态的信息

Clone 必须深复制。

## 测试

工程测试：

-   clone isolation
-   hash
-   serialization
-   RNG
-   replay

机制测试：

每个机制测试必须对应资料来源。

Golden Test：

使用：

-   游戏录像；
-   截图；
-   手动记录。

验证模拟结果。

## 禁止事项

禁止：

-   为了让某策略更强调整规则；
-   为了测试通过修改机制；
-   根据技能名字猜效果；
-   用类似技能推断未知技能；
-   把 synthetic 数据混入真实模式。

资料不足：

    UNKNOWN

无法实现：

    NOT_IMPLEMENTED

不要编造。

## 搜索阶段

模拟器完成后：

Random ↓

Greedy ↓

Beam Search ↓

MCTS ↓

Learning

Greedy 必须通过真实模拟比较动作，不要手写假的伤害估计。

## Python 接口

需要提供：

``` python
reset(seed)

legal_actions()

step(action)

clone_state()

transition_hash()
```

动作空间采用：

选择角色 → 选择技能 → 选择目标 → 提交行动

## 第一阶段目标

完成：

-   资料库；
-   Rust 核心；
-   基础战斗；
-   固定人格/E.G.O；
-   罗生蝶；
-   replay；
-   测试。

完成前不要开始大规模搜索。

原则：

> 不知道就标记未知。
>
> 规则必须来自资料。
>
> 模拟器只负责模拟。
