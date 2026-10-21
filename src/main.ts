import { invoke } from "@tauri-apps/api/core";
import { open as pickDirectory } from "@tauri-apps/plugin-dialog";

type Row = Record<string, unknown>;
type State = {
  config: Row;
  service: Row;
  endpoint: Row | null;
  snapshot: Row | null;
  mode: string;
  note: string;
  gameProcesses: number[];
};

const state = {
  current: null as State | null,
  busy: false,
  page: "service",
  names: {} as Record<string, string>,
  namesRequested: new Set<number>(),
  namesOnline: null as boolean | null,
};

const element = <T extends HTMLElement = HTMLElement>(id: string): T => document.getElementById(id) as T;
const attr = (row: Row | null | undefined, key: string): string => String(row?.[`@${key}`] ?? "");
const rowsOf = (name: string, child?: string): Row[] => {
  const raw = (state.current?.snapshot?.[name] as Row[]) ?? [];
  // The live GM reply nests rows as <ActiveQuests><Quest/></ActiveQuests>, which the XML→JSON
  // mapping turns into [{Quest:[…]}]; the offline archive reader emits the rows directly.
  if (child && raw.length && Array.isArray(raw[0][child])) {
    return raw.flatMap((row) => (row[child] as Row[]) ?? []);
  }
  return raw;
};
const snap = (key: string): string => attr(state.current?.snapshot, key);
const number = (key: string): number => Number(snap(key) || 0);

function message(text: string, kind: "info" | "ok" | "bad" = "info") {
  const ticker = element("ticker");
  ticker.textContent = text;
  ticker.className = `ticker${kind === "info" ? "" : ` ${kind}`}`;
}

function describe(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "操作失败。";
}

function fill(table: HTMLTableElement, head: string[], rows: Row[], render: (row: Row) => string[]): void {
  table.innerHTML = "";
  if (head.length) {
    const thead = table.createTHead();
    const tr = thead.insertRow();
    for (const text of head) {
      const th = document.createElement("th");
      th.textContent = text;
      tr.append(th);
    }
  }
  const body = table.createTBody();
  for (const row of rows) {
    const tr = body.insertRow();
    for (const cell of render(row)) {
      const td = tr.insertCell();
      td.textContent = cell;
    }
  }
}

function pairs(table: HTMLTableElement, entries: [string, string][]): void {
  table.innerHTML = "";
  const body = table.createTBody();
  for (const [key, value] of entries) {
    const tr = body.insertRow();
    const left = tr.insertCell();
    left.textContent = key;
    const right = tr.insertCell();
    right.textContent = value;
    right.className = "mono";
  }
}

function setLamp(id: string, on: boolean, warn = false) {
  const dot = element(id);
  dot.className = `dot ${on ? (warn ? "warn" : "on") : "off"}`;
}

function renderStatus() {
  const current = state.current;
  if (!current) return;
  const service = current.service;
  const channelPort = Number(service.channelPort ?? 0);
  const entrancePort = Number(service.entrancePort ?? 0);
  element("channel-port").textContent = String(channelPort);
  element("entrance-port").textContent = String(entrancePort);
  element("gm-pid").textContent = current.endpoint ? String(current.endpoint.pid ?? "-") : "无握手";
  element("client-pids").textContent = current.gameProcesses.length ? current.gameProcesses.join(", ") : "未运行";
  setLamp("lamp-channel", Boolean(service.channelUp));
  setLamp("lamp-entrance", Boolean(service.entranceUp));
  setLamp("lamp-gm", Boolean(service.channelUp) && Boolean(current.endpoint), !service.channelUp && Boolean(current.endpoint));
  setLamp("lamp-client", current.gameProcesses.length > 0);
  element("role-summary").textContent = current.snapshot
    ? `${snap("roleName")} · Lv.${snap("level")} · ${snap("firstGrowName")}${number("secondGrow") ? " · 已一次觉醒" : ""} · 金币 ${snap("gold")}`
    : "读不到角色数据";
  const badge = element("mode-badge");
  badge.textContent = current.mode;
  badge.className = `badge ${current.mode === "服务端实时" ? "live" : "archive"}`;
}

