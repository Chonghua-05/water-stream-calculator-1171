# Minecraft 1.17.1 掉落物水道源码建模审阅与补完

本文基于 `src/main/java` 中的 Minecraft 1.17.1 源码，目标是给出一份可直接回到源码逐条复核的 `ItemEntity` 水道运动模型。本文只讨论“掉落物单体”的服务端 tick 逻辑；多掉落物互推、合并、客户端插值不作为主模型。

标记约定：

- `[源码已证实]`：可直接由当前仓库源码确认。
- `[仍需实测]`：源码给出机制，但具体工程表现依赖布局、相位、碰撞时机，不能只靠静态读码定量。

---

## 1. 本次审阅后的核心结论

1. `[源码已证实]` `ItemEntity` 在水道里通常每 tick 会吃 **两次** 水流推进，而不是一次。
   - 第一次来自 `Entity.baseTick()` 内的 `updateInWaterStateAndDoFluidPushing()`。
   - 第二次来自 `ItemEntity.tick()` 末尾再次调用的 `updateInWaterStateAndDoFluidPushing()`。

2. `[源码已证实]` 掉落物“是否走水中漂浮分支”不是看 `isInWater()` 一项，而是还要求：
   - `getFluidHeight(WATER) > getEyeHeight() - 0.11111111F`
   - 对 `ItemEntity`，这条阈值数值是 `0.2125 - 0.11111111 = 0.10138889`

3. `[源码已证实]` 冰、浮冰、霜冰、蓝冰不会改变 `FlowingFluid` 的水流方向或水推常数；它们只会在 `ItemEntity` 本 tick 落地时，通过 `blockBelow.getFriction() * 0.98F` 影响本 tick 的水平阻尼。

4. `[源码已证实]` `FlowingFluid.getFlow()` 的结果在返回前会 `normalize()`。
   - 因此，对非玩家实体、标准直线单向水流而言，只要局部流向非零，单次水推的水平量级通常就是 `0.014`，而不是随 `7/9 ... 1/9` 线性缩小。

5. `[源码已证实]` `PistonMovingBlockEntity` 里，移动中的粘液块撞到非玩家实体时，会把对应轴的速度分量直接改写成 `+1` 或 `-1`，不是“叠加一点冲量”。

6. `[仍需实测]` 某个具体水道布局里，掉落物每 tick 是否落地、在哪些 tick 触发 `stepOn()`、浅水末端何时转入重力分支，仍然取决于几何、相位和碰撞顺序，不能只靠常数直接下最终工程结论。

---

## 2. 直接相关源码入口

- `src/main/java/net/minecraft/world/entity/item/ItemEntity.java`
- `src/main/java/net/minecraft/world/entity/Entity.java`
- `src/main/java/net/minecraft/world/level/material/FlowingFluid.java`
- `src/main/java/net/minecraft/world/level/material/WaterFluid.java`
- `src/main/java/net/minecraft/world/level/block/Blocks.java`
- `src/main/java/net/minecraft/world/level/block/Block.java`
- `src/main/java/net/minecraft/world/level/block/state/BlockBehaviour.java`
- `src/main/java/net/minecraft/world/level/block/LiquidBlock.java`
- `src/main/java/net/minecraft/world/phys/shapes/EntityCollisionContext.java`
- `src/main/java/net/minecraft/world/level/block/piston/PistonMovingBlockEntity.java`
- `src/main/java/net/minecraft/server/level/ServerLevel.java`

建议优先阅读顺序：

1. `ItemEntity.tick()`
2. `Entity.baseTick()`
3. `Entity.updateFluidHeightAndDoFluidPushing()`
4. `FlowingFluid.getFlow()`
5. `FlowingFluid.getHeight()` / `getOwnHeight()`
6. `Blocks` 中相关 `friction(...)`
7. `PistonMovingBlockEntity.moveCollidedEntities()`

---

## 3. 固定常数与几何参数

### 3.1 掉落物尺寸与眼高

`[源码已证实]`

