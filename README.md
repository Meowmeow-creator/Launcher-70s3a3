# 70S3A3 单机启动器

单用途登录器：一键起服、查存档里的账号角色、对角色做 GM 操作。技术栈与 90US 登录器一致（Tauri 2 + Rust + TypeScript），
但只做单机：不联网校验、不读别人家的 `inventory.db`，GM 走 70S3A3 服务端自己的文件式控制通道。

参考源码：`D:\USDOF-laucher\90US_Launcher-master`（`apps/standalone` 的进程管理、GM 面板、spawn_blocking 结构，
以及"GM 端点只允许 127.0.0.1、令牌不落 DFO 参数"的纪律）。视觉框架没有沿用它的紫色皮肤，改成信息密度更高的控制台布局。

出处说明：`src-tauri/icons/` 下 4 个图标文件直接复制自 liuss66 的 90US 登录器仓库，仅用于本机自用构建；
对外发布或换皮肤前请先替换它们（`tauri.conf.json` 的 `bundle.icon` 列出了这几个路径）。
其余 Rust/TS 代码（服务管理、存档直读、FG201 GM 客户端、PVF 名字解析、全局快捷键、界面）为本仓库新写。

## 功能对应

| 需求 | 落点 | 状态 |
|---|---|---|
| 1 一键启动服务端 | 服务端页 → `service_start`：按仓库产物目录起频道服(7002)与入口服(17001)，日志先归档再新建（证据日志用 CreateNew 打开） | 已实测进程与端口 |
| 2 查存档里的账号和角色 | 服务端在线时读 FG201 `status` 实时快照；服务端空闲时直接解析 `roles-CX.xml` / `economy-*.xml` / `progression-*.xml` / `skills-*.xml`。**FG202 之后每个角色一套档**：id1 仍是 `economy-account1-yoyo-FG.xml`，其它按 `r<id>` 命名（照抄服务端 `Role-Scope-FG202.cs:18-20`），角色页可切换查看任意一个 | 两种模式都已实测（6 角色名册） |
| 3 改角色：发道具（含强化/品质） / 发点券 / 发金币 / 加等级 / 读已接任务并完成选定任务 / 转职 / 换转职分支 / 觉醒 | 服务端 `GM-Console-FG201.cs` 的 15 个操作，全部走既有事务与任务表结算；**先选 GM 目标角色再操作**（`role` 操作让服务端重绑，见下） | 已实测（见下） |
| 4 一键清图 | `dungeon-killall`（带 limit）复用服务端 `KillMonster(uid)` 登记 | **代码已接，未在真实客户端里验证** |
| 5 自动拾取 | `loot-pickall` 复用服务端 `Pickup` 路径 | **代码已接，未在真实客户端里验证** |

4/5 需要客户端连上频道服并进入副本，服务端才承认（否则直接返回"需要客户端已经进入副本"）。

## GM 控制通道（FG201）