function renderService() {
  const current = state.current;
  if (!current) return;
  const service = current.service;
  const config = current.config;
  const issue = service.configIssue as string | null;
  const alert = element<HTMLDivElement>("server-issue");
  alert.hidden = !issue;
  if (issue) alert.textContent = `起服前置没满足：${issue}`;
  pairs(element<HTMLTableElement>("service-table"), [
    ["频道服", service.channelUp ? `监听 127.0.0.1:${service.channelPort}（pid ${service.channelPid ?? "-"}）` : `未监听 ${service.channelPort}`],
    ["入口服", service.entranceUp ? `监听 127.0.0.1:${service.entrancePort}（pid ${service.entrancePid ?? "-"}）` : `未监听 ${service.entrancePort}`],
    ["GM 通道", current.endpoint ? `pid ${current.endpoint.pid} · 角色 ${current.endpoint.role} · ready=${current.endpoint.ready ? "1" : "0"}` : "缺少 TestTools/service.xml"],
    ["同镜像进程", `${(service.channelInstances as number[] | undefined)?.length ?? 0} 频道 / ${(service.entranceInstances as number[] | undefined)?.length ?? 0} 入口`],
    ["数据源", current.mode],
    ["说明", current.note],
  ]);
  pairs(element<HTMLTableElement>("paths-table"), [
    ["产物目录", String(config.binDir ?? service.binDir ?? "")],
    ["存档目录", String(config.store ?? "")],
    ["日志目录", String(config.logs ?? "")],
    ["客户端", `${config.clientDir ?? ""}\\${config.clientExe ?? ""}`],
    ["启动参数", String(config.launchArguments ?? "")],
    ["秒怪便利", config.oneHitKill ? "DNF70_TEST_ONE_HIT_KILL=1" : "关闭"],
  ]);
}

const QUEST_COLUMNS = ["ID", "任务名", "类型", "目标", "等级段", "接入"];

function renderCharacter() {
  const current = state.current;
  if (!current) return;
  const roles = rowsOf("Roles", "Role");
  fill(
    element<HTMLTableElement>("roles-table"),
    ["ID", "角色名", "职业", "存档记录等级", "创建时间"],
    roles,
    (row) => [attr(row, "id"), attr(row, "name"), attr(row, "job"), attr(row, "storedLevel"), attr(row, "created")],
  );
  const inventoryHint = current.mode === "存档直读" ? "（已用 SP；剩余 SP 需连服务端查看）" : "";
  pairs(element<HTMLTableElement>("snapshot-table"), [
    ["账号 / 角色", `${snap("account")} / ${snap("roleName")}（roleId ${snap("roleId")}，job ${snap("job")}）`],
    ["等级 / 经验", `${snap("level")} / ${snap("experience")}${snap("nextLevelExperience") ? `（下一级门槛 ${snap("nextLevelExperience")}）` : ""}`],
    ["金币 / 任务点", `${snap("gold")} / ${snap("questPoints")}`],
    ["技能点", current.mode === "存档直读" ? `${snap("spentSp")} 已用${inventoryHint}` : `剩余 ${snap("sp")}`],
    ["转职状态", `${snap("firstGrowName")}（grow ${snap("grow")}，一次 ${snap("firstGrow")}，觉醒 ${snap("secondGrow")}）`],
    ["觉醒技能", snap("awakeningSkill") && snap("awakeningSkill") !== "0" ? `技能 ${snap("awakeningSkill")}` : "无"],
    ["专家职", snap("expertJob") === "0" ? "无" : `类型 ${snap("expertJob")}`],
    ["疲劳 / 城镇", `${snap("fatigueRemaining")} / 存档城镇 ${snap("savedTown")}`],
    ["背包", `${snap("inventoryRows")} 行 / ${snap("inventoryItems")} 件（存档修订 ${snap("revision")}）`],
    ["任务", `进行中 ${snap("activeCount")}，已完成 ${snap("completedCount")}`],
    ["连接状态", `live=${snap("live")} selected=${snap("selected")} ready=${snap("ready")}`],
  ]);
  const promoted = number("firstGrow") !== 0;
  const awakened = number("secondGrow") !== 0;
  element<HTMLSpanElement>("grow-state").textContent = promoted
    ? `当前 ${snap("firstGrowName")}${awakened ? "（已一次觉醒）" : "，可觉醒"}`
    : "尚未转职";
  for (const button of Array.from(document.querySelectorAll<HTMLButtonElement>('[data-gm="promote"]'))) {
    button.title = promoted ? "该角色已转职，服务端不允许改第二次" : `写入 grow=${button.dataset.grow}`;
  }
  const gate = element("character-gate");
  gate.textContent =
    current.mode === "服务端实时" && snap("ready") === "1"
      ? "客户端正在城镇里，操作会即时推送到游戏内。"
      : current.mode === "服务端实时"
        ? "服务端在线但角色不在城镇（未选角或正在副本）：写入存档，回城镇或下次登录后生效。"
        : "服务端空闲：直接写入存档，下次进入城镇生效。";
}