- `EntityType.ITEM` 尺寸：`0.25 x 0.25`
  - `EntityType.java`：`EntityType.ITEM ... sized(0.25F, 0.25F)`
- 默认眼高公式：`height * 0.85`
  - `Entity.java`：`getEyeHeight(Pose pose, EntityDimensions size)`
- 因此掉落物眼高：
  - `0.25 * 0.85 = 0.2125`

### 3.2 “足够浸没”阈值

`[源码已证实]`

`ItemEntity.tick()` 用：

```text
f = getEyeHeight() - 0.11111111F
```

代入掉落物：

```text
f = 0.2125 - 0.11111111 = 0.10138889
```

因此只有当：

```text
getFluidHeight(WATER) > 0.10138889
```

时，掉落物才进入水中漂浮分支。

### 3.3 水推常数

`[源码已证实]`

- 水：`0.014`
- 熔岩（超热维度）：`0.007`
- 熔岩（普通维度）：`0.0023333333333333335`

定义位置：`Entity.java`

### 3.4 通用阻尼与重力常数

`[源码已证实]`

- 水中水平缩放：`0.99`
- 岩浆中水平缩放：`0.95`
- 通用空气/落地乘子：`0.98`
- 重力：`vy -= 0.04`
- 水/岩浆微上浮条件：`vy < 0.06` 时，`vy += 0.0005`
- 落地反弹：若 `onGround && vy < 0`，则 `vy *= -0.5`

### 3.5 流体接触箱与小量阈值

`[源码已证实]`

`Entity.updateFluidHeightAndDoFluidPushing()` 中：

- 接触用 AABB：`getBoundingBox().deflate(0.001)`
- 若某接触流体格的浸没深度 `d < 0.4`，先把该格 `flow` 缩放成 `flow * d`
- 速度托底判定：
  - 若 `abs(vx) < 0.003`
  - 且 `abs(vz) < 0.003`
  - 且本次待加流体向量长度 `< 0.0045`
  - 则把本次待加向量归一化到 `0.0045`

说明：

- `[源码已证实]` 对 `ItemEntity` 这种非玩家实体，只要合成流向非零，后续通常还会先 `normalize()` 再乘 `0.014`。
- `[源码已证实]` 所以在“标准直线、单一主流向”的水道里，`d < 0.4` 这一步往往不会改变最终单次水平水推的 `0.014` 量级。
- `[仍需实测]` 在多格同时接触、转角、分叉、落水边缘等复杂局部里，`d < 0.4` 与平均化仍可能改变方向和最终结果。

---

## 4. 水面高度与 `fluidHeight`

### 4.1 单格水高

`[源码已证实]`

`FlowingFluid.getOwnHeight()`：

```text
ownHeight = amount / 9.0F
```

而 `WaterFluid` 中：

- 源水 `getAmount() = 8`
- 流水 `getAmount() = LEVEL`，范围 `7..1`

所以在“上方不是同类水”的普通情形下：

- 源水：`8/9 = 0.88888889`
- 流水：`7/9, 6/9, ..., 1/9`

### 4.2 上方同类水时的高度

`[源码已证实]`

`FlowingFluid.getHeight()`：

- 若上方还是同类流体，则本格高度按 `1.0F` 算
- 否则按 `getOwnHeight()` 算

### 4.3 `getFluidHeight(WATER)` 真正存的是什么

`[源码已证实]`

`Entity.updateFluidHeightAndDoFluidPushing()` 里写入的 `fluidHeight` 是：

```text
本 tick 接触到的所有同标签流体格中，
max(流体表面高度 - AABB.minY)
```

它是“最大浸没深度”，不是体积占比，也不是平均深度。

---

## 5. `ItemEntity` 单 tick 精确顺序

### 5.1 服务器端总顺序

`[源码已证实]`

`ServerLevel.tick()` 中：

1. 先 tick 实体
2. 后 tick 方块实体

并且 `tickNonPassenger()` 里先做：

```text
entity.tickCount++
entity.tick()
```