服务端在存档目录里开 `<store>\TestTools\`：

1. `service.xml` 是握手文件：`token`（进程内随机）、`pid`、`service`、`store`、`role`、`ready`、`level`、`town`。
   `ready=1` 表示客户端已选角且不在副本。
2. 请求：`<store>\TestTools\<32位小写十六进制>.gm.xml`
   ```xml
   <GMRequest version="FG201" token="…" receipt="同一个GUID" op="level" to="70"/>
   ```
   服务端在主循环里每 100ms 轮询一次，和玩法请求同线程处理，所以任何改动都走既有的落盘事务。
3. 回执：`<receipt>.gm.reply.xml`，请求文件被服务端删除。回执含 `success/message` 与一份新的 `Snapshot`。
4. 操作与边界（服务端会独立再校验一遍，登录器的校验只为早报错）：

   | op | 参数 | 行为 |
   |---|---|---|
   | `status` | — | 角色/任务/背包/地上条目快照（含 `roleId`/`job`/`jobName`/`awakeningName`，以及名册全员工号） |
   | `role` | `roleId` | **换 GM 目标角色**：没有客户端连接时让服务端重绑到名册里的任意角色（照抄 opcode 4 选角的顺序：清会话缓存→挪槽位→`BindRole`→`Sync`），后续写操作都落在该角色的档上 |
   | `level` | `to=1..70` | 写该等级门槛经验；若已分技能点不允许降级透支 |
   | `item` | `item,count=1..1000000,shortcut=0/1`（可选 `upgrade=0..31`、`seed=0..2147483647`、`reseal=0..7`、`sealed=0/1`） | 复用 FG106 的发放与满包无损判定；装备类（物品类型 4/8）可顺带写强化等级、品质种子、再封印次数与封印状态，非装备传这些属性会被服务端拒 |
   | `cash` | `amount=1..99999999` | 给 `OfflineCash`（商城点券）加钱，超钱包上限明确报错不静默截断；客户端在线时同步推 op53 余额包。**点券是账号钱包**（服务端 FG271）：加给谁都是加进 `cash-account1-FG271.xml`，同账号所有角色共用一笔。`BoundCash`（绑定代币券）本服务端没有任何发放线路，只在快照里回读 |
   | `gold` | `amount=1..99999999` | 给角色金币加钱，走 FG106 的同一笔钱包事务；超钱包上限 999999999 明确报错不静默截断，客户端在线时推 op14 金币行即时刷新 |
   | `promote` | `grow=1..4`（按职业收窄） | 一次转职 grow 字节 + 该职业该分支的赠技；已转职则拒绝，职业没有这条分支也拒绝 |
   | `change-branch` | `grow=1..4`（按职业收窄） | 已转职角色换本职业的另一条转职分支：先把技能档退回本职业未转职基线（等同免费洗点、觉醒清零、第二技能栏一并退回），再写新 grow。未转职/同分支/越界分支都被拒 |
   | `awaken` | — | 在现有一次转职上叠加首次觉醒，并按 (job,grow) 发觉醒技能 |
   | `quest-accept` | `id` | 合成 op33 请求体走客户端接取线路 |
   | `quest-complete` | `id,accept=0/1,topup=0/1,selection?` | 先把任务目标计数器写满、按需补齐提交物，再合成 op36 走真实结算线路 |
   | `dungeon-killall` | `limit=1..40`（默认 20） | **排队**清图：把本房间未死的 uid 入队，每 350ms 结算一只（经验/掉落/任务计数照常登记）；队列未空时再按会被拒 |
   | `loot-pickall` | — | 逐个拾取当前地上条目 |
| `loot-autopick` | `enable=0/1` | 自动拾取常开开关，写 `<store>\TestTools\gm-toggles.xml`；服务端每 100ms 按 mtime 重读，重启后仍生效。掉落后**延迟 400ms 再捡**（等客户端先把地上条目生成出来），换房间自动作废 |
   | `shutdown` | — | 让频道服在空闲循环里正常退出 |

   客户端在线且已选角时，这些操作会照旧下发属性/技能/背包/任务包，游戏内即时可见；
   没有客户端时只写存档，下次进城镇生效。回执消息会明确写出是哪种。

## 多角色：GM 目标角色与按职业的转职名

服务端同一时刻只"绑定"一个角色（建角、客户端 opcode 4 选角、或登录器发 `role`），所有写操作都落在绑定角色的档上。
角色页顶部的下拉就是干这个的：

- 选中另一个角色 → 写进 `config.json` 的 `gmRole`（下次启动仍认它），服务端空闲时立即发一次 `role` 重绑；
- **客户端在线时不抢绑定**：服务端会拒（会话状态必须跟玩家看到的一致），界面据此禁用写入按钮并说明原因，
  等玩家退出到选角界面或把目标改成玩家正在玩的那个角色才能继续；
- 服务端没起（只有 `roles-CX.xml` 可读）时，下拉切换＝换读哪个角色的档，纯只读；此时点写入会得到
  "服务端还没有开放 GM 通道"，不会偷偷改文件——所有写入一律走服务端事务。

转职按钮的名字按当前角色的 `job` 渲染，来源是服务端 `data/quest-catalog.xml` 里 `JobChange=true` 且
`RewardGrowType>0` 的"转职 - X (n/m)"结算步，一次觉醒名来自觉醒链的 `RewardSecondGrow=1` 结算步（分支号取该任务的
`<GrowType>`）。九职业枚举顺序见 `local-run/recon/role-create-jobs-report.md` §2。暗夜使者（job 6）与男魔法师（job 8）
在这套脚本里只有 grow 1、2 两条分支，所以只出两个按钮；服务端对越界的 grow 会直接拒。
同一分支若同时存在 legacy `quest/cchange` 与现行 `n_quest` 两行，取 `Supported=true` 的那条（剑魂显示 7803 而不是 806，
按任务线路走 806 会被 `Quest unavailable` 拒掉）。

## 一键清图的正确姿势（曾经的崩溃原因）

同步版一次性把整房间杀完，实测把客户端打崩过：`12:58:26` 连点 5 次清图后服务端写 socket 抛
`GM_FAULT ... IOException 无法将数据写入传输连接`，即客户端在包洪中掉线。现在服务端每次只结算一只
（350ms 一只），死亡帧实测落在 +700/+1015/+1591/+2183/+2750/+3331ms；换房间或首领房已满足条件时队列自动作废并记
`GM_KILLALL_ABORTED`。所以清图请**一个房间按一次**，等它跑完（回执会写"已排队 N 只"）。

## 全局快捷键

**出厂默认 `Ctrl+Alt+K`**：`config.json` 里没有登记过组合键时（含老配置里显式写着 `hotkeyVk: 0`
的那批），启动即自动注册这一个，装好就能在游戏里直接按，不必先进登录器设置。玩家主动点过"清除"
会写 `hotkeyCleared: true`，之后启动不再补回来。

设置页 → "一键清图全局快捷键"：点输入框后按组合键（必须含 Ctrl/Alt/Shift/Win，避免吃掉游戏按键），
点"登记"即时生效，不需要重启登录器；"清除"取消。实现是 `src-tauri/src/hotkey.rs` 里直接 P/Invoke
`RegisterHotKey` + 一个消息泵线程（不额外引 crate），按下即在后台发一次 `dungeon-killall`，
结果通过 `gm-event` 回推到界面底部状态条。被系统占用时 `hotkey-state` 会提示换一个组合。

## 界面结构

任务页只保留两块：**已接任务**表（含"重新读取任务"，强制向服务端重读并回显 `服务端实时 · 已接 N / 已完成 M · 时间`）
与其下方的**完成选定任务**；任务框可直接输入任务 ID（点行会自动填）。任务检索面板与"已完成"清单框已按要求删除。

角色页顶部是 **GM 目标角色** 下拉（`名字 · 职业 · Lv. · 转职`）＋名册表（每个角色自己的等级/金币/一次转职，实时模式下由登录器按各自存档补出来，
因为服务端 `Roles` 只带 `roles-CX.xml` 里建角时的 1 级）；点行等同于选下拉。右侧操作区按当前职业渲染转职按钮，
觉醒按钮写成"一次觉醒 → 剑圣/武神/魔皇…"这种带该分支觉醒名的形式。

## 构建

秒怪便利（`DNF70_TEST_ONE_HIT_KILL=1`）默认**关闭**：开着它做清图/掉落类验证会失真，需要时在设置页勾回来。

需要 `cargo`（已在 PATH）与 Node。Node 在本机不在 PATH，用 `D:\USDOF-laucher\nodejs\node-v24.21.0-win-x64`。
`node_modules` 是一个 junction，指向 `D:\USDOF-laucher\90US_Launcher-master\node_modules`（依赖版本相同，离线可用）；
若要独立，删掉 junction 后 `npm install`。

```powershell
cd D:\70S3A3\Launcher-70s3a3
npm run build          # tsc + vite build，产出 dist/
npm run build:exe      # tauri build --no-bundle，产出 src-tauri\target\release\launcher-70s3a3.exe
npm run dev:app        # 开发模式
cargo test --manifest-path src-tauri\Cargo.toml
```

绿色 exe 需要 WebView2 运行时（Win11 自带）。当前不打包 NSIS 安装包（`bundle.active=false`），
避免构建期去下载打包工具。

## 这个仓库之外还需要什么

登录器只是半套东西，缺下面任一项都会掉功能，不会静默假装成功：

1. **服务端必须含 FG201 GM 接口**（`Server-70s3a3/src/Tools/VmTestKit/GM-Console-FG201.cs` 与配套的
   `Batch-Service-FG.cs`/`Progression-FG.cs`/`Dungeon-Gate-Protocol.cs` 改动，在那个仓库里，不在本仓库）。
   没有它：`TestTools/` 下不会有人处理 `.gm.xml`，登录器会在 6 秒后报
   "服务端 6 秒内没有回执…"，其余面板（起服、查存档）仍可用。
2. **目录结构**：默认按 `<根>/Server-70s3a3/build/stable`、`<根>/local-run/DNF70-batch-test-FG`、
   `<根>/地下城与勇士/DNF70_v50.exe` 推断；不一样就在设置页改。存档目录名必须仍是
   `DNF70-batch-test-FG`（服务端硬要求），入口端口必须落在 17000..17999。
3. **npm 依赖**：`node_modules/` 不入库。本机是指向 90US 登录器仓库的 junction，换机器请
   `npm install`（需要网络）。Rust 依赖走 `~/.cargo` 缓存，`--offline` 可用是因为版本被钉在
   `src-tauri/Cargo.lock`。
4. **游戏本体**：`DNF70_v50.exe`、`Script.pvf` 等原版客户端与 PVF 不在任何仓库里，需自备；
   登录器只负责带参数拉起它。
5. **pvfUtility（可选）**：物品中文名来自本机 `127.0.0.1:27000` 的读 PVF 服务；没起时道具面板
   只显示 ID 与类型，不影响发放。

## 配置

`%LOCALAPPDATA%\70S3A3单机启动器\config.json`：交付根目录、存档目录、日志目录、客户端目录/可执行文件、
入口端口（必须 17000..17999，服务端只认这段测试端口）、频道端口（默认 7002，需与频道表一致）、
启动参数、秒怪便利开关、`gmRole`（角色页选中的 GM 目标角色 id，0＝不指定；下次启动时若服务端空闲会自动把它切回去）。默认按 `D:\70S3A3` 结构推断：

- 产物：`D:\70S3A3\Server-70s3a3\build\stable`
- 频道表：`D:\70S3A3\Server-70s3a3\data\DNF70-channel-info-CG.etc`
- 存档：`D:\70S3A3\local-run\DNF70-batch-test-FG`（目录名必须是 `DNF70-batch-test-FG`，服务端硬要求）
- 客户端参数：`99?127.0.0.1?<入口端口>?1?1?01?1?0?0?0?0?1?9n2b1c8r3w8y?0?0?19847`
  第 4、5 字段必须是 `1`（服务端只接受长度 1 的 "1"，对应固定离线账号 1）。

## 已实测 / 未实测

已实测（2026-10-20，隔离实例 17003/17005 + 可丢弃存档副本，端口均在 17000..17999）：

- 空闲态 `status` 应答（无客户端连接也能处理请求）；
- `awaken`：grow 1 → 17，觉醒技能 91 Lv.1，可见技能 26；另一份未转职存档 grow 2 → 18，技能 89；
- `promote`：未转职存档 grow=2 成功（鬼泣，可见技能 20）；重复转职被拒；
- `level`：Lv.70→45→70 与 Lv.70→50 均落盘（经验写门槛值）；降到 Lv.1 因技能点透支被拒；
- `item`：1000 持有 5→10；未接入物品 999999 被拒；装备类超 32 件被拒；
- `quest-complete`：`[meet npc]` 211、`[hunt monster]` 4409（还差 20）、`[seeking]` 502（自动补齐 4170×10 后提交）
  三条都按任务表结算，存档里金币 7495→8887、任务点 12→18、Completed 新增对应 ID，补齐的提交物被扣掉；
- 转职/觉醒任务按任务线路走会被等级段或前置拒绝（`Quest unavailable 806/917`），所以转职/觉醒请用角色页的直操作按钮。

界面走查（release exe，2026-10-20）：服务端/角色/任务/道具/设置五页均渲染真实数据——
状态灯显示频道 7002、入口 17001、GM 通道 pid、客户端未运行；角色页列出 `roles-CX.xml` 的 yoyo 与实时快照
（Lv.70、剑魂 grow 1、剩余 SP 7275、金币 7495、背包 11 行 22 件、进行中 16 / 已完成 17）；
任务页显示 16 条已接任务的名字/类型/还差目标/等级段与已完成 ID 列表；道具页显示背包 10 行明细；
设置页回填真实路径与启动参数。未选任务时"完成选定任务"按钮为禁用，物品 ID 为空时"发放"为禁用。

界面与联机实测（release exe，2026-10-20 晚）：

- 用登录器"一键启动服务端"起服成功，频道服日志写入 `LISTENING loopback=7002 noTimer=True interactive=True oneHitKill=0`，
  即秒怪便利确实没带；两端口随后由 `netstat` 复核为监听中；
- 服务停掉时数据源自动降级为"存档直读"，界面仍可查角色/任务/背包；
- 道具页背包 12 行全部带出 PVF 中文名：野草莓 / 运动型短卷发 / 命运硬币 / 普通战斧 / 死神的邀请函 /
  越桔 / 魔力之花 / 有机草莓 / 苎麻花叶 / 邪恶的结晶 / 无色小晶块（含他发的 1000 与 100 两笔）。

服务端配套修复（同批）：`Offline70Gate.Parse` 原来要求所有 op38 都命中门矩形，
把客户端"世界地图/飞艇"跨城镇请求（source 字段为 0）判成非法；现在按 rednf `move_area.inc` 的口径，
跨城镇只校验目标区域存在并记 `route=worldmap`，同城仍按门矩形。用他那一包真实抓包回放验证：
`request-663`（站在 3/6 → 6/0）由 REJECTED 变 ACCEPTED route=worldmap，
`request-657/660`（3/4→3/1、3/1→3/6）仍按 route=door 通过，站在别的区域时依旧被拒。
GM 请求被拒时现在会写 `GM_REJECTED receipt=… op=… reason=…`，之前失败不留痕。
拾取核心从 op46 处理器里抽成 `PickupGroundFG201(slot)`（三种入口共用，不再合成假加密包）；
`service.xml` 与回执都由服务端 temp+replace 发布，登录器读取时按"空内容/非元素起点"重试，
并容忍数据表文件头的 UTF-8 BOM（`quest-catalog.xml` 带 BOM，`item-metadata.xml` 不带）。

自动拾取的实机证据（隔离实例 17003 + 真协议驱动 `local-run\driverutopick`，全程**没有发过一条 op46**）：
`FLOOR_DROP_PLANNED uid=101 item=3211 slot=2000` → `PICKUP_COMMITTED ground=2000 item=3211 count=1` → `AUTO_PICKUP ground=2000`；
关档重启后 `status` 仍回 `autoPickup=1`，UI 开关回读为"常开"，再点一次写回 `autoPickup="0"`。

多角色与按职业转职（2026-10-23，隔离实例 17055 + 可丢弃存档副本 `local-run\isolated-test\gm-role`，驱动脚本在 `local-run\driver\gm-role\gmcli.py`）：

- `role` 重绑：`GM_ROLE_BOUND receipt=… from=1 to=3`／`from=3 to=1`／`to=6` 四类往返都过，`service.xml` 随绑定回显 `roleId`/`job`；
  不存在的目标按名册拒（`角色 99 不在这个账号的角色列表里（现有 id：1、2、3、4、5、6）`）。
- **写操作按角色隔离**：`item 1000×3` 给 yoyo（持有 5→8，`inventoryItems` 1199→1202，revision 896→897），
  同一物品给 2778 是 5→12（13→20，revision 14→15）；切回 yoyo 后仍是 8/1202/897。落盘核对
  `economy-account1-yoyo-FG.xml` 与 `economy-account1-r3-FG.xml` 的 `RoleId/Job/GrowType/Level/Revision` 与回执逐字相符。
- 格斗家（job 1）`promote grow=2` → "转职已写入存档：格斗家-散打（grow 字节 2），可见技能 18 个"；
  `awaken` → "格斗家-散打 → 武神（grow 字节 18），觉醒技能 70 Lv.1，可见技能 19 个"，
  `skills-r3-EZ.xml` 解码后 Ids 末尾确有 70、Levels 对应 1。
- 男魔法师（job 8）`promote grow=3` 按职业被拒："男魔法师 在这套脚本里只有 grow 1、2，没有 grow=3 这个转职分支"；
  `grow=1` 成功 → 元素爆破师，`awaken` → 魔皇（技能 51）。
- 登录器界面实测（dev 构建 + 同一隔离档）：角色页下拉显示 `yoyo · 鬼剑士 · Lv.70 · 剑魂`，名册表 6 行的职业名/等级/金币/一次转职齐全
  （yoyo 70/59138/剑魂、1234 男魔法师 3/244、2778 格斗家 60/散打、2134 与 2313 神枪手、4711 鬼剑士）；
  点名册第 2 行后服务端握手文件与快照同步变成 `roleId 2 / 男魔法师`，转职按钮只剩 `元素爆破师`、`冰结师` 两个；
  yoyo 那一版是 剑魂/鬼泣/狂战士/阿修罗 四个按钮、觉醒按钮写"一次觉醒 → 剑圣"。
- 真档回归：重建 `build\stable` 后重启 7002/17001，`status` 回 `roleId=1 job=0 jobName=鬼剑士 firstGrowName=剑魂
  awakeningName=剑圣 gold=59138 revision=896`，与本轮测试前一致（真档没被写过）。

服务端配套改动（同批，都在 `Server-70s3a3`，已随 `scripts/build.py --smoke` 重编译）：

- `GM-Console-FG201.cs`：新增 `role` 操作（离线重绑，照 opcode 4 的顺序清缓存→挪槽位→`BindRole`→`Sync`，探档失败就把绑定退回原角色）；
  `promote`/`awaken` 去掉"只接入鬼剑士"的硬编码，改按 `(job,grow)` 查分支名并拒越界；快照加 `jobName`/`awakeningName`。
- `Reward-Types-FG183.cs`：九职业一次转职名与一次觉醒名表（FG204）落在这里，GM 文案与档校验共用同一份数据。
- `Economy-State-FG.cs`：`Validate` 里 `s.Job!=0&&s.GrowType!=0` 这条鬼剑士专属 pin 换成"该职业有没有这条分支"，
  否则非鬼剑士一转根本写不进去（实测报 `Economy state profile`）。
- `Promotion-Skills-FG139.cs`：`NormalizeAwakeningStateFG183`/`BaseLevelFG139` 里 `AwakeningSkill(0,grow)` 的 job 0 换成
  `Offline70RoleScopeFG202.CurrentJob`（否则格斗家觉醒抛 `Awakening skill outside the character's own branch`，grow 已写而回执却失败）。