/// The 任务 field accepts "806" or "806 转职 - 剑魂 (4/4)"; the leading number is the quest id.
function selectedQuestId(): number {
  const match = /^(\d{1,5})/.exec(element<HTMLInputElement>("quest-selected").value.trim());
  return match ? Number(match[1]) : 0;
}

const VIRTUAL_KEYS: Record<string, number> = {
  Backspace: 0x08, Tab: 0x09, Enter: 0x0d, Escape: 0x1b, Space: 0x20,
  PageUp: 0x21, PageDown: 0x22, End: 0x23, Home: 0x24, Insert: 0x2d, Delete: 0x2e,
};

type Captured = { modifiers: number; virtualKey: number; label: string };

/// Maps a keydown into RegisterHotKey's fsModifiers/vk pair plus a human label.
function captureHotkey(event: KeyboardEvent): Captured | null {
  const key = event.key;
  if (["Control", "Alt", "Shift", "Meta"].includes(key)) return null;
  let virtualKey = 0;
  let name = key;
  if (/^F([1-9]|1[0-2])$/.test(key)) virtualKey = 0x70 + Number(key.slice(1)) - 1;
  else if (/^[A-Za-z]$/.test(key)) virtualKey = key.toUpperCase().charCodeAt(0);
  else if (/^[0-9]$/.test(key)) virtualKey = key.charCodeAt(0);
  else if (VIRTUAL_KEYS[key] !== undefined) virtualKey = VIRTUAL_KEYS[key];
  else if (key.length === 1) virtualKey = key.toUpperCase().charCodeAt(0);
  if (!virtualKey) return null;
  let modifiers = 0;
  if (event.ctrlKey) modifiers |= 0x2;
  if (event.altKey) modifiers |= 0x1;
  if (event.shiftKey) modifiers |= 0x4;
  if (event.metaKey) modifiers |= 0x8;
  if (!modifiers) return null;
  if (virtualKey >= 0x41 && virtualKey <= 0x5a) name = String.fromCharCode(virtualKey);
  else if (virtualKey >= 0x30 && virtualKey <= 0x39) name = String.fromCharCode(virtualKey);
  const label = `${(modifiers & 0x2 ? "Ctrl+" : "") + (modifiers & 0x1 ? "Alt+" : "") + (modifiers & 0x4 ? "Shift+" : "") + (modifiers & 0x8 ? "Win+" : "")}${name}`;
  return { modifiers, virtualKey, label };
}

let pendingHotkey: Captured | null = null;

function hotkeyStatus(text: string, bad = false) {
  const node = element("hotkey-state");
  node.textContent = text;
  node.className = bad ? "ticker bad" : "muted";
}