这意味着 `ItemEntity.tick()` 中的 `(tickCount + id) % 4` 使用的是“已经自增后的 tickCount”。

### 5.2 `ItemEntity.tick()` 的源码顺序

`[源码已证实]`

每个有效掉落物 tick 的主流程可以写成：

1. `super.tick()`
   - 即 `Entity.tick() -> baseTick()`
2. `pickupDelay` 递减
3. 记录旧坐标 `xo/yo/zo`
4. 读取当前 `deltaMovement`
5. 判断走：
   - 水中漂浮分支
   - 岩浆漂浮分支
   - 重力分支
6. 处理 `noPhysics` / 挤出最近空位
7. 满足条件时调用 `move(MoverType.SELF, deltaMovement)`
8. 移动后乘阻尼：
   - 空中：`g = 0.98`
   - 落地：`g = blockBelow.getFriction() * 0.98`
9. 若 `onGround && vy < 0`，再做 `vy *= -0.5`
10. 合并邻近掉落物
11. `age++`
12. tick 末再次 `updateInWaterStateAndDoFluidPushing()`

### 5.3 第一次水推发生在何处

`[源码已证实]`

第一次水推来自 `Entity.baseTick()`：

```text
baseTick()
  -> updateInWaterStateAndDoFluidPushing()
     -> updateInWaterStateAndDoWaterCurrentPushing()
        -> updateFluidHeightAndDoFluidPushing(WATER, 0.014)
```

也就是说，在 `ItemEntity.tick()` 进入“水中漂浮 or 重力”判断之前，`fluidHeight(WATER)` 和第一次水流推进都已经算过了。

### 5.4 漂浮分支与重力分支

`[源码已证实]`

若：

```text
isInWater() && getFluidHeight(WATER) > 0.10138889
```

则执行：

```text
vx *= 0.99
vz *= 0.99
if (vy < 0.06) vy += 0.0005
```

若在岩浆里且满足同样阈值逻辑，则：

```text
vx *= 0.95
vz *= 0.95
if (vy < 0.06) vy += 0.0005
```

否则若无失重：

```text
vy -= 0.04
```

关键点：

- `[源码已证实]` “碰到水”不等于“一定走水中漂浮分支”。
- `[源码已证实]` 只要 `fluidHeight(WATER) <= 0.10138889`，本 tick 仍然可能走重力分支。

### 5.5 何时会跳过 `move(...)`

`[源码已证实]`

只有在下列条件全部不满足时，掉落物本 tick 才会跳过 `move(...)`：

```text
!onGround
horizontalDistanceSqr() > 1.0E-5
(tickCount + id) % 4 == 0
```

等价地说，若：

- 当前在地上
- 且水平速度平方 `<= 1.0E-5`
- 且当前不落在那 1/4 的采样 tick 上

则本 tick 不执行 `move(...)` 与随后的那组摩擦/反弹逻辑。

### 5.6 移动后阻尼与反弹

`[源码已证实]`

`ItemEntity.tick()` 调用 `move(MoverType.SELF, deltaMovement)` 时，`Entity.move()` 会先做碰撞修正。若请求位移 `pos.y` 与实际位移 `vec3.y` 不同，会调用落脚方块：

```text
block.updateEntityAfterFallOn(level, entity)
```

普通 `Block.updateEntityAfterFallOn()` 会执行：

```text
deltaMovement *= (1.0, 0.0, 1.0)
```

因此，对普通方块、冰、浮冰、蓝冰等未覆写该方法的方块，落地碰撞后垂直速度会先被清零。粘液块例外：`SlimeBlock.updateEntityAfterFallOn()` 会在未按潜行逻辑时把负 `vy` 反弹成 `-vy * 0.8`。

随后才回到 `ItemEntity.tick()` 的移动后阻尼段。若本 tick 执行了 `move(...)`，之后：

```text
g = 0.98
if (onGround) g = blockBelow.getFriction() * 0.98

vx *= g
vy *= 0.98
vz *= g
```

然后若：

```text
onGround && vy < 0
```

再做：

