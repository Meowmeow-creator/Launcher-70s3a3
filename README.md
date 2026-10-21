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
| 2 查存档里的账号和角色 | 服务端在线时读 FG201 `status` 实时快照；服务端空闲时直接解析 `roles-CX.xml` / `economy-*.xml` / `progression-*.xml` / `skills-*.xml` | 两种模式都已实测 |
| 3 改角色：发道具 / 加等级 / 读已接任务并完成选定任务 / 转职 / 觉醒 | 服务端 `GM-Console-FG201.cs` 的 6 个操作，全部走既有事务与任务表结算 | 已实测（见下） |
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
   | `status` | — | 角色/任务/背包/地上条目快照 |
   | `level` | `to=1..70` | 写该等级门槛经验；若已分技能点不允许降级透支 |
   | `item` | `item,count=1..1000000,shortcut=0/1` | 复用 FG106 的发放与满包无损判定 |
   | `promote` | `grow=1..4` | 一次转职 grow 字节；已转职则拒绝 |
   | `awaken` | — | 在现有一次转职上叠加首次觉醒，并按 grow 发觉醒技能 |
   | `quest-accept` | `id` | 合成 op33 请求体走客户端接取线路 |
   | `quest-complete` | `id,accept=0/1,topup=0/1,selection?` | 先把任务目标计数器写满、按需补齐提交物，再合成 op36 走真实结算线路 |
   | `dungeon-killall` | `limit=1..40`（默认 20） | **排队**清图：把本房间未死的 uid 入队，每 350ms 结算一只（经验/掉落/任务计数照常登记）；队列未空时再按会被拒 |
   | `loot-pickall` | — | 逐个拾取当前地上条目 |
| `loot-autopick` | `enable=0/1` | 自动拾取常开开关，写 `<store>\TestTools\gm-toggles.xml`；服务端每 100ms 按 mtime 重读，重启后仍生效。掉落后**延迟 400ms 再捡**（等客户端先把地上条目生成出来），换房间自动作废 |
   | `shutdown` | — | 让频道服在空闲循环里正常退出 |

   客户端在线且已选角时，这些操作会照旧下发属性/技能/背包/任务包，游戏内即时可见；
   没有客户端时只写存档，下次进城镇生效。回执消息会明确写出是哪种。

## 一键清图的正确姿势（曾经的崩溃原因）

同步版一次性把整房间杀完，实测把客户端打崩过：`12:58:26` 连点 5 次清图后服务端写 socket 抛
`GM_FAULT ... IOException 无法将数据写入传输连接`，即客户端在包洪中掉线。现在服务端每次只结算一只
（350ms 一只），死亡帧实测落在 +700/+1015/+1591/+2183/+2750/+3331ms；换房间或首领房已满足条件时队列自动作废并记
`GM_KILLALL_ABORTED`。所以清图请**一个房间按一次**，等它跑完（回执会写"已排队 N 只"）。

## 全局快捷键

设置页 → "一键清图全局快捷键"：点输入框后按组合键（必须含 Ctrl/Alt/Shift/Win，避免吃掉游戏按键），
点"登记"即时生效，不需要重启登录器；"清除"取消。实现是 `src-tauri/src/hotkey.rs` 里直接 P/Invoke
`RegisterHotKey` + 一个消息泵线程（不额外引 crate），按下即在后台发一次 `dungeon-killall`，
结果通过 `gm-event` 回推到界面底部状态条。被系统占用时 `hotkey-state` 会提示换一个组合。

## 界面结构

任务页只保留两块：**已接任务**表（含"重新读取任务"，强制向服务端重读并回显 `服务端实时 · 已接 N / 已完成 M · 时间`）
与其下方的**完成选定任务**；任务框可直接输入任务 ID（点行会自动填）。任务检索面板与"已完成"清单框已按要求删除。

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

## 配置

`%LOCALAPPDATA%\70S3A3单机启动器\config.json`：交付根目录、存档目录、日志目录、客户端目录/可执行文件、
入口端口（必须 17000..17999，服务端只认这段测试端口）、频道端口（默认 7002，需与频道表一致）、
启动参数、秒怪便利开关。默认按 `D:\70S3A3` 结构推断：

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

未实测：

- 真机里再点一次区域移动（服务端侧已由抓包回放证明放行）与一键清图；
- 副本内即时推送类包的观感（`SEND_SKIPPED_NO_PEER` 只在无客户端时出现，有客户端时的表现需要真人开游戏看）；
- `dungeon-killall` / `loot-pickall`（需要客户端进副本，服务端在没有连接时直接拒绝）；
- 真实客户端里"启动客户端"按钮拉起 v50 后双击频道→游戏开始的全流程（参数与手工流程一致，但没在这个 GUI 里走过）。

## 边界

- 只允许 127.0.0.1 的服务端；令牌只存在于握手文件与登录器进程内。
- GM 请求不是玩家协议，不改变任何掉落率/概率；所有写入都走服务端既有事务，重复回执号不会二次发放。
- 不动玩家真实存档：默认存档目录就是可丢弃的 `DNF70-batch-test-FG`；换成别的目录前请先确认。
- 任务"完成选定任务"是 GM 直令：目标计数器由服务端写成完成，奖励仍由任务表结算，日志里记 `GM_QUEST_COMPLETED`。