async function applyHotkey(target: Captured | null) {
  const box = element<HTMLInputElement>("cfg-hotkey");
  try {
    if (!target) {
      const cleared = await invoke<{ message: string }>("set_hotkey", { modifiers: 0, virtualKey: 0, label: "" });
      box.value = "";
      pendingHotkey = null;
      hotkeyStatus(cleared.message);
      return;
    }
    const reply = await invoke<{ message: string }>("set_hotkey", {
      modifiers: target.modifiers,
      virtualKey: target.virtualKey,
      label: target.label,
    });
    box.value = target.label;
    hotkeyStatus(reply.message);
    void refresh(true);
  } catch (error) {
    hotkeyStatus(describe(error), true);
  }
}

/// 强制向服务端重读一次快照（已接/已完成/可接都以服务端当场答复为准）。
async function refreshQuestView(button?: HTMLElement) {
  const label = button ? button.textContent ?? "" : "";
  if (button) button.textContent = "读取中…";
  try {
    state.current = await invoke<State>("refresh_state");
    renderStatus();
    renderQuests();
    renderItems();
    renderDungeon();
    refreshControls();
    const read = element("quest-read-state");
    read.textContent = `${state.current.mode} · 已接 ${snap("activeCount")} / 已完成 ${snap("completedCount")} · ${new Date().toLocaleTimeString()}`;
    message(`任务已重新读取：${state.current.mode}。`, "ok");
  } catch (error) {
    message(describe(error), "bad");
  } finally {
    if (button && label) button.textContent = label;
  }
}

function renderQuests() {
  const active = rowsOf("ActiveQuests", "Quest");
  const table = element<HTMLTableElement>("active-quests");
  fill(table, QUEST_COLUMNS, active, (row) => {
    const goal = attr(row, "goal");
    const remaining = attr(row, "remaining");
    const recorded = attr(row, "recorded");
    return [
      attr(row, "id"),
      attr(row, "name"),
      attr(row, "type"),
      remaining !== "" ? `还差 ${remaining}` : `${recorded}/${goal}`,
      `${attr(row, "minLevel")}~${attr(row, "maxLevel")}`,
      attr(row, "supported") === "1" ? "是" : `否（${attr(row, "reason")}）`,
    ];
  });
  for (const [index, row] of active.entries()) {
    const tr = table.tBodies[0]?.rows[index];
    if (!tr) continue;
    tr.className = Number(attr(row, "id")) === selectedQuestId() ? "selected" : "";
    tr.onclick = () => {
      element<HTMLInputElement>("quest-selected").value = `${attr(row, "id")} ${attr(row, "name")}`;
      refreshControls();
    };
  }
}

function renderItems() {
  const inventory = rowsOf("Inventory", "Entry");
  fill(
    element<HTMLTableElement>("inventory-table"),
    ["物品 ID", "名称", "数量", "背包页", "槽位"],
    inventory,
    (row) => [attr(row, "item"), state.names[attr(row, "item")] ?? "…", attr(row, "quantity"), attr(row, "list"), attr(row, "slot")],
  );
  void requestNames(inventory.map((row) => Number(attr(row, "item"))).filter(Number.isFinite));
}

/// Names come from the local pvfUtility service (PVF item files); ask once per id and refill the
/// table when the batch lands, so a missing service never blocks the panel.
async function requestNames(ids: number[]) {
  const fresh = ids.filter((id) => id > 2 && !state.namesRequested.has(id));
  if (!fresh.length) return;
  for (const id of fresh) state.namesRequested.add(id);
  try {
    const reply = await invoke<{ names: Record<string, string>; pvfOnline: boolean; message: string }>("item_names", { ids: fresh });
    Object.assign(state.names, reply.names);
    state.namesOnline = reply.pvfOnline;
    if (!reply.pvfOnline) {
      for (const id of fresh) if (!(id in reply.names)) state.namesRequested.delete(id);
      message(`PVF 名字服务不可用：${reply.message}`, "bad");
    }
  } catch (error) {
    state.namesOnline = false;
    for (const id of fresh) state.namesRequested.delete(id);
    message(describe(error), "bad");
  }
  renderItems();
  renderItemResults();
}