```text
vy *= -0.5
```

注意：

- `[源码已证实]` 普通方块/冰/浮冰/蓝冰落地时，`vy` 在 `Entity.move()` 内已经由 `Block.updateEntityAfterFallOn()` 清零，所以通常不会走到这里的 `vy *= -0.5` 反弹。
- `[源码已证实]` 这里取摩擦的地板坐标是：
  - `new BlockPos(getX(), getY() - 1.0, getZ())`
- `[源码已证实]` 这不是 `Entity.move()` 中的 `getOnPos()` 逻辑。
- `[仍需实测]` 因此在边缘、台阶、非整块碰撞面附近，`stepOn()` 触发方块和 `ItemEntity` 自己取摩擦的方块，可能不是同一个。

旧版简化模型若把所有落地都按 `vy *= -0.5` 处理，会让掉落物在干段末端和下一段水中飞得过高，导致水接触相位漂移；这是 `W3-I_D3-B` 早期模型比游戏实测快约 `0.008~0.010 m/gt` 的主要原因。

历史简化写法如下，只能在没有发生普通方块落地碰撞清零时使用：

```text
g = 0.98
if (onGround) g = blockBelow.getFriction() * 0.98

vx *= g
vy *= 0.98
vz *= g

if (onGround && vy < 0) vy *= -0.5
```

---

### 5.7 第二次水推

`[源码已证实]`

`ItemEntity.tick()` 末尾再次执行：

```text
this.updateInWaterStateAndDoFluidPushing()
```

因此 tick 末还会再来一轮：

- 更新 `fluidHeight`
- 更新 `wasTouchingWater`
- 若局部流向非零，再加一次 `0.014 * flowDirection`

---

## 6. `updateFluidHeightAndDoFluidPushing()` 的精确机制

`[源码已证实]`

对给定流体标签，算法可以概括为：

1. 取 `AABB.deflate(0.001)`
2. 枚举 AABB 覆盖到的所有整格
3. 对每个匹配流体格，算：
   - `surface = q + fluidState.getHeight(level, pos)`
4. 若 `surface >= aabb.minY`：
   - 认为本格参与接触
   - 用 `surface - aabb.minY` 更新最大浸没深度 `d`
   - 若 `isPushedByFluid() == true`，再把 `fluidState.getFlow(...)` 加入合成向量
5. 所有参与格处理完后：
   - 先除以参与推流计数 `o`
   - 非玩家实体再 `normalize()`
   - 再乘 `motionScale`
   - 满足极小速度条件时，托底到 `0.0045`
   - 最后把该向量加到 `deltaMovement`

补充：

- `[源码已证实]` `Entity.isPushedByFluid()` 默认返回 `true`，掉落物没有覆盖它。
- `[源码已证实]` 所以 `ItemEntity` 默认会受流体推进。

---

## 7. `FlowingFluid.getFlow()` 的精确分支

### 7.1 水平流向主公式

`[源码已证实]`

对四个水平相邻方向逐一检查。

若邻格流体“可参与流动计算”：

- 邻格为空或同类流体都算可参与

再按以下规则求该方向贡献：

1. 取邻格 `f = fluidState2.getOwnHeight()`
2. 若 `f == 0`：
   - 且邻格方块 `!blocksMotion()`
   - 再看邻格下方同类流体 `fluidState3`
   - 若其 `f > 0`，则
     - `g = selfOwnHeight - (f - 0.8888889F)`
3. 若 `f > 0`：
   - `g = selfOwnHeight - f`
4. 若 `g != 0`：
   - `d += direction.stepX * g`
   - `e += direction.stepZ * g`

最后得到水平向量：

```text
vec = (d, 0, e)
```

### 7.2 落水 `FALLING == true` 分支

`[源码已证实]`

若当前流体状态带 `FALLING == true`，再检查四个水平侧面：

- 只要本格侧面或上方对应侧面存在“实面”
- 则：

```text
vec = normalize(vec) + (0, -6, 0)
```

最后整个向量再统一 `normalize()` 返回。