- `Test-Item-Service-FG106.cs`：`service.xml` 加 `roleId`/`job`，并把角色 id 纳入 state 门（同级同镇换绑也要重写握手文件）。

点券 / 换转职分支 / 装备属性（2026-10-24，同一套隔离实例 17055 + 存档副本，`gmcli.py` 驱动）：

- `cash`：yoyo 点券 9400 → 14400（回执同时报"代币券 0 未变"）；`amount=999999999` 与非数字被 1..99999999 边界挡下，
  超钱包上限时服务端 `AddWallet` 直接抛错而不是截断，登录器侧同样先拒。
- `change-branch`：yoyo 从 剑魂+一次觉醒（grow 17）换到 狂战士（grow 3）→ 鬼泣（2）→ 阿修罗（4），
  每次都是 SP 全额退回（6355 → 7470，等于 Lv.70 的 `TotalSp`）、`secondGrow` 归 0、觉醒技能 91 从技能档里消失、
  可见技能 19；未转职角色调用被拒并提示改用 `promote`，重复选同一分支也被拒。
  **带第二技能栏的角色单独验过**：给 role 6 注入合法的 `SecondSkillsFG129`（`Bank=0`、profession=2）后换到阿修罗，
  主栏与第二栏的 `ProfessionFG139` 都变成 4、都带上阿修罗赠技 47/55/61；注入非法 `Bank=1` 时
  `Apply` 在校验阶段抛 `Skill slot store profile`、回执失败且 `Revision` 纹丝不动（9→9），证明写档是原子的。