let lastItemRows: Row[] = [];

function renderItemResults() {
  fill(
    element<HTMLTableElement>("item-results"),
    ["物品 ID", "名称", "类型", "堆叠上限", "需求等级"],
    lastItemRows,
    (row) => [String(row.id), state.names[String(row.id)] ?? "…", String(row.kindName), String(row.stackLimit), String(row.minimumLevel)],
  );
  bindRows("item-results", lastItemRows, (row) => {
    element<HTMLInputElement>("item-id").value = String(row.id);
    refreshControls();
    message(`已填入物品 ${row.id}${state.names[String(row.id)] ? `（${state.names[String(row.id)]}）` : ""}。`);
  });
}

function bindRows(tableId: string, rows: Row[], onPick: (row: Row) => void) {
  const table = element<HTMLTableElement>(tableId);
  rows.forEach((row, index) => {
    const tr = table.tBodies[0]?.rows[index];
    if (!tr) return;
    tr.style.cursor = "pointer";
    tr.onclick = () => onPick(row);
  });
}

function renderDungeon() {
  const inDungeon = number("dungeon") === 1;
  const alert = element("dungeon-state");
  alert.hidden = inDungeon;
  const live = number("live") === 1;
  alert.className = `alert ${live ? "warn" : ""}`;
  alert.textContent = inDungeon
    ? ""
    : live
      ? "客户端已连接但当前不在副本，进入副本后再用一键清图与“拾取当前地上条目”。"
      : "一键清图与逐次拾取需要客户端连上频道服并进入副本（服务端只登记当前房间的怪物）；自动拾取开关不要求客户端在线。";
  const autopick = element("autopick-toggle");
  const on = snap("autoPickup") === "1";
  autopick.textContent = `自动拾取：${snap("autoPickup") === "" ? "未知" : on ? "常开" : "关闭"}`;
  autopick.className = on ? "primary" : "";
  pairs(element<HTMLTableElement>("dungeon-table"), [
    ["副本 / 难度", inDungeon ? `${snap("dungeonId")} / ${snap("difficulty")}` : "-"],
    ["房间", inDungeon ? `(${snap("roomX")},${snap("roomY")}) 加载=${snap("roomLoaded")}` : "-"],
    ["本房间怪物", inDungeon ? `${snap("roomMonsters")} 只，可清=${snap("readyToClear")}` : "-"],
    ["地上条目", `${rowsOf("Ground", "Entry").length} 个`],
  ]);
  fill(element<HTMLTableElement>("ground-table"), ["槽位", "物品", "数量"], rowsOf("Ground", "Entry"), (row) => [
    attr(row, "slot"),
    attr(row, "item"),
    attr(row, "quantity"),
  ]);
}

function fillSettings() {
  const config = state.current?.config ?? {};
  const map: [string, string][] = [
    ["cfg-root", "root"],
    ["cfg-store", "store"],
    ["cfg-logs", "logs"],
    ["cfg-client-dir", "clientDir"],
    ["cfg-client-exe", "clientExe"],
    ["cfg-arguments", "launchArguments"],
  ];
  for (const [id, key] of map) element<HTMLInputElement>(id).value = String(config[key] ?? "");
  element<HTMLInputElement>("cfg-channel-port").value = String(config.channelPort ?? 7002);
  element<HTMLInputElement>("cfg-entrance-port").value = String(config.entrancePort ?? 17001);
  element<HTMLInputElement>("cfg-one-hit").checked = Boolean(config.oneHitKill);
  element<HTMLInputElement>("cfg-hotkey").value = String(config.hotkeyLabel ?? "");
}