结论：

- `[源码已证实]` 落水态会显著增强向下分量。
- `[源码已证实]` 这会削弱最终水平投影。

### 7.3 为什么直线顺流水段常常是固定 `0.014`

`[源码已证实]`

因为：

1. `FlowingFluid.getFlow()` 返回前会 `normalize()`
2. `ItemEntity` 是非玩家实体
3. `Entity.updateFluidHeightAndDoFluidPushing()` 对非玩家还会再次 `normalize()`
4. 然后才乘 `motionScale = 0.014`

所以在“平地、直线、单向、非零流向”的标准水段里，单次水平水推通常就是：

```text
+0.014 或 -0.014
```

而不是按 `7/9 ... 1/9` 直接缩放。

### 7.4 何时会得到 `0` 推流

`[源码已证实]`

若局部 `getFlow()` 算出来就是零向量，则该次不会提供水平推力。

这就是静水刹车段、源水阻尼段的源码基础。

---

## 8. 地板摩擦、`speedFactor` 与特殊方块

### 8.1 默认摩擦

`[源码已证实]`

`BlockBehaviour.Properties` 默认：

- `friction = 0.6F`
- `speedFactor = 1.0F`

### 8.2 相关方块摩擦

`[源码已证实]`

- 普通方块：`0.6`
- 粘液块：`0.8`
- 冰：`0.98`
- 浮冰：`0.98`
- 霜冰：`0.98`
- 蓝冰：`0.989`

### 8.3 `ItemEntity` 实际用到的水平阻尼

`[源码已证实]`

若本 tick 落地，则：

```text
horizontalMultiplier = blockBelow.getFriction() * 0.98
```

对应常见值：

- 普通方块：`0.6 * 0.98 = 0.588`
- 粘液块：`0.8 * 0.98 = 0.784`
- 冰/浮冰/霜冰：`0.98 * 0.98 = 0.9604`
- 蓝冰：`0.989 * 0.98 = 0.96922`

### 8.4 `speedFactor` 为什么通常不直接影响标准 1 格深水道

`[源码已证实]`

`Entity.move()` 末尾还会乘一次：

```text
deltaMovement *= getBlockSpeedFactor()
```

但 `getBlockSpeedFactor()` 的规则是：

- 若实体当前所在格是 `WATER` 或 `BUBBLE_COLUMN`
  - 直接返回当前格自己的 `speedFactor`
- 不再去读脚下 `getBlockPosBelowThatAffectsMyMovement()`

而水方块的 `speedFactor` 是默认 `1.0`。

因此：

- `[源码已证实]` 标准 1 格深水道里，水下蜂蜜块、灵魂沙这类 `speedFactor` 通常不会直接进入 `ItemEntity` 的本 tick 乘法链。
- `[仍需实测]` 若掉落物在浅水、边界或部分离水状态下，当前所在格不再是 `WATER`，则 `speedFactor` 的参与条件会变化。

### 8.5 粘液块不只有 `friction = 0.8`

`[源码已证实]`

`Entity.move()` 在 `onGround && !isSteppingCarefully()` 时会调用 `block.stepOn(...)`。

`SlimeBlock.stepOn()` 里：

- 若 `abs(vy) < 0.1`
- 且实体不潜行

则再做：

```text
e = 0.4 + 0.2 * abs(vy)
vx *= e
vz *= e
```

因为掉落物不会潜行，所以源码上这条逻辑对掉落物是可达的。

结论：

- `[源码已证实]` 粘液块当地板时，可能在 `friction = 0.8` 之外，再额外缩一次水平速度。
- `[仍需实测]` 在具体水道中，这条 `stepOn()` 会在多少 tick 真正触发，取决于掉落物是否被判定为 `onGround`。

---

## 9. 水面碰撞承托为什么通常不适用于掉落物

`[源码已证实]`

`LiquidBlock.getCollisionShape()` 只有在以下条件同时满足时，才给出源水表面的稳定碰撞面：