- `item` 装备属性：`10000 upgrade=12 seed=777 reseal=2 sealed=1` 落盘逐字核对
  （`List=0 Slot=10 Upgrade=12 EquipmentSeed=777 ResealCount=2 Sealed=true Durability=28`）；
  装扮类装备 12400 也收 `upgrade/sealed`；给材料 1000 传 `upgrade` 被拒，并已把英文异常翻成中文说明。
- 登录器侧：`cargo test` 44 passed / 0 failed，`tsc --noEmit` 干净；强化列改为按物品类型 4/8 才显示，
  实时模式下名册的「点券」列由登录器按各角色存档补齐。
  > 更正（2026-10-25，FG271）：点券改成账号钱包后，「按各角色存档补齐」不再成立——角色档里那份只是缓存，
  > 没做过交易的角色还留着旧数额。登录器现在先读 `cash-account1-FG271.xml`，侧车档不存在（老存档/未起新版服务端）
  > 才回落到各角色档；`cargo test` 到 50 passed / 0 failed。

未实测：

- 客户端在线时切目标角色、换分支、发点券（服务端按设计直接拒抢绑定，登录器随即禁用写入；这条只有真人开着游戏才验得动）；
- 非鬼剑士角色**进世界后**的即时推送观感（转职/觉醒的属性与技能包）：服务端侧 `World()` 已按九职业放行，
  但本轮全部写操作都在没有客户端时做的；
- 真机里再点一次区域移动（服务端侧已由抓包回放证明放行）与一键清图；
- 副本内即时推送类包的观感（`SEND_SKIPPED_NO_PEER` 只在无客户端时出现，有客户端时的表现需要真人开游戏看）；
- `dungeon-killall` / `loot-pickall`（需要客户端进副本，服务端在没有连接时直接拒绝）；
- 真实客户端里"启动客户端"按钮拉起 v50 后双击频道→游戏开始的全流程（参数与手工流程一致，但没在这个 GUI 里走过）。

## 边界

- 只允许 127.0.0.1 的服务端；令牌只存在于握手文件与登录器进程内。
- GM 请求不是玩家协议，不改变任何掉落率/概率；所有写入都走服务端既有事务，重复回执号不会二次发放。
- 不动玩家真实存档：默认存档目录就是可丢弃的 `DNF70-batch-test-FG`；换成别的目录前请先确认。
- 任务"完成选定任务"是 GM 直令：目标计数器由服务端写成完成，奖励仍由任务表结算，日志里记 `GM_QUEST_COMPLETED`。