async function refresh(quiet = false) {
  try {
    state.current = await invoke<State>("refresh_state");
    renderStatus();
    renderService();
    renderCharacter();
    renderQuests();
    renderItems();
    renderDungeon();
    refreshControls();
    if (!quiet) fillSettings();
    if (!quiet) message(state.current.note, "info");
  } catch (error) {
    message(describe(error), "bad");
  }
}

function refreshControls() {
  const busy = state.busy;
  const current = state.current;
  const readyForMutation = Boolean(current?.snapshot);
  const inTown = snap("ready") === "1" || snap("live") !== "1";
  const promoted = number("firstGrow") !== 0;
  const awakened = number("secondGrow") !== 0;
  const live = number("live") === 1;
  const actions: [string, boolean][] = [
    ["[data-gm=\"level\"]", readyForMutation && !busy],
    ["[data-gm=\"promote\"]", readyForMutation && !busy && !promoted],
    ["[data-gm=\"awaken\"]", readyForMutation && !busy && promoted && !awakened],
    ["[data-gm=\"item\"]", readyForMutation && !busy && Number(element<HTMLInputElement>("item-id").value) > 0],
    ["[data-gm=\"quest-complete\"]", readyForMutation && !busy && selectedQuestId() > 0 && inTown],
    ["[data-gm=\"quest-accept\"]", readyForMutation && !busy && selectedQuestId() > 0 && inTown],
    ["#kill-all", readyForMutation && !busy && live],
    ["#pick-all", readyForMutation && !busy && live],
    ["#autopick-toggle", readyForMutation && !busy],
    ["#start-server", !busy],
    ["#stop-server", !busy],
    ["#launch-client", !busy],
  ];
  for (const [selector, enabled] of actions) {
    for (const button of Array.from(document.querySelectorAll<HTMLButtonElement>(selector))) {
      button.disabled = !enabled;
    }
  }
  if (!inTown && readyForMutation) {
    element("character-gate").textContent = "角色正在副本里：等级/道具/转职/觉醒需要回城镇，清图与拾取只能在副本里用。";
  }
}

function setBusy(busy: boolean) {
  state.busy = busy;
  refreshControls();
}

async function runGm(op: string, params: Record<string, unknown>) {
  if (state.busy) return;
  setBusy(true);
  message(`正在执行 ${op}…`);
  try {
    const reply = await invoke<{ op: string; message: string; snapshot: Row }>("gm_command", { op, params });
    element<HTMLTextAreaElement>("gm-result").value = `${reply.op}：${reply.message}\n\n${JSON.stringify(reply.snapshot, null, 2).slice(0, 4000)}`;
    if (state.current) state.current.snapshot = reply.snapshot;
    renderStatus();
    renderCharacter();
    renderQuests();
    renderItems();
    renderDungeon();
    refreshControls();
    message(`${reply.op} 完成：${reply.message}`, "ok");
  } catch (error) {
    const text = describe(error);
    element<HTMLTextAreaElement>("gm-result").value = text;
    message(`${op} 失败：${text}`, "bad");
  } finally {
    setBusy(false);
    void refresh(true);
  }
}

function gmParams(op: string, button?: HTMLButtonElement): Record<string, unknown> {
  if (op === "level") return { to: Number(element<HTMLInputElement>("level-input").value) };
  if (op === "promote") return { grow: Number(button?.dataset.grow ?? 0) };
  if (op === "item") {
    return {
      item: Number(element<HTMLInputElement>("item-id").value),
      count: Number(element<HTMLInputElement>("item-count").value),
      shortcut: element<HTMLInputElement>("item-shortcut").checked ? 1 : 0,
    };
  }
  if (op === "quest-complete") {
    const params: Record<string, unknown> = {
      id: selectedQuestId(),
      accept: element<HTMLInputElement>("quest-accept").checked ? 1 : 0,
      topup: element<HTMLInputElement>("quest-topup").checked ? 1 : 0,
    };
    const selection = Number(element<HTMLInputElement>("quest-selection").value);
    if (Number.isFinite(selection) && selection > 0) params.selection = selection;
    return params;
  }
  if (op === "quest-accept") return { id: selectedQuestId() };
  if (op === "dungeon-killall") return { limit: Number(element<HTMLInputElement>("kill-scope").value) };
  return {};
}