1. `state.getValue(LEVEL) == 0`
2. `context.isAbove(STABLE_SHAPE, pos, true)`
3. `context.canStandOnFluid(level.getFluidState(pos.above()), this.fluid)`

而 `EntityCollisionContext` 对非 `LivingEntity` 的 `canStandOnFluid` 默认是：

```text
fluid -> false
```

因此：

- `[源码已证实]` `ItemEntity` 不能像某些可站水实体那样，把源水表面直接当作碰撞承托面。

---

## 10. `PistonMovingBlockEntity` 与粘液块弹射

### 10.1 tick 时序

`[源码已证实]`

世界中先 tick 实体，后 tick 方块实体。

所以“移动中的粘液块/活塞方块实体改写速度”发生在该游戏刻的 **方块实体阶段**，掉落物会在 **下一个实体 tick** 带着这份新速度进入水道模型。

### 10.2 粘液碰撞时的速度改写

`[源码已证实]`

`PistonMovingBlockEntity.moveCollidedEntities()` 中：

- 若移动中的方块是 `SLIME_BLOCK`
- 且被撞实体不是 `ServerPlayer`

则按活塞运动轴直接改写该轴分量：

- X 轴：`vx = direction.getStepX()`
- Y 轴：`vy = direction.getStepY()`
- Z 轴：`vz = direction.getStepZ()`

也就是：

```text
+1 或 -1
```

### 10.3 为什么常提到 `0.51`

`[源码已证实]`

类里声明了：

```text
TICK_MOVEMENT = 0.51
```

而实际推动逻辑中：

1. 每 tick 进度推进 `0.5F`
2. 碰撞推出量使用：

```text
i = min(i, d) + 0.01
```

其中 `d = f - progress`，常规情况下就是 `0.5`

所以单 tick 最大推出量是：

```text
0.5 + 0.01 = 0.51
```

### 10.4 对建模的意义

结论：

- `[源码已证实]` “粘液块活塞弹射进水道”不是单纯加速，而是：
  - 先可能把某一轴速度硬改成 `±1`
  - 同 tick 再可能被活塞位移一段，理论上上限约 `0.51`
- `[仍需实测]` 若某布局里掉落物在连续多个 tick 与移动方块继续接触，则速度可能被多次重写，最终入水瞬间状态要看实际相位。

---

## 11. 可直接使用的 1 维离散模型

以下只写主运输方向速度 `v`，并作这些前提：

- `[源码已证实]` 单体 `ItemEntity`
- `[源码已证实]` 不考虑多实体互推和合并
- `[源码已证实]` 不考虑横向墙撞把速度清零
- `[源码已证实]` `getFlow()` 在该方向上非零时，单次推力按 `0.014 * s`
- `[仍需实测]` 某 tick 是否落地、`s_pre/s_post` 是否为 `0/+1/-1`，取决于实际布局

记：

- `v_t`：tick 开始时主方向速度
- `s_pre, s_post ∈ {-1, 0, +1}`：本 tick 前后两次水推方向
- `mu`：本 tick 若落地时读到的地板摩擦

### 11.1 完全浸没，且本 tick 执行水中漂浮分支

若本 tick 移动后不落地，则：

```text
v_(t+1) = 0.99 * 0.98 * v_t + 0.99 * 0.98 * 0.014 * s_pre + 0.014 * s_post
         = 0.9702 * v_t + 0.0135828 * s_pre + 0.014 * s_post
```

若稳定顺流 `s_pre = s_post = +1`：

```text
v_(t+1) = 0.9702 * v_t + 0.0275828
```

若本 tick 移动后落地，则把 `0.98` 换成 `0.98 * mu`：

```text
v_(t+1) = 0.9702 * mu * v_t + 0.0135828 * mu * s_pre + 0.014 * s_post
```

### 11.2 接触到水，但浸没深度不够，仍走重力分支

这时水平不乘 `0.99`，但两次水推仍可能发生。

若移动后不落地：