async function searchItems() {
  const query = element<HTMLInputElement>("item-query").value;
  try {
    const result = await invoke<{ rows: Row[] }>("item_search", { query, limit: 60 });
    lastItemRows = result.rows;
    renderItemResults();
    void requestNames(result.rows.map((row) => Number(row.id)).filter(Number.isFinite));
    message(`物品检索命中 ${result.rows.length} 条。`);
  } catch (error) {
    message(describe(error), "bad");
  }
}

async function service(action: "start" | "stop") {
  setBusy(true);
  message(action === "start" ? "正在启动服务端…" : "正在停止服务端…");
  try {
    const reply = await invoke<{ service: Row; state: State }>(action === "start" ? "service_start" : "service_stop");
    state.current = reply.state;
    renderStatus();
    renderService();
    renderCharacter();
    renderQuests();
    renderItems();
    renderDungeon();
    refreshControls();
    fillSettings();
    message(String(reply.service.message ?? "已完成。"), reply.service.started || reply.service.stopped ? "ok" : "bad");
  } catch (error) {
    message(describe(error), "bad");
  } finally {
    setBusy(false);
  }
}

function showPage(page: string) {
  state.page = page;
  if (page === "dungeon" || page === "items") void refresh(true);
  for (const tab of Array.from(document.querySelectorAll<HTMLButtonElement>(".rail-tab"))) {
    tab.classList.toggle("active", tab.dataset.page === page);
  }
  for (const section of Array.from(document.querySelectorAll<HTMLElement>(".page"))) {
    section.hidden = section.id !== `${page}-page`;
  }
}

async function loadLogs() {
  try {
    const logs = await invoke<{ channel: string[]; entrance: string[]; directory: string }>("logs_refresh");
    element("channel-log").textContent =
      `${logs.directory}\n\n` + logs.channel.join("\n") + (logs.channel.length ? "" : "\n（还没有日志）");
    message(`日志尾部 ${logs.channel.length} 行。`);
  } catch (error) {
    message(describe(error), "bad");
  }
}

async function saveConfiguration() {
  if (!state.current) return;
  const config = {
    ...state.current.config,
    root: element<HTMLInputElement>("cfg-root").value.trim(),
    store: element<HTMLInputElement>("cfg-store").value.trim(),
    logs: element<HTMLInputElement>("cfg-logs").value.trim(),
    clientDir: element<HTMLInputElement>("cfg-client-dir").value.trim(),
    clientExe: element<HTMLInputElement>("cfg-client-exe").value.trim(),
    launchArguments: element<HTMLInputElement>("cfg-arguments").value.trim(),
    channelPort: Number(element<HTMLInputElement>("cfg-channel-port").value),
    entrancePort: Number(element<HTMLInputElement>("cfg-entrance-port").value),
    oneHitKill: element<HTMLInputElement>("cfg-one-hit").checked,
  };
  try {
    const reply = await invoke<{ message: string; issue: string | null }>("save_config", { config });
    const alert = element<HTMLDivElement>("config-issue");
    alert.hidden = !reply.issue;
    alert.textContent = reply.issue ?? "";
    message(reply.issue ? `${reply.message} 但还有问题：${reply.issue}` : reply.message, reply.issue ? "bad" : "ok");
    void refresh(true);
  } catch (error) {
    message(describe(error), "bad");
  }
}