```text
v_(t+1) = 0.98 * v_t + 0.98 * 0.014 * s_pre + 0.014 * s_post
         = 0.98 * v_t + 0.01372 * s_pre + 0.014 * s_post
```

若移动后落地：

```text
v_(t+1) = 0.98 * mu * v_t + 0.01372 * mu * s_pre + 0.014 * s_post
```

### 11.3 静水/零流向段

若 `s_pre = s_post = 0`：

- 完全浸没、离地：
  - `v_(t+1) = 0.9702 * v_t`
- 完全浸没、落地：
  - `v_(t+1) = 0.9702 * mu * v_t`
- 不足浸没、离地：
  - `v_(t+1) = 0.98 * v_t`
- 不足浸没、落地：
  - `v_(t+1) = 0.98 * mu * v_t`

### 11.4 反向水流段

若 `s_pre = s_post = -1`，只需把对应常数项改成负号即可。

例如完全浸没、离地时：

```text
v_(t+1) = 0.9702 * v_t - 0.0275828
```

---

## 12. 本轮明确修正/补全的关键点

### 12.1 已从源码确认并在模型中固定的点

- `[源码已证实]` 掉落物通常每 tick 两次水推。
- `[源码已证实]` 水中漂浮阈值是 `0.10138889`，不是“只要在水里就漂浮”。
- `[源码已证实]` `fluidHeight` 存的是最大浸没深度。
- `[源码已证实]` `FlowingFluid.getFlow()` 返回前 `normalize()`。
- `[源码已证实]` 非玩家实体在流体推进里还会再 `normalize()` 一次。
- `[源码已证实]` 落地摩擦只在 `ItemEntity.tick()` 的 `onGround` 分支里通过 `getFriction() * 0.98` 进入。
- `[源码已证实]` 水下 `speedFactor` 一般不直接读到脚下蜂蜜块/灵魂沙。
- `[源码已证实]` 粘液块当地板时，不只是 `friction = 0.8`，还可能触发 `stepOn()` 额外减速。
- `[源码已证实]` 移动中的粘液块碰撞会把非玩家实体某一轴速度硬设为 `±1`。
- `[源码已证实]` 活塞同 tick 的实体推出量上限约 `0.51`。

### 12.2 不能只靠源码定死、需要后续实测的点

- `[仍需实测]` 某具体水道布局里，掉落物在哪些 tick 被判定为 `onGround`。
- `[仍需实测]` 浅水尾段、边缘段、源水段里，何时从漂浮分支切到重力分支。
- `[仍需实测]` 多格同时接触水、转角、扩宽、局部下落时，`getFlow()` 平均化后的真实方向。
- `[仍需实测]` 粘液块 `stepOn()` 在水道底板设计里究竟贡献多少额外减速。
- `[仍需实测]` “活塞/粘液弹射进水道”的实际入水状态，是否会被连续多 tick 的移动方块接触再次重写。

---

## 13. 后续实测建议

1. 先做单实体、单段直线水道，不要混入多实体合并。
2. 先分开测三类段：
   - 深水顺流段
   - 浅水尾段
   - 静水/反向水流刹车段
3. 若要测底板差异，建议把“是否触发 `onGround`”和“是否触发 `stepOn()`”一起记录。
4. 若要测活塞入水，建议记录：
   - 活塞方块实体 tick 的相位
   - 入水前最后一刻的 `deltaMovement`
   - 是否连续多 tick 继续接触移动方块

---

## 14. 简短结论

本轮按源码核对后，1.17.1 掉落物水道模型的主链已经可以稳定写成：

```text
第一次水推
-> 水中漂浮 or 重力
-> move/碰撞
-> 落地摩擦与反弹
-> 第二次水推
```

其中最重要的工程事实是：

- 水推通常一 tick 两次
- 深水直线顺流的单次有效水平推力通常是 `0.014`
- 冰系差异只在“本 tick 是否落地”这一前提成立时才通过摩擦生效
- 活塞粘液弹射属于“硬写速度 + 再推出位移”，不是普通顺流加速

这几条已经足以作为后续实测和布局搜索的源码基线。