function wire() {
  for (const tab of Array.from(document.querySelectorAll<HTMLButtonElement>(".rail-tab"))) {
    tab.addEventListener("click", () => showPage(tab.dataset.page ?? "service"));
  }
  element("refresh-button").addEventListener("click", () => void refresh());
  element("start-server").addEventListener("click", () => void service("start"));
  element("stop-server").addEventListener("click", () => void service("stop"));
  element("reload-logs").addEventListener("click", () => void loadLogs());
  element("launch-client").addEventListener("click", async () => {
    try {
      const reply = await invoke<{ pid: number; message: string }>("client_launch");
      message(`${reply.message}（PID ${reply.pid}）`, "ok");
      void refresh(true);
    } catch (error) {
      message(describe(error), "bad");
    }
  });
  element<HTMLInputElement>("item-id").addEventListener("input", refreshControls);
  element("item-find").addEventListener("click", () => void searchItems());
  element("kill-all").addEventListener("click", () => void runGm("dungeon-killall", gmParams("dungeon-killall")));
  element("pick-all").addEventListener("click", () => void runGm("loot-pickall", {}));
  element("autopick-toggle").addEventListener("click", () =>
    void runGm("loot-autopick", { enable: snap("autoPickup") === "1" ? 0 : 1 }),
  );
  element("quest-refresh").addEventListener("click", (event) => void refreshQuestView(event.currentTarget as HTMLElement));
  element<HTMLInputElement>("cfg-hotkey").addEventListener("keydown", (event) => {
    const captured = captureHotkey(event as KeyboardEvent);
    if (!captured) return;
    event.preventDefault();
    event.stopPropagation();
    pendingHotkey = captured;
    element<HTMLInputElement>("cfg-hotkey").value = captured.label;
    hotkeyStatus("按下“登记”生效；未包含修饰键的组合不会提示。");
  });
  element("hotkey-set").addEventListener("click", () => void applyHotkey(pendingHotkey));
  element("hotkey-clear").addEventListener("click", () => void applyHotkey(null));
  element("save-config").addEventListener("click", () => void saveConfiguration());
  element("fill-arguments").addEventListener("click", async () => {
    try {
      const reply = await invoke<{ defaultLaunchArguments: string }>("read_config");
      element<HTMLInputElement>("cfg-arguments").value = reply.defaultLaunchArguments;
      message("已按当前入口端口生成启动参数，记得保存配置。");
    } catch (error) {
      message(describe(error), "bad");
    }
  });
  for (const button of Array.from(document.querySelectorAll<HTMLButtonElement>("[data-pick]"))) {
    button.addEventListener("click", async () => {
      const target = button.dataset.pick ?? "";
      const picked = await pickDirectory({ directory: true, multiple: false, title: "选择目录" });
      if (typeof picked === "string") {
        element<HTMLInputElement>(target).value = picked;
        message(`已选择 ${picked}，记得保存配置。`);
      }
    });
  }
  for (const button of Array.from(document.querySelectorAll<HTMLButtonElement>("[data-gm]"))) {
    button.addEventListener("click", () => {
      const op = button.dataset.gm ?? "";
      void runGm(op, gmParams(op, button));
    });
  }
  element<HTMLInputElement>("item-query").addEventListener("keydown", (event) => {
    if ((event as KeyboardEvent).key === "Enter") void searchItems();
  });
  element<HTMLInputElement>("quest-selected").addEventListener("input", refreshControls);
}

async function listenRuntimeEvents() {
  const { listen } = await import("@tauri-apps/api/event");
  await listen<{ registered?: boolean; cleared?: boolean; message?: string }>("hotkey-state", (event) => {
    if (event.payload?.registered === false && event.payload?.cleared !== true) {
      hotkeyStatus("这组按键被系统或其它程序占用，请换一个。", true);
    }
  });
  await listen<{ ok: boolean; message: string; source: string }>("gm-event", (event) => {
    message(event.payload.ok ? `快捷键触发 ${event.payload.source}：${event.payload.message}` : `快捷键失败：${event.payload.message}`, event.payload.ok ? "ok" : "bad");
    void refresh(true);
  });
}

void listenRuntimeEvents();
wire();
void refresh();
void loadLogs();
